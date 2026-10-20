//! Showing a workspace's browser in the console, and letting the person use it.
//!
//! The browser is Chromium on a display of its own in the workspace, a
//! headless sway that wayvnc serves over VNC on the workspace's loopback
//! (see `iglu-guest browser`). iglud reaches it through a port tunnel, like
//! the preview gateway does, and relays VNC's bytes between it and the
//! console as they are: the browser draws everything, its tabs, menus,
//! dialogs and pickers included.
//!
//! Beside that, iglud draws the display at the console's pixel ratio, and
//! speaks Chrome's debugging protocol (CDP) on the browser's own port for
//! what VNC can't carry: the downloads the person saves from here, and
//! addresses the console asks it to open. Agents in the workspace drive the
//! same browser through that port, so the person watches what they do.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as Console, WebSocket};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Empty, Limited};
use hyper_util::rt::TokioIo;
use iglu_api::Closed;
use iglu_api::browser::{BrowserEvent, BrowserInput};
use iglu_domain::column::{BROWSER_DEBUG_PORT, BROWSER_DISPLAY_PORT};
use iglu_domain::display::{DisplayReport, DisplayScale};
use iglu_domain::download::DownloadName;
use iglu_domain::id::WorkspaceId;
use iglu_domain::port::GuestPort;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as Devtools;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::app::{App, Caller};
use crate::hosts::HostClient;
use crate::model::WorkspaceRecord;
use crate::terminal::{LEASE, Use, refusal, still_allowed};

/// How long opening waits for a browser that's starting.
const STARTING: Duration = Duration::from_secs(20);

/// How long opening waits for a frozen workspace to thaw.
const THAW: Duration = Duration::from_secs(60);

/// How long one send, to either side, may take before the relay gives up on
/// it: a peer that stops reading would otherwise hold off the lease.
const SEND: Duration = Duration::from_secs(10);

/// The most read from the display at once, as one frame to the console.
const CHUNK: usize = 64 * 1024;

/// The largest message taken from the browser's debugging port: download
/// events are small, and nothing else is listened for.
const MAX_MESSAGE: usize = 1024 * 1024;

/// The longest path a download is told with.
const MAX_PATH: usize = 4096;

/// The most of the browser's version report read, and how long asking takes.
const VERSION_BYTES: usize = 64 * 1024;
const VERSION_TIME: Duration = Duration::from_secs(5);

/// The first command iglud sends the browser: to hear of its downloads.
/// They go where the browser's own settings say, the user's Downloads; only
/// these events say where each landed. While the console shows it, this
/// replaces where an agent asked the browser to put them.
#[must_use]
pub fn hearing_downloads() -> String {
    json!({
        "id": 1,
        "method": "Browser.setDownloadBehavior",
        "params": { "behavior": "default", "eventsEnabled": true },
    })
    .to_string()
}

/// The command that opens `typed` in a new tab, in front.
#[must_use]
pub fn opening(id: u64, typed: &str) -> String {
    json!({
        "id": id,
        "method": "Target.createTarget",
        "params": { "url": address(typed) },
    })
    .to_string()
}

/// What the browser said that the console hears of: a download it finished,
/// wherever it was started.
#[must_use]
pub fn heard(message: &str) -> Option<BrowserEvent> {
    #[derive(Deserialize)]
    struct Message {
        method: Option<String>,
        #[serde(default)]
        params: Value,
    }
    let message: Message = serde_json::from_str(message).ok()?;
    if message.method.as_deref() != Some("Browser.downloadProgress")
        || message.params["state"] != "completed"
    {
        return None;
    }
    let path = message.params["filePath"].as_str()?;
    Some(BrowserEvent::Downloaded {
        name: DownloadName::of_download(path),
        path: path.chars().take(MAX_PATH).collect(),
    })
}

/// Where an address typed into the console goes: as typed when it has a
/// scheme, over plain HTTP when it's the workspace's own (`localhost:3000`),
/// and over HTTPS otherwise.
fn address(typed: &str) -> String {
    let typed = typed.trim();
    let has_scheme = typed.split_once(':').is_some_and(|(scheme, rest)| {
        let mut chars = scheme.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            // `localhost:3000` is a host and a port, not a scheme.
            && !rest.chars().next().is_some_and(|c| c.is_ascii_digit())
    });
    if has_scheme || typed.is_empty() {
        return typed.to_owned();
    }
    // The host as a browser reads it, so plain HTTP never reaches another:
    // a backslash ends it as a slash does, and user info and a port are
    // either side of it.
    let authority = typed
        .split(['/', '\\', '?', '#'])
        .next()
        .unwrap_or_default();
    let host_and_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host_and_port.split(':').next().unwrap_or_default();
    let local = host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<std::net::Ipv4Addr>().is_ok();
    if local {
        format!("http://{typed}")
    } else {
        format!("https://{typed}")
    }
}

/// Whether a frame of RFB from the console is the person using the
/// browser: a key, the pointer, or a paste. The console sends each message
/// in a frame of its own, and asks for the display's updates the whole time
/// it's shown, which isn't use.
#[must_use]
pub fn is_input(frame: &[u8]) -> bool {
    const KEY: u8 = 4;
    const POINTER: u8 = 5;
    const CUT_TEXT: u8 = 6;
    matches!(frame.first(), Some(&(KEY | POINTER | CUT_TEXT)))
}

fn event(event: &BrowserEvent) -> Console {
    Console::text(serde_json::to_string(event).unwrap_or_default())
}

/// Draws the browser's display at `scale`, waiting a little for one that's
/// starting.
async fn display(
    host: &HostClient,
    workspace: WorkspaceId,
    scale: DisplayScale,
) -> Result<DisplayReport, Closed> {
    let deadline = tokio::time::Instant::now() + STARTING;
    loop {
        match host.display(workspace, scale).await {
            Ok(report) => return Ok(report),
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(error) => {
                tracing::debug!(%workspace, %error, "the browser's display didn't answer");
                return Err(Closed::BrowserNotRunning);
            }
        }
    }
}

/// What showing the browser takes: its display, drawn at the console's
/// scale, the VNC connection to it, and its debugging port, without which
/// it still shows; only downloads and opening addresses wait for it.
async fn open(
    app: &App,
    ws: &WorkspaceRecord,
    host: &HostClient,
    scale: DisplayScale,
) -> Result<(DisplayReport, reqwest::Upgraded, Option<Browser>), Closed> {
    let started = std::time::Instant::now();
    if !crate::idle::thaw(app, ws, THAW).await {
        return Err(Closed::NotRunning);
    }
    let report = display(host, ws.id, scale).await?;
    let port = GuestPort::try_from(BROWSER_DISPLAY_PORT).expect("the display's port is a port");
    let screen = host.tunnel(ws.id, port).await.map_err(|error| {
        tracing::info!(workspace = %ws.id, %error, "couldn't reach the browser's display");
        Closed::BrowserUnreachable
    })?;
    let devtools = match connect(host, ws.id).await {
        Ok(mut browser) => browser
            .send(Devtools::text(hearing_downloads()))
            .await
            .ok()
            .map(|()| browser),
        Err(error) => {
            tracing::info!(workspace = %ws.id, %error, "couldn't reach the browser's debugging port");
            None
        }
    };
    tracing::info!(
        workspace = %ws.id,
        hardware = report.hardware,
        devtools = devtools.is_some(),
        took = ?started.elapsed(),
        "showing the browser"
    );
    Ok((report, screen, devtools))
}

/// Opens the browser in front of `caller` and relays it until either side
/// leaves or their access ends.
pub async fn relay(
    app: Arc<App>,
    caller: Caller,
    ws: WorkspaceRecord,
    host: Arc<HostClient>,
    scale: DisplayScale,
    console: WebSocket,
) {
    let (mut to_console, mut from_console) = console.split();
    let (report, screen, mut devtools) = match open(&app, &ws, &host, scale).await {
        Ok(opened) => opened,
        Err(error) => {
            let _ = to_console.send(refusal(error)).await;
            return;
        }
    };
    let (mut from_screen, mut to_screen) = tokio::io::split(screen);
    let display = BrowserEvent::Display {
        hardware: report.hardware,
    };
    if to_console.send(event(&display)).await.is_err() {
        return;
    }
    let mut lease = tokio::time::interval(LEASE);
    lease.tick().await;
    let mut used = Use::Idle;
    let mut revoked = false;
    let mut next_id = 1_u64;
    let mut buffer = vec![0_u8; CHUNK];

    loop {
        let from_browser = async {
            match devtools.as_mut() {
                Some(browser) => browser.next().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            message = from_console.next() => match message {
                Some(Ok(Console::Binary(bytes))) => {
                    used = if is_input(&bytes) { Use::Active } else { used };
                    let sent = tokio::time::timeout(SEND, to_screen.write_all(&bytes)).await;
                    if !matches!(sent, Ok(Ok(()))) {
                        break;
                    }
                }
                Some(Ok(Console::Text(text))) => match serde_json::from_str::<BrowserInput>(text.as_str()) {
                    Ok(BrowserInput::Scale { scale }) => {
                        if let Err(error) = host.display(ws.id, scale).await {
                            tracing::debug!(workspace = %ws.id, %error, "the browser's display kept its scale");
                        }
                    }
                    Ok(BrowserInput::Open { address }) => {
                        used = Use::Active;
                        if let Some(browser) = devtools.as_mut() {
                            next_id += 1;
                            let _ = browser.send(Devtools::text(opening(next_id, &address))).await;
                        }
                    }
                    Err(error) => tracing::debug!(%error, "ignoring browser input iglu doesn't know"),
                },
                Some(Ok(Console::Ping(_) | Console::Pong(_))) => {}
                Some(Ok(Console::Close(_)) | Err(_)) | None => break,
            },
            read = from_screen.read(&mut buffer) => match read {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let frame = Console::Binary(Bytes::copy_from_slice(&buffer[..n]));
                    if !matches!(tokio::time::timeout(SEND, to_console.send(frame)).await, Ok(Ok(()))) {
                        break;
                    }
                }
            },
            message = from_browser => match message {
                Some(Ok(Devtools::Text(text))) => {
                    if let Some(told) = heard(text.as_str())
                        && to_console.send(event(&told)).await.is_err()
                    {
                        break;
                    }
                }
                Some(Ok(_)) => {}
                // The browser was closed and starts again: downloads it
                // finishes then go unheard until the console reconnects.
                Some(Err(_)) | None => devtools = None,
            },
            _ = lease.tick() => {
                // A browser in front of someone keeps its workspace awake.
                app.usage.used(ws.id);
                if !still_allowed(&app, &caller, &ws, used).await {
                    tracing::info!(workspace = %ws.id, "closing a browser whose authorization ended");
                    revoked = true;
                    break;
                }
                used = Use::Idle;
            }
        }
    }
    let _ = to_screen.shutdown().await;
    if let Some(mut browser) = devtools {
        let _ = browser.close(None).await;
    }
    if revoked {
        let _ = to_console.send(refusal(Closed::AccessEnded)).await;
    } else {
        let _ = to_console.close().await;
    }
}

type Browser = WebSocketStream<reqwest::Upgraded>;

/// Connects to the browser's debugging port, waiting a little for one
/// that's starting.
async fn connect(host: &HostClient, workspace: WorkspaceId) -> Result<Browser, String> {
    let port = GuestPort::try_from(BROWSER_DEBUG_PORT).map_err(|e| e.to_string())?;
    let deadline = tokio::time::Instant::now() + STARTING;
    let path = loop {
        let asked = tokio::time::timeout(VERSION_TIME, endpoint(host, workspace, port)).await;
        match asked.unwrap_or_else(|_| Err(anyhow::anyhow!("no answer"))) {
            Ok(path) => break path,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    let stream = host
        .tunnel(workspace, port)
        .await
        .map_err(|e| e.to_string())?;
    // Chromium takes DevTools connections only for a loopback host.
    let request = format!("ws://127.0.0.1:{BROWSER_DEBUG_PORT}{path}")
        .into_client_request()
        .map_err(|e| e.to_string())?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let (browser, _) = tokio_tungstenite::client_async_with_config(request, stream, Some(config))
        .await
        .map_err(|e| format!("the browser refused the connection: {e}"))?;
    Ok(browser)
}

/// The path of the browser's own CDP endpoint, which it makes up as it
/// starts.
async fn endpoint(
    host: &HostClient,
    workspace: WorkspaceId,
    port: GuestPort,
) -> anyhow::Result<String> {
    let stream = host.tunnel(workspace, port).await?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(connection);
    let request = hyper::Request::get("/json/version")
        .header(
            hyper::header::HOST,
            format!("127.0.0.1:{BROWSER_DEBUG_PORT}"),
        )
        .body(Empty::<Bytes>::new())?;
    let response = sender.send_request(request).await?;
    anyhow::ensure!(response.status().is_success(), "{}", response.status());
    // What's on the port is the workspace's: a version report is small.
    let body = Limited::new(response.into_body(), VERSION_BYTES)
        .collect()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .to_bytes();
    endpoint_path(&body)
}

fn endpoint_path(version: &[u8]) -> anyhow::Result<String> {
    #[derive(Deserialize)]
    struct Version {
        #[serde(rename = "webSocketDebuggerUrl")]
        url: String,
    }
    let version: Version = serde_json::from_slice(version)?;
    let url = url::Url::parse(&version.url)?;
    Ok(url.path().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_pointers_and_pastes_are_use_and_asking_for_updates_isnt() {
        assert!(is_input(&[4, 1, 0, 0, 0, 0, 0, 0x61]));
        assert!(is_input(&[5, 0, 0, 10, 0, 20]));
        assert!(is_input(&[6, 0, 0, 0, 0, 0, 0, 0]));
        // FramebufferUpdateRequest and SetEncodings.
        assert!(!is_input(&[3, 1, 0, 0, 0, 0, 4, 0, 3, 0]));
        assert!(!is_input(&[2, 0, 0, 1, 0, 0, 0, 16]));
        assert!(!is_input(&[]));
    }

    #[test]
    fn finished_downloads_are_told_by_name_when_theyre_in_downloads() {
        let first: Value = serde_json::from_str(&hearing_downloads()).expect("JSON");
        assert_eq!(first["method"], "Browser.setDownloadBehavior");
        assert_eq!(first["params"]["eventsEnabled"], true);
        let progress = |state: &str, path: &str| {
            json!({ "method": "Browser.downloadProgress", "params": { "guid": "g", "state": state, "filePath": path } })
                .to_string()
        };
        assert_eq!(
            heard(&progress("inProgress", "/home/dev/Downloads/a.txt")),
            None
        );
        assert_eq!(
            heard(&progress("completed", "/home/dev/Downloads/note (1).txt")),
            Some(BrowserEvent::Downloaded {
                name: Some("note (1).txt".parse().expect("a name")),
                path: "/home/dev/Downloads/note (1).txt".into(),
            })
        );
        // Where an agent had them go, it's only said where.
        assert!(matches!(
            heard(&progress("completed", "/tmp/playwright/ab12")),
            Some(BrowserEvent::Downloaded { name: None, .. })
        ));
        assert_eq!(heard(r#"{"id":1,"result":{}}"#), None);
        assert_eq!(heard("not json"), None);
    }

    #[test]
    fn an_address_opens_in_a_tab_of_its_own_as_a_browser_would_take_it() {
        let open: Value =
            serde_json::from_str(&opening(7, "localhost:8085/callback")).expect("JSON");
        assert_eq!(open["id"], 7);
        assert_eq!(open["method"], "Target.createTarget");
        assert_eq!(open["params"]["url"], "http://localhost:8085/callback");
    }

    #[test]
    fn addresses_go_where_a_browser_would_take_them() {
        assert_eq!(address("localhost:3000"), "http://localhost:3000");
        assert_eq!(address("app.localhost/x"), "http://app.localhost/x");
        assert_eq!(address("127.0.0.1:8080/a?b"), "http://127.0.0.1:8080/a?b");
        assert_eq!(address("example.com"), "https://example.com");
        // What only looks local goes over HTTPS.
        assert_eq!(address("app\\.localhost"), "https://app\\.localhost");
        assert_eq!(
            address("localhost:3000@evil.example"),
            "https://localhost:3000@evil.example"
        );
        assert_eq!(address(" https://example.com/a "), "https://example.com/a");
        assert_eq!(address("about:blank"), "about:blank");
        assert_eq!(
            address("file:///home/dev/index.html"),
            "file:///home/dev/index.html"
        );
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        fn typed() -> impl Strategy<Value = String> {
            let piece = prop::sample::select(
                &[
                    "localhost",
                    ".localhost",
                    "app",
                    "127.0.0.1",
                    "10.0.0.2",
                    "127.1",
                    ":",
                    "@",
                    "/",
                    "\\",
                    "?",
                    "#",
                    "evil.example",
                    "3000",
                    "[",
                    "]",
                    "%40",
                    " ",
                    "\t",
                    "http:",
                    "x",
                ][..],
            );
            prop_oneof![
                prop::collection::vec(piece, 0..8).prop_map(|pieces| pieces.concat()),
                any::<String>(),
            ]
        }

        proptest! {
            /// Only the workspace's own addresses go over plain HTTP:
            /// whatever is typed, what's given `http://` here reaches
            /// localhost or a numbered address, never a named host.
            #[test]
            fn plain_http_only_reaches_the_workspace_itself(typed in typed()) {
                let opened = address(&typed);
                if opened != typed.trim()
                    && opened.starts_with("http://")
                    && let Ok(url) = url::Url::parse(&opened)
                {
                    let local = match url.host() {
                        Some(url::Host::Domain(host)) => host == "localhost" || host.ends_with(".localhost"),
                        Some(url::Host::Ipv4(_)) => true,
                        Some(url::Host::Ipv6(_)) | None => false,
                    };
                    prop_assert!(local, "{:?} opens {} over plain HTTP", typed, url);
                }
            }

            #[test]
            fn addresses_without_a_scheme_get_one(typed in "[a-z0-9.]{1,12}(:[0-9]{1,5})?(/[a-z]{0,6})?") {
                let opened = address(&typed);
                prop_assert!(opened == format!("http://{typed}") || opened == format!("https://{typed}"), "{}", opened);
            }
        }
    }

    #[test]
    fn the_endpoint_is_the_path_chromium_reports() {
        let version = br#"{"Browser":"Chrome/140","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/browser/abc-123"}"#;
        assert_eq!(
            endpoint_path(version).expect("a path"),
            "/devtools/browser/abc-123"
        );
    }
}
