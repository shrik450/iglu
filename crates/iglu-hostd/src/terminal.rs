//! Relaying a client's WebSocket to a terminal session. Closing either side
//! detaches; the session and its processes keep running.

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use iglu_proto::{CommandError, TerminalControl};

use crate::runtime::{Terminal, TerminalInput};

pub async fn relay(client: WebSocket, terminal: Result<Terminal, CommandError>) {
    let (mut to_client, mut from_client) = client.split();
    let Terminal { input, mut output } = match terminal {
        Ok(terminal) => terminal,
        Err(error) => {
            tracing::warn!(%error, "attach failed");
            let _ = to_client
                .send(Message::Close(Some(close_frame(&error.message))))
                .await;
            return;
        }
    };
    let upstream = async {
        while let Some(Ok(message)) = from_client.next().await {
            let forwarded = match message {
                Message::Binary(bytes) => TerminalInput::Bytes(bytes),
                Message::Text(text) => match serde_json::from_str::<TerminalControl>(&text) {
                    Ok(TerminalControl::Resize(size)) => TerminalInput::Resize(size),
                    Err(error) => {
                        tracing::debug!(%error, "ignoring a malformed control frame");
                        continue;
                    }
                },
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) => continue,
            };
            if input.send(forwarded).await.is_err() {
                break;
            }
        }
    };
    let downstream = async {
        while let Some(bytes) = output.recv().await {
            if to_client.send(Message::Binary(bytes)).await.is_err() {
                return;
            }
        }
        let _ = to_client
            .send(Message::Close(Some(close_frame("session ended"))))
            .await;
    };
    tokio::select! {
        () = upstream => {}
        () = downstream => {}
    }
}

fn close_frame(reason: &str) -> CloseFrame {
    let reason: String = reason.chars().take(100).collect();
    CloseFrame {
        code: 1011,
        reason: reason.into(),
    }
}
