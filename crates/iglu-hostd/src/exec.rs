//! Running fixed programs inside guests through the Incus exec API.
//!
//! Every argv here is built from constants and parsed domain values. Nothing
//! is ever passed through a shell.

use std::collections::BTreeMap;
use std::time::Duration;

use hyper::Method;
use iglu_domain::env::GuestUser;
use iglu_domain::id::InstanceName;
use iglu_domain::terminal::TerminalSize;
use serde::Deserialize;
use serde_json::json;
use tokio::net::UnixStream;
use tokio_tungstenite::WebSocketStream;

use crate::incus::{Incus, IncusError};

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

    fn body(&self, interactive: Option<TerminalSize>) -> serde_json::Value {
        let user = self.user;
        let env = BTreeMap::from([
            ("HOME", user.home.to_string()),
            ("USER", user.name.to_string()),
            ("LOGNAME", user.name.to_string()),
            ("XDG_RUNTIME_DIR", format!("/run/user/{}", user.uid.get())),
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
        });
        if let Some(size) = interactive {
            body["interactive"] = json!(true);
            body["wait-for-websocket"] = json!(true);
            body["width"] = json!(size.cols());
            body["height"] = json!(size.rows());
        } else {
            body["interactive"] = json!(false);
            body["wait-for-websocket"] = json!(false);
            body["record-output"] = json!(true);
        }
        body
    }
}

#[derive(Debug)]
pub struct Output {
    pub status: i64,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub const fn success(&self) -> bool {
        self.status == 0
    }

    /// The end of stderr, for error messages.
    pub fn stderr_tail(&self) -> String {
        tail(&self.stderr, 2000)
    }
}

pub fn tail(text: &str, max: usize) -> String {
    let trimmed = text.trim_end();
    if max == 0 {
        return String::new();
    }
    let start = trimmed
        .char_indices()
        .rev()
        .nth(max - 1)
        .map_or(0, |(i, _)| i);
    trimmed[start..].to_owned()
}

#[derive(Deserialize)]
struct ExecResult {
    #[serde(rename = "return")]
    status: i64,
    #[serde(default)]
    output: BTreeMap<String, String>,
}

/// Runs a program to completion and returns its exit status and output.
pub async fn capture(
    incus: &Incus,
    instance: InstanceName,
    program: &Program<'_>,
    timeout: Duration,
) -> Result<Output, IncusError> {
    let path = format!("/1.0/instances/{instance}/exec");
    let metadata = incus
        .run(Method::POST, &path, Some(&program.body(None)), timeout)
        .await?;
    let result: ExecResult =
        serde_json::from_value(metadata).map_err(|e| IncusError::Protocol(e.to_string()))?;
    let read_log = async |fd: &str| -> Result<String, IncusError> {
        match result.output.get(fd) {
            Some(log) => {
                let bytes = incus.get_raw(log).await?;
                if let Err(error) = incus.delete(log).await {
                    tracing::debug!(%error, "couldn't remove an exec log");
                }
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            }
            None => Ok(String::new()),
        }
    };
    let stdout = read_log("1").await?;
    let stderr = read_log("2").await?;
    Ok(Output {
        status: result.status,
        stdout,
        stderr,
    })
}

/// An interactive program attached to a PTY: `data` carries terminal bytes
/// both ways and `control` carries resizes.
pub struct Interactive {
    pub data: WebSocketStream<UnixStream>,
    pub control: WebSocketStream<UnixStream>,
}

#[derive(Deserialize)]
struct Fds {
    fds: BTreeMap<String, String>,
}

pub async fn interactive(
    incus: &Incus,
    instance: InstanceName,
    program: &Program<'_>,
    size: TerminalSize,
) -> Result<Interactive, IncusError> {
    let path = format!("/1.0/instances/{instance}/exec");
    let (operation, metadata) = incus
        .start(Method::POST, &path, &program.body(Some(size)))
        .await?;
    let Fds { fds } =
        serde_json::from_value(metadata).map_err(|e| IncusError::Protocol(e.to_string()))?;
    let secret = |fd: &str| {
        fds.get(fd)
            .cloned()
            .ok_or_else(|| IncusError::Protocol(format!("exec has no {fd} socket")))
    };
    let control = incus.websocket(&operation, &secret("control")?).await?;
    let data = incus.websocket(&operation, &secret("0")?).await?;
    Ok(Interactive { data, control })
}

/// The control message that resizes an interactive program's PTY.
pub fn resize_message(size: TerminalSize) -> String {
    json!({
        "command": "window-resize",
        "args": { "width": size.cols().to_string(), "height": size.rows().to_string() },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::tail;

    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("ab", 3), "ab");
        assert_eq!(tail("héllo\n", 4), "éllo");
    }
}
