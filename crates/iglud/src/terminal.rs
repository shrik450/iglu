//! Relaying a browser terminal to hostd, with a five-second authorization lease.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message as Browser, WebSocket};
use futures_util::{SinkExt, StreamExt};
use iglu_domain::auth::{Action, Decision, Resource, authorize};
use iglu_domain::terminal::{SessionName, TerminalControl, TerminalSize};
use tokio_tungstenite::tungstenite::Message as Host;

use crate::app::{App, Caller, now, session_valid};
use crate::db;
use crate::hosts::HostClient;
use crate::model::WorkspaceRecord;

/// How long an authorization decision for an open stream lasts.
pub const LEASE: Duration = Duration::from_secs(5);

/// How long attaching waits for a frozen workspace to thaw.
const THAW: Duration = Duration::from_secs(60);

/// The close code for a terminal iglud ended on purpose. The frame's reason
/// says why, for the console and CLI to show.
const REFUSED: u16 = 4000;

fn refusal(reason: &str) -> Browser {
    Browser::Close(Some(CloseFrame {
        code: REFUSED,
        reason: close_reason(reason).into(),
    }))
}

/// As much of `text` as fits in a close frame's reason: 123 bytes.
fn close_reason(text: &str) -> &str {
    let mut end = text.len().min(123);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Whether the person used a stream since its last lease check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    /// They typed, which keeps an idle browser session alive like a request does.
    Active,
    /// It only carried output, which doesn't.
    Idle,
}

/// Re-checks that the caller's session is still valid and the principal
/// still active. Streams close when this fails.
pub async fn lease_valid(app: &App, caller: &Caller, used: Use) -> bool {
    let hash = caller.session_hash.clone();
    let kind = caller.kind;
    let idle =
        iglu_domain::time::Millis::from_secs(app.config.sessions.idle_minutes.saturating_mul(60));
    let principal = caller.principal.id;
    let result = app
        .db
        .call(move |tx| {
            let Some(row) = db::session(tx, &hash, kind)? else {
                return Ok(false);
            };
            let Some(record) = db::principal(tx, principal)? else {
                return Ok(false);
            };
            let active = matches!(
                authorize(
                    record.principal(),
                    Action::ViewWorkspace,
                    Resource { owner: record.id }
                ),
                Decision::Allow
            );
            let valid = active && session_valid(&row, kind, now(), idle);
            if valid && used == Use::Active {
                db::touch_session(tx, &hash, now())?;
            }
            Ok(valid)
        })
        .await;
    result.unwrap_or(false)
}

async fn still_allowed(app: &App, caller: &Caller, ws: &WorkspaceRecord, used: Use) -> bool {
    let id = ws.id;
    let live = app
        .db
        .call(move |tx| db::workspace(tx, id))
        .await
        .ok()
        .flatten();
    lease_valid(app, caller, used).await
        && live.is_some_and(|ws| caller.authorize(Action::OperateWorkspace, ws.owner).is_ok())
}

pub async fn relay(
    app: Arc<App>,
    caller: Caller,
    ws: WorkspaceRecord,
    host: Arc<HostClient>,
    session: SessionName,
    size: TerminalSize,
    browser: WebSocket,
) {
    let (mut to_browser, mut from_browser) = browser.split();
    // Opening a frozen workspace thaws it.
    if !crate::idle::thaw(&app, &ws, THAW).await {
        let _ = to_browser
            .send(refusal("the workspace isn't running"))
            .await;
        return;
    }
    let upstream = match host.attach(ws.id, &session, size).await {
        Ok(socket) => socket,
        Err(error) => {
            tracing::warn!(workspace = %ws.id, %error, "attach failed");
            let reason = crate::api::host_error(error).to_string();
            let _ = to_browser.send(refusal(&reason)).await;
            return;
        }
    };
    let (mut to_host, mut from_host) = upstream.split();
    let mut lease = tokio::time::interval(LEASE);
    lease.tick().await;
    let mut used = Use::Idle;
    let mut revoked = false;

    loop {
        tokio::select! {
            message = from_browser.next() => match message {
                Some(Ok(Browser::Binary(bytes))) => {
                    used = Use::Active;
                    if to_host.send(Host::Binary(bytes)).await.is_err() { break; }
                }
                Some(Ok(Browser::Text(text))) => {
                    // Forward only well-formed control frames.
                    if let Ok(control) = serde_json::from_str::<TerminalControl>(text.as_str()) {
                        let frame = serde_json::to_string(&control).unwrap_or_default();
                        if to_host.send(Host::text(frame)).await.is_err() { break; }
                    }
                }
                Some(Ok(Browser::Ping(_) | Browser::Pong(_))) => {}
                Some(Ok(Browser::Close(_)) | Err(_)) | None => break,
            },
            message = from_host.next() => match message {
                Some(Ok(Host::Binary(bytes))) => {
                    if to_browser.send(Browser::Binary(bytes)).await.is_err() { break; }
                }
                Some(Ok(Host::Text(_) | Host::Ping(_) | Host::Pong(_) | Host::Frame(_))) => {}
                Some(Ok(Host::Close(_)) | Err(_)) | None => break,
            },
            _ = lease.tick() => {
                // An attached terminal keeps its workspace awake.
                app.usage.used(ws.id);
                if !still_allowed(&app, &caller, &ws, used).await {
                    tracing::info!(workspace = %ws.id, "closing a terminal whose authorization ended");
                    revoked = true;
                    break;
                }
                used = Use::Idle;
            }
        }
    }
    let _ = to_host.close().await;
    if revoked {
        let _ = to_browser
            .send(refusal("access to this workspace ended"))
            .await;
    } else {
        let _ = to_browser.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_reasons_fit_a_frame_without_splitting_a_character() {
        assert_eq!(close_reason("short"), "short");
        let long = "é".repeat(100);
        let cut = close_reason(&long);
        assert!(cut.len() <= 123 && cut.chars().all(|c| c == 'é'), "{cut}");
    }
}
