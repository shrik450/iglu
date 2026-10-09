//! hostd's HTTP surface: the fixed command set from `iglu-proto`, over any
//! runtime.

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use hyper_util::rt::TokioIo;
use iglu_domain::env::Arch;
use iglu_domain::git::GitState;
use iglu_domain::id::WorkspaceId;
use iglu_domain::label::HostId;
use iglu_domain::listener::Listener;
use iglu_domain::port::GuestPort;
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_proto::{
    AttachParams, BuildOutcome, BuildRequest, Command, CommandError, CommandOutcome, ErrorCode,
    HostReport, Inventory, PROTOCOL_VERSION, SessionSpec, TUNNEL_UPGRADE, TerminalInfo, path,
};

use crate::auth::Verifier;
use crate::host::Host;
use crate::runtime::{Isolating, Runtime};

pub struct App<R> {
    pub host_id: HostId,
    pub arch: Arch,
    pub host: Host<R>,
    pub verifier: Arc<Verifier>,
}

/// Serves iglud over TLS on any address. Only an isolating runtime may: a
/// host reachable from the network runs other people's workspaces.
///
/// # Errors
///
/// When the listener fails.
pub async fn serve_tls<R: Isolating>(
    app: Arc<App<R>>,
    listen: SocketAddr,
    tls: axum_server::tls_rustls::RustlsConfig,
) -> std::io::Result<()> {
    tracing::info!(%listen, "hostd listening");
    axum_server::bind_rustls(listen, tls)
        .serve(router(app).into_make_service())
        .await
}

/// Serves iglud over plain HTTP on this machine only, for any runtime.
///
/// # Errors
///
/// When the listener fails.
pub async fn serve_loopback<R: Runtime>(app: Arc<App<R>>, listen: Loopback) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen.0).await?;
    tracing::info!(listen = %listen.0, "hostd listening on loopback");
    axum::serve(listener, router(app)).await
}

/// A socket address on the loopback interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct Loopback(SocketAddr);

#[derive(Debug, thiserror::Error)]
pub enum LoopbackError {
    #[error("not a socket address: {0}")]
    Address(#[from] std::net::AddrParseError),
    #[error("{0} isn't a loopback address")]
    NotLoopback(SocketAddr),
}

impl FromStr for Loopback {
    type Err = LoopbackError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let address: SocketAddr = s.parse()?;
        if address.ip().is_loopback() {
            Ok(Self(address))
        } else {
            Err(LoopbackError::NotLoopback(address))
        }
    }
}

impl TryFrom<String> for Loopback {
    type Error = LoopbackError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

fn router<R: Runtime>(app: Arc<App<R>>) -> Router {
    Router::new()
        .route(path::HOST, get(host::<R>))
        .route(path::INVENTORY, get(inventory::<R>))
        .route(path::BUILDS, post(build::<R>))
        .route("/v1/workspaces/{workspace}/commands", post(command::<R>))
        .route("/v1/workspaces/{workspace}/listeners", get(listeners::<R>))
        .route("/v1/workspaces/{workspace}/git", get(git_state::<R>))
        .route(
            "/v1/workspaces/{workspace}/terminals",
            get(terminals::<R>).post(open_terminal::<R>),
        )
        .route(
            "/v1/workspaces/{workspace}/terminals/{session}",
            delete(close_terminal::<R>),
        )
        .route(
            "/v1/workspaces/{workspace}/terminals/{session}/attach",
            get(attach::<R>),
        )
        .route(
            "/v1/workspaces/{workspace}/ports/{port}/tunnel",
            get(tunnel::<R>),
        )
        .layer(middleware::from_fn_with_state(
            app.clone(),
            authenticate::<R>,
        ))
        .with_state(app)
}

async fn authenticate<R: Runtime>(
    State(app): State<Arc<App<R>>>,
    request: Request,
    next: Next,
) -> Response {
    let header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    match app.verifier.verify(header).await {
        Ok(()) => next.run(request).await,
        Err(error) => {
            tracing::warn!(%error, "rejected a request");
            (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
        }
    }
}

/// A command error as an HTTP response.
struct Failure(CommandError);

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::Conflict | ErrorCode::InvalidState => StatusCode::CONFLICT,
            ErrorCode::ImageMissing | ErrorCode::ImageIncompatible | ErrorCode::GuestFailed => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            ErrorCode::Timeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::Runtime => StatusCode::BAD_GATEWAY,
        };
        (status, Json(self.0)).into_response()
    }
}

async fn host_report<R: Runtime>(app: &App<R>) -> HostReport {
    HostReport {
        host: app.host_id.clone(),
        protocol: PROTOCOL_VERSION,
        arch: app.arch,
        memory_available: app.host.memory_available().await,
    }
}

async fn host<R: Runtime>(State(app): State<Arc<App<R>>>) -> Json<HostReport> {
    Json(host_report(&app).await)
}

async fn inventory<R: Runtime>(State(app): State<Arc<App<R>>>) -> Result<Json<Inventory>, Failure> {
    let workspaces = app.host.inventory().await.map_err(Failure)?;
    Ok(Json(Inventory {
        host: host_report(&app).await,
        workspaces,
    }))
}

async fn command<R: Runtime>(
    State(app): State<Arc<App<R>>>,
    Path(workspace): Path<WorkspaceId>,
    Json(command): Json<Command>,
) -> Json<CommandOutcome> {
    Json(app.host.perform(workspace, command).await)
}

async fn terminals<R: Runtime>(
    State(app): State<Arc<App<R>>>,
    Path(workspace): Path<WorkspaceId>,
) -> Result<Json<Vec<TerminalInfo>>, Failure> {
    app.host
        .terminals(workspace)
        .await
        .map(Json)
        .map_err(Failure)
}

async fn listeners<R: Runtime>(
    State(app): State<Arc<App<R>>>,
    Path(workspace): Path<WorkspaceId>,
) -> Result<Json<Vec<Listener>>, Failure> {
    app.host
        .listeners(workspace)
        .await
        .map(Json)
        .map_err(Failure)
}

async fn git_state<R: Runtime>(
    State(app): State<Arc<App<R>>>,
    Path(workspace): Path<WorkspaceId>,
) -> Result<Json<Option<GitState>>, Failure> {
    app.host
        .git_state(workspace)
        .await
        .map(Json)
        .map_err(Failure)
}

async fn open_terminal<R: Runtime>(
    State(app): State<Arc<App<R>>>,
    Path(workspace): Path<WorkspaceId>,
    Json(session): Json<SessionSpec>,
) -> Result<StatusCode, Failure> {
    app.host
        .open_terminal(workspace, &session)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(Failure)
}

async fn close_terminal<R: Runtime>(
    State(app): State<Arc<App<R>>>,
    Path((workspace, session)): Path<(WorkspaceId, SessionName)>,
) -> Result<StatusCode, Failure> {
    app.host
        .close_terminal(workspace, &session)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(Failure)
}

async fn attach<R: Runtime>(
    State(app): State<Arc<App<R>>>,
    Path((workspace, session)): Path<(WorkspaceId, SessionName)>,
    Query(params): Query<AttachParams>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let size = TerminalSize::new(params.cols, params.rows).unwrap_or(TerminalSize::DEFAULT);
    upgrade.on_upgrade(move |socket| async move {
        let terminal = app.host.attach(workspace, &session, size).await;
        crate::terminal::relay(socket, terminal).await;
    })
}

async fn tunnel<R: Runtime>(
    State(app): State<Arc<App<R>>>,
    Path((workspace, port)): Path<(WorkspaceId, GuestPort)>,
    mut request: Request,
) -> Response {
    let wants_tunnel = request
        .headers()
        .get(header::UPGRADE)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(TUNNEL_UPGRADE.as_bytes()));
    if !wants_tunnel {
        return (StatusCode::BAD_REQUEST, "expected an iglu-tunnel upgrade").into_response();
    }
    let mut guest = match app.host.connect(workspace, port).await {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(%workspace, %port, %error, "tunnel failed");
            return Failure(error).into_response();
        }
    };
    let on_upgrade = hyper::upgrade::on(&mut request);
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let mut upgraded = TokioIo::new(upgraded);
                if let Err(error) = tokio::io::copy_bidirectional(&mut upgraded, &mut guest).await {
                    tracing::debug!(%error, "tunnel closed");
                }
            }
            Err(error) => tracing::debug!(%error, "tunnel upgrade failed"),
        }
    });
    let mut response = StatusCode::SWITCHING_PROTOCOLS.into_response();
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    response
        .headers_mut()
        .insert(header::UPGRADE, HeaderValue::from_static(TUNNEL_UPGRADE));
    response
}

async fn build<R: Runtime>(
    State(app): State<Arc<App<R>>>,
    Json(request): Json<BuildRequest>,
) -> Json<BuildOutcome> {
    Json(app.host.build(&request.source, &request.tokens).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_http_is_served_only_on_loopback() {
        assert!("127.0.0.1:7200".parse::<Loopback>().is_ok());
        assert!("[::1]:7200".parse::<Loopback>().is_ok());
        for exposed in ["0.0.0.0:7200", "192.168.1.5:7200", "[::]:7200"] {
            assert!(exposed.parse::<Loopback>().is_err(), "{exposed}");
        }
    }
}
