//! `iglu attach`: one of a workspace's columns in this terminal, the way the
//! console shows it. The session carries on after detaching.

use std::io::Write;
use std::os::fd::AsFd;

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt};
use iglu_api::SetDesiredState;
use iglu_domain::column::ColumnState;
use iglu_domain::lifecycle::{DesiredState, Phase};
use iglu_domain::terminal::{SessionName, TerminalControl, TerminalSize};
use rustix::termios::{self, OptionalActions, Termios};
use tokio::io::AsyncReadExt;
use tokio::signal::unix::{SignalKind, signal};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// Ctrl-], as in telnet: unlikely to matter to anything running inside.
const DETACH: u8 = 0x1d;

/// Puts the terminal back as it was, however attaching ends.
struct Raw {
    saved: Termios,
}

impl Raw {
    fn enter() -> anyhow::Result<Self> {
        let stdin = std::io::stdin();
        let saved = termios::tcgetattr(stdin.as_fd()).context("stdin isn't a terminal")?;
        let mut raw = saved.clone();
        raw.make_raw();
        termios::tcsetattr(stdin.as_fd(), OptionalActions::Now, &raw)?;
        Ok(Self { saved })
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = termios::tcsetattr(std::io::stdin().as_fd(), OptionalActions::Now, &self.saved);
    }
}

fn size() -> TerminalSize {
    termios::tcgetwinsize(std::io::stdout().as_fd())
        .ok()
        .and_then(|size| TerminalSize::new(size.ws_col, size.ws_row).ok())
        .unwrap_or(TerminalSize::DEFAULT)
}

fn resize(size: TerminalSize) -> Message {
    Message::text(serde_json::to_string(&TerminalControl::Resize(size)).unwrap_or_default())
}

use crate::client::Client;

/// Attaches to a workspace's column, or its first open one, then exits:
/// reading stdin holds a blocking thread the runtime would otherwise wait on.
///
/// # Errors
///
/// When the workspace or column can't be found or isn't open.
pub async fn run(
    client: &Client,
    workspace: &str,
    column: Option<SessionName>,
) -> anyhow::Result<()> {
    let mut ws = client.resolve(workspace).await?;
    // Opening a frozen workspace thaws it, as a preview request does.
    if ws.phase == Phase::Frozen {
        eprintln!("thawing {}…", ws.name);
        client
            .set_desired_state(
                ws.id,
                &SetDesiredState {
                    state: DesiredState::Running,
                    expected_revision: ws.revision,
                },
            )
            .await?;
        ws = crate::wait_for(client, ws.id, DesiredState::Running)
            .await?
            .context("the workspace was deleted")?;
    }
    let columns = client.columns(ws.id).await?;
    let chosen = match column {
        Some(name) => match columns.iter().find(|c| c.name == name).map(|c| c.state) {
            Some(ColumnState::Open | ColumnState::Adopted) => name,
            Some(ColumnState::Ended) => bail!("{name} has ended; restart it from the console"),
            None => bail!("{} has no column called {name}", ws.name),
        },
        None => columns
            .into_iter()
            .find(|c| matches!(c.state, ColumnState::Open | ColumnState::Adopted))
            .map(|c| c.name)
            .with_context(|| format!("{} has no open columns", ws.name))?,
    };
    let (url, token) = client.attach_target(ws.id, &chosen)?;
    let attached = Box::pin(relay(url, token)).await;
    match &attached {
        Ok(()) => eprintln!("\r\ndetached from {} {chosen}", ws.name),
        Err(error) => eprintln!("\r\n{} {chosen}: {error}", ws.name),
    }
    std::process::exit(i32::from(attached.is_err()));
}

/// Relays this terminal to a column until it ends or the person detaches.
///
/// # Errors
///
/// When stdin isn't a terminal or the connection can't be made.
async fn relay(mut url: url::Url, token: &str) -> anyhow::Result<()> {
    let start = size();
    url.query_pairs_mut()
        .append_pair("cols", &start.cols().to_string())
        .append_pair("rows", &start.rows().to_string());
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|()| anyhow::anyhow!("can't attach over {}", url.scheme()))?;
    let mut request = url.as_str().into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse()?);
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .context("couldn't attach")?;
    let (mut to_column, mut from_column) = socket.split();

    let raw = Raw::enter()?;
    let mut stdin = tokio::io::stdin();
    let mut stdout = std::io::stdout();
    let mut resized = signal(SignalKind::window_change())?;
    let mut buffer = [0u8; 4096];
    let ended = loop {
        tokio::select! {
            read = stdin.read(&mut buffer) => {
                let input = match read {
                    Ok(0) | Err(_) => break false,
                    Ok(n) => &buffer[..n],
                };
                if input.contains(&DETACH) {
                    break false;
                }
                if to_column.send(Message::binary(input.to_vec())).await.is_err() {
                    break true;
                }
            }
            message = from_column.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    stdout.write_all(&bytes)?;
                    stdout.flush()?;
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => break true,
                Some(Ok(_)) => {}
            },
            _ = resized.recv() => {
                if to_column.send(resize(size())).await.is_err() {
                    break true;
                }
            }
        }
    };
    let _ = to_column.close().await;
    drop(raw);
    if ended {
        bail!("the column's session ended or the connection dropped");
    }
    Ok(())
}
