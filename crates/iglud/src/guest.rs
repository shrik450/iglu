//! Serving the API to workspaces, over their channels.
//!
//! Each host hands iglud the connections its workspaces make through their
//! channels (see hostd's `channel` module), labelled with the workspace. A
//! label only that host could have put there: iglud asked that host for the
//! connection, over its own authenticated connection, and checks the
//! workspace is on it. iglud serves the routes a workspace may reach on
//! each one, with the request marked as coming from that workspace, which
//! nothing the guest sends can do. Its grants decide the rest.

use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use iglu_domain::id::WorkspaceId;
use tokio::sync::Semaphore;

use crate::app::{App, FromWorkspace};
use crate::db;
use crate::hosts::HostClient;

/// How many connections each host may have asked for at once: one is
/// handed over while another waits.
const ASKING: usize = 2;

/// How many guest connections each host's workspaces may have open at once.
const SERVING: usize = 64;

/// Asks every host for its workspaces' connections, for as long as iglud runs.
pub fn serve(app: &Arc<App>) {
    for host in &app.hosts {
        let serving = Arc::new(Semaphore::new(SERVING));
        for _ in 0..ASKING {
            tokio::spawn(ask(app.clone(), host.clone(), serving.clone()));
        }
    }
}

async fn ask(app: Arc<App>, host: Arc<HostClient>, serving: Arc<Semaphore>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let Ok(permit) = serving.clone().acquire_owned().await else {
            return;
        };
        match host.guest_connection().await {
            Ok(Some((workspace, stream))) => {
                backoff = Duration::from_secs(1);
                let app = app.clone();
                let host = host.clone();
                tokio::spawn(async move {
                    answer(app, &host, workspace, stream).await;
                    drop(permit);
                });
            }
            Ok(None) => backoff = Duration::from_secs(1),
            Err(error) => {
                tracing::debug!(host = %host.id, %error, "couldn't ask for guest connections");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

async fn answer(
    app: Arc<App>,
    host: &HostClient,
    workspace: WorkspaceId,
    stream: reqwest::Upgraded,
) {
    let on_host = app
        .db
        .call(move |tx| db::workspace(tx, workspace))
        .await
        .ok()
        .flatten()
        .is_some_and(|ws| ws.host == host.id);
    if !on_host {
        tracing::warn!(host = %host.id, %workspace, "a host passed on a connection from a workspace it doesn't run");
        return;
    }
    let service = crate::api::guest_router()
        .layer(Extension(FromWorkspace(workspace)))
        .with_state(app);
    if let Err(error) = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(stream), TowerToHyperService::new(service))
        .await
    {
        tracing::debug!(%workspace, %error, "guest connection ended");
    }
}
