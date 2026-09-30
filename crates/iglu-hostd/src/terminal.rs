//! Terminal sessions: listing, closing and attaching to zmx sessions through
//! `iglu-guest`, which runs inside the guest as the workspace user.

use axum::extract::ws::{Message as ClientMessage, WebSocket};
use futures_util::{SinkExt, StreamExt};
use iglu_domain::id::WorkspaceId;
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_proto::{CommandError, ErrorCode, TerminalControl, TerminalInfo};
use tokio_tungstenite::tungstenite::Message as IncusMessage;

use crate::exec::{self, Program};
use crate::guestfs::paths;
use crate::observe::Owned;
use crate::server::App;

async fn owned_running(app: &App, workspace: WorkspaceId) -> Result<Owned, CommandError> {
    let name = workspace.instance_name();
    let instance = app
        .incus
        .instance(name)
        .await
        .map_err(|e| CommandError::new(ErrorCode::Runtime, e.to_string()))?
        .ok_or_else(|| CommandError::new(ErrorCode::NotFound, "no such workspace instance"))?;
    let owned = Owned::parse(instance)
        .map_err(|e| CommandError::new(ErrorCode::Conflict, e.to_string()))?;
    if owned.pid().is_none() {
        return Err(CommandError::new(
            ErrorCode::InvalidState,
            "the workspace isn't running",
        ));
    }
    Ok(owned)
}

pub async fn list(app: &App, workspace: WorkspaceId) -> Result<Vec<TerminalInfo>, CommandError> {
    let owned = owned_running(app, workspace).await?;
    let program = Program::new(&owned.user, [paths::GUEST_TOOL, "sessions"]);
    let output = exec::capture(
        &app.incus,
        owned.name,
        &program,
        app.config.timeouts.operation(),
    )
    .await
    .map_err(|e| CommandError::new(ErrorCode::Runtime, e.to_string()))?;
    if !output.success() {
        return Err(CommandError::new(
            ErrorCode::GuestFailed,
            output.stderr_tail(),
        ));
    }
    serde_json::from_str(&output.stdout).map_err(|e| {
        CommandError::new(
            ErrorCode::GuestFailed,
            format!("unreadable session list: {e}"),
        )
    })
}

pub async fn close(
    app: &App,
    workspace: WorkspaceId,
    session: &SessionName,
) -> Result<(), CommandError> {
    let owned = owned_running(app, workspace).await?;
    let program = Program::new(&owned.user, [paths::GUEST_TOOL, "close", session.as_str()]);
    let output = exec::capture(
        &app.incus,
        owned.name,
        &program,
        app.config.timeouts.operation(),
    )
    .await
    .map_err(|e| CommandError::new(ErrorCode::Runtime, e.to_string()))?;
    if output.success() {
        Ok(())
    } else {
        Err(CommandError::new(
            ErrorCode::GuestFailed,
            output.stderr_tail(),
        ))
    }
}

/// Relays one client connection to a zmx session. Closing either side
/// detaches; the session and its processes keep running.
pub async fn attach(
    app: &App,
    workspace: WorkspaceId,
    session: SessionName,
    size: TerminalSize,
    client: WebSocket,
) {
    let (mut to_client, mut from_client) = client.split();
    let owned = match owned_running(app, workspace).await {
        Ok(owned) => owned,
        Err(error) => {
            let _ = to_client
                .send(ClientMessage::Close(Some(close_frame(&error.message))))
                .await;
            return;
        }
    };
    let program = Program::new(&owned.user, [paths::GUEST_TOOL, "attach", session.as_str()]);
    let exec = match exec::interactive(&app.incus, owned.name, &program, size).await {
        Ok(exec) => exec,
        Err(error) => {
            tracing::warn!(%workspace, %error, "attach failed");
            let _ = to_client
                .send(ClientMessage::Close(Some(close_frame("attach failed"))))
                .await;
            return;
        }
    };
    let (mut to_guest, mut from_guest) = exec.data.split();
    let mut control = exec.control;

    let upstream = async {
        while let Some(Ok(message)) = from_client.next().await {
            match message {
                ClientMessage::Binary(bytes) => {
                    if to_guest.send(IncusMessage::Binary(bytes)).await.is_err() {
                        break;
                    }
                }
                ClientMessage::Text(text) => {
                    match serde_json::from_str::<TerminalControl>(text.as_str()) {
                        Ok(TerminalControl::Resize(size)) => {
                            let resize = IncusMessage::text(exec::resize_message(size));
                            if control.send(resize).await.is_err() {
                                break;
                            }
                        }
                        Err(error) => tracing::debug!(%error, "ignoring a malformed control frame"),
                    }
                }
                ClientMessage::Close(_) => break,
                ClientMessage::Ping(_) | ClientMessage::Pong(_) => {}
            }
        }
        let _ = to_guest.close().await;
        let _ = control.close(None).await;
    };
    let downstream = async {
        while let Some(Ok(message)) = from_guest.next().await {
            let forwarded = match message {
                IncusMessage::Binary(bytes) => ClientMessage::Binary(bytes),
                IncusMessage::Text(text) => {
                    ClientMessage::Binary(text.as_str().as_bytes().to_vec().into())
                }
                IncusMessage::Close(_) => break,
                IncusMessage::Ping(_) | IncusMessage::Pong(_) | IncusMessage::Frame(_) => continue,
            };
            if to_client.send(forwarded).await.is_err() {
                break;
            }
        }
        let _ = to_client
            .send(ClientMessage::Close(Some(close_frame("session ended"))))
            .await;
    };
    tokio::select! {
        () = upstream => {}
        () = downstream => {}
    }
}

fn close_frame(reason: &str) -> axum::extract::ws::CloseFrame {
    let reason: String = reason.chars().take(100).collect();
    axum::extract::ws::CloseFrame {
        code: 1011,
        reason: reason.into(),
    }
}
