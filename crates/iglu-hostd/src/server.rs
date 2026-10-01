//! hostd's HTTP surface: the fixed command set from `iglu-proto`.

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
use iglu_domain::capacity::Bytes;
use iglu_domain::env::Arch;
use iglu_domain::id::WorkspaceId;
use iglu_domain::port::GuestPort;
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_proto::{
    AttachParams, BuildOutcome, BuildRequest, Command, CommandError, CommandOutcome, ErrorCode,
    HostReport, Inventory, PROTOCOL_VERSION, TUNNEL_UPGRADE, TerminalInfo, path,
};

use crate::auth::Verifier;
use crate::config::Config;
use crate::incus::Incus;
use crate::observe::{self, BootCache, Owned, Ownership};

pub struct App {
    pub config: Config,
    pub incus: Incus,
    pub boots: BootCache,
    pub verifier: Arc<Verifier>,
    pub arch: Arch,
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route(path::HOST, get(host))
        .route(path::INVENTORY, get(inventory))
        .route(path::BUILDS, post(build))
        .route("/v1/workspaces/{workspace}/commands", post(command))
        .route("/v1/workspaces/{workspace}/terminals", get(terminals))
        .route(
            "/v1/workspaces/{workspace}/terminals/{session}",
            delete(close_terminal),
        )
        .route(
            "/v1/workspaces/{workspace}/terminals/{session}/attach",
            get(attach),
        )
        .route(
            "/v1/workspaces/{workspace}/ports/{port}/tunnel",
            get(tunnel),
        )
        .layer(middleware::from_fn_with_state(app.clone(), authenticate))
        .with_state(app)
}

async fn authenticate(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
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
            ErrorCode::ImageMissing | ErrorCode::GuestFailed => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::Timeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::Runtime => StatusCode::BAD_GATEWAY,
        };
        (status, Json(self.0)).into_response()
    }
}

async fn memory_available() -> Bytes {
    let text = tokio::fs::read_to_string("/proc/meminfo")
        .await
        .unwrap_or_default();
    let kib = text
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|rest| {
            rest.trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0);
    Bytes::new(kib.saturating_mul(1024))
}

async fn host_report(app: &App) -> HostReport {
    HostReport {
        host: app.config.host_id.clone(),
        protocol: PROTOCOL_VERSION,
        arch: app.arch,
        memory_available: memory_available().await,
    }
}

async fn host(State(app): State<Arc<App>>) -> Json<HostReport> {
    Json(host_report(&app).await)
}

async fn inventory(State(app): State<Arc<App>>) -> Result<Json<Inventory>, Failure> {
    let instances = app
        .incus
        .instances()
        .await
        .map_err(|e| Failure(CommandError::new(ErrorCode::Runtime, e.to_string())))?;
    let mut workspaces = Vec::new();
    for instance in instances {
        match Owned::parse(instance) {
            Ok(owned) => workspaces.push(observe::report(&owned, &app.boots).await),
            Err(Ownership::Foreign) => {}
            Err(error @ Ownership::Damaged(..)) => tracing::warn!(%error, "skipping an instance"),
        }
    }
    Ok(Json(Inventory {
        host: host_report(&app).await,
        workspaces,
    }))
}

async fn command(
    State(app): State<Arc<App>>,
    Path(workspace): Path<WorkspaceId>,
    Json(command): Json<Command>,
) -> Json<CommandOutcome> {
    Json(crate::commands::perform(&app, workspace, command).await)
}

async fn terminals(
    State(app): State<Arc<App>>,
    Path(workspace): Path<WorkspaceId>,
) -> Result<Json<Vec<TerminalInfo>>, Failure> {
    crate::terminal::list(&app, workspace)
        .await
        .map(Json)
        .map_err(Failure)
}

async fn close_terminal(
    State(app): State<Arc<App>>,
    Path((workspace, session)): Path<(WorkspaceId, SessionName)>,
) -> Result<StatusCode, Failure> {
    crate::terminal::close(&app, workspace, &session)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(Failure)
}

async fn attach(
    State(app): State<Arc<App>>,
    Path((workspace, session)): Path<(WorkspaceId, SessionName)>,
    Query(params): Query<AttachParams>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let size = TerminalSize::new(params.cols, params.rows).unwrap_or(TerminalSize::DEFAULT);
    upgrade.on_upgrade(move |socket| async move {
        crate::terminal::attach(&app, workspace, session, size, socket).await;
    })
}

async fn tunnel(
    State(app): State<Arc<App>>,
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
    let guest = match crate::tunnel::connect(&app, workspace.instance_name(), port).await {
        Ok(stream) => stream,
        Err(error) => return Failure(error).into_response(),
    };
    let on_upgrade = hyper::upgrade::on(&mut request);
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let mut upgraded = TokioIo::new(upgraded);
                let mut guest = guest;
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

async fn build(
    State(app): State<Arc<App>>,
    Json(request): Json<BuildRequest>,
) -> Json<BuildOutcome> {
    Json(crate::build::build(&app, &request.source, &request.tokens).await)
}
