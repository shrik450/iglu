//! Running fixed programs inside guests through the Incus exec API.
//!
//! Every argv here is built from constants and parsed domain values. Nothing
//! is ever passed through a shell.

use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hyper::Method;
use iglu_domain::env::GuestUser;
use iglu_domain::id::InstanceName;
use iglu_domain::terminal::TerminalSize;
use serde::Deserialize;
use serde_json::json;
use tokio::net::UnixStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use super::client::{Incus, IncusError};
use super::guestfs::paths;
use crate::runtime::Output;

/// Output beyond this much per stream is dropped.
const MAX_OUTPUT: usize = 1024 * 1024;

/// A program run as the workspace user. hostd never runs anything in a
/// guest as root.
#[derive(Clone, Debug)]
pub struct Program<'a> {
    argv: Vec<String>,
    user: &'a GuestUser,
}

impl<'a> Program<'a> {
    pub fn new(user: &'a GuestUser, argv: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            argv: argv.into_iter().map(Into::into).collect(),
            user,
        }
    }

    /// Every exec waits for hostd to connect its streams, so even programs
    /// that only produce output can be given input.
    fn body(&self, interactive: Option<TerminalSize>) -> serde_json::Value {
        let user = self.user;
        let env = BTreeMap::from([
            ("HOME", user.home.to_string()),
            ("USER", user.name.to_string()),
            ("LOGNAME", user.name.to_string()),
            ("XDG_RUNTIME_DIR", paths::runtime_dir(user)),
            (
                iglu_domain::guest::CHANNEL_ENV,
                iglu_domain::guest::INSTANCE_CHANNEL.to_owned(),
            ),
            (
                "PATH",
                format!(
                    "/run/wrappers/bin:/etc/profiles/per-user/{}/bin:/run/current-system/sw/bin",
                    user.name
                ),
            ),
        ]);
        let mut body = json!({
            "command": self.argv,
            "environment": env,
            "user": user.uid.get(),
            "group": user.gid.get(),
            "cwd": user.home.to_string(),
            "wait-for-websocket": true,
            "record-output": false,
        });
        if let Some(size) = interactive {
            body["interactive"] = json!(true);
            body["width"] = json!(size.cols());
            body["height"] = json!(size.rows());
        } else {
            body["interactive"] = json!(false);
        }
        body
    }
}

#[derive(Deserialize)]
struct ExecResult {
    #[serde(rename = "return")]
    status: i64,
}

#[derive(Deserialize)]
struct Fds {
    fds: BTreeMap<String, String>,
}

/// An exec started in WebSocket mode, with its streams open.
struct Started {
    operation: String,
    sockets: BTreeMap<String, WebSocketStream<UnixStream>>,
}

impl Started {
    async fn new(
        incus: &Incus,
        instance: InstanceName,
        body: &serde_json::Value,
        names: &[&str],
    ) -> Result<Self, IncusError> {
        let path = format!("/1.0/instances/{instance}/exec");
        let (operation, metadata) = incus.start(Method::POST, &path, body).await?;
        let Fds { fds } =
            serde_json::from_value(metadata).map_err(|e| IncusError::Protocol(e.to_string()))?;
        let mut sockets = BTreeMap::new();
        for name in names {
            let secret = fds
                .get(*name)
                .ok_or_else(|| IncusError::Protocol(format!("exec has no {name} socket")))?;
            sockets.insert(
                (*name).to_owned(),
                incus.websocket(&operation, secret).await?,
            );
        }
        Ok(Self { operation, sockets })
    }

    fn take(&mut self, name: &str) -> WebSocketStream<UnixStream> {
        self.sockets
            .remove(name)
            .expect("Started::new opened every socket it was asked for")
    }
}

/// Runs a program to completion, with `stdin` as its input, and returns its
/// exit status and output.
pub async fn capture(
    incus: &Incus,
    instance: InstanceName,
    program: &Program<'_>,
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<Output, IncusError> {
    tokio::time::timeout(timeout, async {
        let mut exec = Started::new(
            incus,
            instance,
            &program.body(None),
            &["control", "0", "1", "2"],
        )
        .await?;
        // Unused, but held open like the other streams until the exec is over.
        let _control = exec.take("control");
        let (input, stdout, stderr) = tokio::join!(
            feed(exec.take("0"), stdin),
            drain(exec.take("1")),
            drain(exec.take("2")),
        );
        if let Err(error) = input {
            tracing::debug!(%error, "the program stopped reading its input");
        }
        let result: ExecResult =
            serde_json::from_value(incus.wait(&exec.operation, timeout).await?)
                .map_err(|e| IncusError::Protocol(e.to_string()))?;
        Ok(Output {
            success: result.status == 0,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    })
    .await
    .map_err(|_| IncusError::Operation(format!("timed out after {}s", timeout.as_secs())))?
}

/// Writes the input, then closes the socket, which Incus passes on as the
/// end of the program's input.
async fn feed(
    mut socket: WebSocketStream<UnixStream>,
    input: Option<&[u8]>,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    if let Some(bytes) = input {
        socket.send(Message::binary(bytes.to_vec())).await?;
    }
    socket.close(None).await
}

/// Reads an output stream to its end: a close, or the empty text message
/// Incus sends as a barrier.
async fn drain(mut socket: WebSocketStream<UnixStream>) -> Vec<u8> {
    let mut output = Vec::new();
    while let Some(Ok(message)) = socket.next().await {
        let bytes = match message {
            Message::Binary(bytes) => bytes,
            Message::Text(text) if text.is_empty() => break,
            Message::Text(text) => text.as_bytes().to_vec().into(),
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
        };
        let room = MAX_OUTPUT.saturating_sub(output.len());
        output.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }
    output
}

/// An interactive program attached to a PTY: `data` carries terminal bytes
/// both ways and `control` carries resizes.
pub struct Interactive {
    pub data: WebSocketStream<UnixStream>,
    pub control: WebSocketStream<UnixStream>,
}

pub async fn interactive(
    incus: &Incus,
    instance: InstanceName,
    program: &Program<'_>,
    size: TerminalSize,
) -> Result<Interactive, IncusError> {
    let mut exec = Started::new(
        incus,
        instance,
        &program.body(Some(size)),
        &["control", "0"],
    )
    .await?;
    Ok(Interactive {
        control: exec.take("control"),
        data: exec.take("0"),
    })
}

/// The control message that resizes an interactive program's PTY.
pub fn resize_message(size: TerminalSize) -> String {
    json!({
        "command": "window-resize",
        "args": { "width": size.cols().to_string(), "height": size.rows().to_string() },
    })
    .to_string()
}
