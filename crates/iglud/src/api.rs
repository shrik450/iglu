//! The versioned JSON API the console and CLI share.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use futures_util::Stream;
use iglu_api::{
    ActivityEntry, BuildStarted, CreateEnvironment, CreateWorkspace, EnvironmentView, Me,
    NewTerminal, PublishPort, PutSecret, RouteView, SecretView, SetDesiredState, TerminalView,
    WorkspaceView,
};
use iglu_domain::auth::Action;
use iglu_domain::env::EnvName;
use iglu_domain::id::{EnvRevisionId, PrincipalId, RouteId, SecretId, WorkspaceId};
use iglu_domain::label::{RouteName, WorkspaceName};
use iglu_domain::lifecycle::{DesiredState, Revision, allow_transition};
use iglu_domain::names;
use iglu_domain::secret::SecretName;
use iglu_domain::terminal::{SessionName, TerminalSize, next_session_name};
use iglu_proto::BuildOutcome;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::app::{ApiError, App, Caller, now};
use crate::crypto::{self, Binding};
use crate::db::{self, NewRoute, SealedSecret};
use crate::model::WorkspaceRecord;
use crate::views::{route_view, workspace_view};

pub fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/v1/me", get(me))
        .route(
            "/v1/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route("/v1/workspaces/{id}", get(get_workspace))
        .route("/v1/workspaces/{id}/desired-state", put(set_desired_state))
        .route("/v1/workspaces/{id}/seen", post(mark_seen))
        .route("/v1/workspaces/{id}/activity", get(activity))
        .route(
            "/v1/workspaces/{id}/terminals",
            get(list_terminals).post(new_terminal),
        )
        .route(
            "/v1/workspaces/{id}/terminals/{session}",
            delete(close_terminal),
        )
        .route(
            "/v1/workspaces/{id}/terminals/{session}/attach",
            get(attach_terminal),
        )
        .route(
            "/v1/workspaces/{id}/routes",
            get(list_routes).post(publish_route),
        )
        .route(
            "/v1/workspaces/{id}/routes/{route}",
            delete(unpublish_route),
        )
        .route(
            "/v1/environments",
            get(list_environments).post(create_environment),
        )
        .route("/v1/environments/{name}/builds", post(build_environment))
        .route("/v1/secrets", get(list_secrets))
        .route("/v1/secrets/{name}", put(put_secret).delete(delete_secret))
        .route("/v1/events", get(events))
}

async fn me(caller: Caller, State(app): State<Arc<App>>) -> Result<Json<Me>, ApiError> {
    let hash = caller.session_hash.clone();
    let csrf = app
        .db
        .call(move |tx| {
            Ok(tx.query_row(
                "SELECT csrf FROM web_session WHERE token_hash = ?1",
                [hash],
                |row| row.get::<_, String>(0),
            )?)
        })
        .await?;
    Ok(Json(Me {
        id: caller.principal.id,
        name: caller.principal.name,
        email: caller.principal.email,
        csrf_token: csrf,
        preview_domain: app.config.preview_domain.clone(),
    }))
}

/// Loads a live workspace the caller may act on.
async fn owned_workspace(
    app: &App,
    caller: &Caller,
    id: WorkspaceId,
    action: Action,
) -> Result<WorkspaceRecord, ApiError> {
    let ws = app
        .db
        .call(move |tx| db::workspace(tx, id))
        .await?
        .ok_or(ApiError::NotFound)?;
    caller.authorize(action, ws.owner)?;
    Ok(ws)
}

async fn view(app: &Arc<App>, ws: WorkspaceRecord) -> Result<WorkspaceView, ApiError> {
    let app2 = app.clone();
    Ok(app
        .db
        .call(move |tx| workspace_view(tx, &app2.config, ws))
        .await?)
}

pub async fn views_for(app: &Arc<App>, owner: PrincipalId) -> Result<Vec<WorkspaceView>, ApiError> {
    let app2 = app.clone();
    Ok(app
        .db
        .call(move |tx| {
            db::workspaces(tx, owner)?
                .into_iter()
                .map(|ws| workspace_view(tx, &app2.config, ws))
                .collect()
        })
        .await?)
}

async fn list_workspaces(
    caller: Caller,
    State(app): State<Arc<App>>,
) -> Result<Json<Vec<WorkspaceView>>, ApiError> {
    Ok(Json(views_for(&app, caller.principal.id).await?))
}

async fn get_workspace(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
) -> Result<Json<WorkspaceView>, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::ViewWorkspace).await?;
    Ok(Json(view(&app, ws).await?))
}

async fn create_workspace(
    caller: Caller,
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(request): Json<CreateWorkspace>,
) -> Result<Response, ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::OperateWorkspace, owner)?;
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if key.as_ref().is_some_and(|k| k.is_empty() || k.len() > 200) {
        return Err(ApiError::BadRequest(
            "Idempotency-Key must be 1-200 characters".into(),
        ));
    }
    let request_hash = hex::encode(Sha256::digest(
        serde_json::to_vec(&request).map_err(|_| ApiError::Internal)?,
    ));
    let host = app
        .default_host()
        .ok_or_else(|| ApiError::Unavailable("no execution host configured".into()))?;
    let entropy = crypto::random_u64()?;
    let id = WorkspaceId::from_uuid(Uuid::new_v4());

    let created = app
        .db
        .call(move |tx| {
            if let Some(key) = &key
                && let Some((existing, hash)) = db::workspace_by_create_key(tx, owner, key)?
            {
                return Ok(if hash == request_hash {
                    Ok(existing)
                } else {
                    Err("Idempotency-Key reused with a different request")
                });
            }
            let Some(env) = db::environment(tx, owner, &request.environment)? else {
                return Ok(Err("no such environment"));
            };
            let Some((revision, _)) = db::latest_ready_revision(tx, env.id)? else {
                return Ok(Err("the environment has no built image yet"));
            };
            let name = if let Some(name) = request.name {
                if db::workspace_name_taken(tx, owner, name.as_str())? {
                    return Ok(Err("a workspace with that name exists"));
                }
                name
            } else {
                let mut attempt = 0;
                loop {
                    let candidate = WorkspaceName::try_from(names::candidate(entropy, attempt))
                        .expect("generated names start with a letter");
                    if !db::workspace_name_taken(tx, owner, candidate.as_str())? {
                        break candidate;
                    }
                    attempt += 1;
                }
            };
            let branch = match request.branch {
                Some(branch) => branch,
                None => name
                    .as_str()
                    .parse()
                    .expect("workspace names are valid branch names"),
            };
            let record = WorkspaceRecord {
                id,
                owner,
                host: host.id.clone(),
                env_revision: revision,
                name,
                repo: request.repo,
                branch,
                base: request.base,
                desired: DesiredState::Running,
                revision: Revision::INITIAL,
                observed: None,
                observed_at: None,
                memory: None,
                condition: None,
                created_at: now(),
            };
            db::insert_workspace(tx, &record, key.as_deref(), &request_hash)?;
            db::add_activity(
                tx,
                Some(id),
                Some(owner),
                "requested",
                &format!("{} on {}", record.branch, record.repo),
                now(),
            )?;
            Ok(Ok(record))
        })
        .await?;
    let record = created.map_err(|message| ApiError::Conflict(message.into()))?;
    app.kick();
    app.changed();
    let view = view(&app, record).await?;
    Ok((StatusCode::ACCEPTED, Json(view)).into_response())
}

async fn set_desired_state(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
    Json(request): Json<SetDesiredState>,
) -> Result<Json<WorkspaceView>, ApiError> {
    let action = match request.state {
        DesiredState::Deleted => Action::DeleteWorkspace,
        DesiredState::Running | DesiredState::Frozen | DesiredState::Stopped => {
            Action::OperateWorkspace
        }
    };
    let ws = owned_workspace(&app, &caller, id, action).await?;
    allow_transition(ws.phase(), request.state).map_err(|e| ApiError::Conflict(e.to_string()))?;
    let actor = caller.principal.id;
    let updated = app
        .db
        .call(move |tx| {
            let revision = db::set_desired(tx, id, request.state, request.expected_revision)?;
            if revision.is_some() {
                db::add_activity(
                    tx,
                    Some(id),
                    Some(actor),
                    "requested",
                    &format!("set to {}", request.state),
                    now(),
                )?;
            }
            revision
                .and_then(|_| db::workspace(tx, id).transpose())
                .transpose()
        })
        .await?
        .ok_or_else(|| ApiError::Conflict("the workspace changed; reload and retry".into()))?;
    app.kick();
    app.changed();
    Ok(Json(view(&app, updated).await?))
}

async fn mark_seen(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
) -> Result<StatusCode, ApiError> {
    owned_workspace(&app, &caller, id, Action::ViewWorkspace).await?;
    app.db.call(move |tx| db::mark_seen(tx, id, now())).await?;
    app.changed();
    Ok(StatusCode::NO_CONTENT)
}

async fn activity(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
) -> Result<Json<Vec<ActivityEntry>>, ApiError> {
    owned_workspace(&app, &caller, id, Action::ViewWorkspace).await?;
    Ok(Json(
        app.db.call(move |tx| db::activity(tx, id, 100)).await?,
    ))
}

fn host_for(app: &App, ws: &WorkspaceRecord) -> Result<Arc<crate::hosts::HostClient>, ApiError> {
    app.host(&ws.host)
        .ok_or_else(|| ApiError::Unavailable("the workspace's host isn't configured".into()))
}

fn host_error(error: crate::hosts::HostError) -> ApiError {
    let error = error.into_command_error();
    match error.code {
        iglu_proto::ErrorCode::NotFound | iglu_proto::ErrorCode::InvalidState => {
            ApiError::Conflict("the workspace isn't running".into())
        }
        iglu_proto::ErrorCode::Conflict
        | iglu_proto::ErrorCode::ImageMissing
        | iglu_proto::ErrorCode::GuestFailed
        | iglu_proto::ErrorCode::Timeout
        | iglu_proto::ErrorCode::Runtime => ApiError::Unavailable(error.message),
    }
}

async fn list_terminals(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
) -> Result<Json<Vec<TerminalView>>, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::ViewWorkspace).await?;
    let terminals = host_for(&app, &ws)?
        .terminals(id)
        .await
        .map_err(host_error)?;
    Ok(Json(
        terminals
            .into_iter()
            .map(|t| TerminalView {
                name: t.name,
                clients: t.clients,
            })
            .collect(),
    ))
}

/// Picks a name for a new terminal. The session itself starts on first attach.
async fn new_terminal(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
) -> Result<Json<NewTerminal>, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::OperateWorkspace).await?;
    let existing = host_for(&app, &ws)?
        .terminals(id)
        .await
        .map_err(host_error)?;
    let name = next_session_name(existing.iter().map(|t| &t.name));
    Ok(Json(NewTerminal { name }))
}

async fn close_terminal(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path((id, session)): Path<(WorkspaceId, SessionName)>,
) -> Result<StatusCode, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::OperateWorkspace).await?;
    host_for(&app, &ws)?
        .close_terminal(id, &session)
        .await
        .map_err(host_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AttachQuery {
    cols: u16,
    rows: u16,
}

async fn attach_terminal(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path((id, session)): Path<(WorkspaceId, SessionName)>,
    Query(query): Query<AttachQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::OperateWorkspace).await?;
    let host = host_for(&app, &ws)?;
    let size = TerminalSize::new(query.cols, query.rows)?;
    Ok(upgrade.on_upgrade(move |socket| {
        crate::terminal::relay(app, caller, ws, host, session, size, socket)
    }))
}

async fn list_routes(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
) -> Result<Json<Vec<RouteView>>, ApiError> {
    owned_workspace(&app, &caller, id, Action::ViewWorkspace).await?;
    let app2 = app.clone();
    Ok(Json(
        app.db
            .call(move |tx| {
                Ok(db::routes(tx, id)?
                    .iter()
                    .map(|r| route_view(&app2.config, r))
                    .collect())
            })
            .await?,
    ))
}

async fn publish_route(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
    Json(request): Json<PublishPort>,
) -> Result<(StatusCode, Json<RouteView>), ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::PublishRoute).await?;
    let entropy = crypto::random_u64()?;
    let owner = ws.owner;
    let actor = caller.principal.id;
    let route = app
        .db
        .call(move |tx| {
            let mut attempt = 0;
            loop {
                let name = RouteName::try_from(names::candidate(entropy, attempt))
                    .expect("generated names are route names");
                match db::create_route(
                    tx,
                    RouteId::from_uuid(Uuid::new_v4()),
                    id,
                    owner,
                    &name,
                    request.port,
                    now(),
                )? {
                    NewRoute::Created(route) => {
                        db::add_activity(
                            tx,
                            Some(id),
                            Some(actor),
                            "published",
                            &format!("port {} as {}", route.port, route.name),
                            now(),
                        )?;
                        break Ok(route);
                    }
                    NewRoute::Exists(route) => break Ok(route),
                    NewRoute::NameTaken => attempt += 1,
                }
            }
        })
        .await?;
    app.changed();
    Ok((StatusCode::CREATED, Json(route_view(&app.config, &route))))
}

async fn unpublish_route(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path((id, route)): Path<(WorkspaceId, RouteId)>,
) -> Result<StatusCode, ApiError> {
    owned_workspace(&app, &caller, id, Action::PublishRoute).await?;
    let removed = app
        .db
        .call(move |tx| db::delete_route(tx, id, route, now()))
        .await?;
    app.changed();
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

async fn list_environments(
    caller: Caller,
    State(app): State<Arc<App>>,
) -> Result<Json<Vec<EnvironmentView>>, ApiError> {
    let owner = caller.principal.id;
    let envs = app.db.call(move |tx| db::environments(tx, owner)).await?;
    Ok(Json(
        envs.into_iter()
            .map(|(env, latest)| EnvironmentView {
                name: env.name,
                source: env.source,
                latest,
            })
            .collect(),
    ))
}

async fn create_environment(
    caller: Caller,
    State(app): State<Arc<App>>,
    Json(request): Json<CreateEnvironment>,
) -> Result<(StatusCode, Json<BuildStarted>), ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageEnvironment, owner)?;
    let name = request.name.clone();
    let created = app
        .db
        .call(move |tx| {
            db::create_environment(
                tx,
                Uuid::new_v4(),
                owner,
                &request.name,
                &request.source,
                now(),
            )
        })
        .await?;
    if !created {
        return Err(ApiError::Conflict(
            "an environment with that name exists".into(),
        ));
    }
    let revision = start_build(&app, owner, name).await?;
    Ok((StatusCode::ACCEPTED, Json(BuildStarted { revision })))
}

async fn build_environment(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(name): Path<EnvName>,
) -> Result<(StatusCode, Json<BuildStarted>), ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageEnvironment, owner)?;
    let revision = start_build(&app, owner, name).await?;
    Ok((StatusCode::ACCEPTED, Json(BuildStarted { revision })))
}

/// Records a new revision and builds it on the host in the background.
async fn start_build(
    app: &Arc<App>,
    owner: PrincipalId,
    name: EnvName,
) -> Result<EnvRevisionId, ApiError> {
    let host = app
        .default_host()
        .ok_or_else(|| ApiError::Unavailable("no execution host configured".into()))?;
    let revision = EnvRevisionId::from_uuid(Uuid::new_v4());
    let env = app
        .db
        .call(move |tx| {
            let Some(env) = db::environment(tx, owner, &name)? else {
                return Ok(None);
            };
            db::insert_revision(tx, revision, env.id, now())?;
            Ok(Some(env))
        })
        .await?
        .ok_or(ApiError::NotFound)?;
    let app = app.clone();
    tokio::spawn(async move {
        let outcome = host.build(&env.source).await;
        let stored = app
            .db
            .call(move |tx| match outcome {
                Ok(BuildOutcome::Built(image)) => {
                    db::finish_revision(tx, revision, Ok(&image), now())
                }
                Ok(BuildOutcome::Failed { log_tail }) => {
                    db::finish_revision(tx, revision, Err(&log_tail), now())
                }
                Err(error) => db::finish_revision(tx, revision, Err(&error.to_string()), now()),
            })
            .await;
        if let Err(error) = stored {
            tracing::error!(%error, "couldn't record a build result");
        }
        app.changed();
    });
    Ok(revision)
}

async fn list_secrets(
    caller: Caller,
    State(app): State<Arc<App>>,
) -> Result<Json<Vec<SecretView>>, ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageSecrets, owner)?;
    Ok(Json(app.db.call(move |tx| db::secrets(tx, owner)).await?))
}

async fn put_secret(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(name): Path<SecretName>,
    Json(request): Json<PutSecret>,
) -> Result<StatusCode, ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageSecrets, owner)?;
    let (nonce, ciphertext) = app.sealer.seal(
        request.value.expose().as_bytes(),
        &Binding {
            owner,
            name: &name,
            target: &request.target,
        },
    )?;
    let sealed = SealedSecret {
        name,
        target: request.target,
        nonce,
        ciphertext,
    };
    let stored = app
        .db
        .call(move |tx| {
            let others = db::other_secret_targets(tx, owner, &sealed.name)?;
            if let Some((other, _)) = others
                .iter()
                .find(|(_, target)| target.conflicts_with(&sealed.target))
            {
                return Ok(Err(format!("{other} already goes there")));
            }
            db::put_secret(
                tx,
                SecretId::from_uuid(Uuid::new_v4()),
                owner,
                &sealed,
                now(),
            )?;
            db::add_activity(
                tx,
                None,
                Some(owner),
                "secret-updated",
                sealed.name.as_str(),
                now(),
            )?;
            Ok(Ok(()))
        })
        .await?;
    stored.map_err(ApiError::Conflict)?;
    app.kick();
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_secret(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(name): Path<SecretName>,
) -> Result<StatusCode, ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageSecrets, owner)?;
    let removed = app
        .db
        .call(move |tx| db::delete_secret(tx, owner, &name))
        .await?;
    app.kick();
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// Server-sent events: the caller's workspaces, whenever anything changes.
/// The stream ends when the session stops being valid.
async fn events(
    caller: Caller,
    State(app): State<Arc<App>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut changes = app.changes.subscribe();
    let stream = async_stream(move |sender| async move {
        loop {
            if !crate::terminal::lease_valid(&app, &caller, crate::terminal::Use::Idle).await {
                break;
            }
            match views_for(&app, caller.principal.id).await {
                Ok(views) => {
                    let data = serde_json::to_string(&views).unwrap_or_else(|_| "[]".into());
                    if sender
                        .send(Event::default().event("workspaces").data(data))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => tracing::warn!(%error, "couldn't build the event snapshot"),
            }
            // Coalesce bursts of changes, and re-check the session at least every five seconds.
            let _ = tokio::time::timeout(Duration::from_secs(5), changes.recv()).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            while changes.try_recv().is_ok() {}
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// A stream fed by a producer task over a small channel.
fn async_stream<F, Fut>(producer: F) -> impl Stream<Item = Result<Event, Infallible>>
where
    F: FnOnce(tokio::sync::mpsc::Sender<Event>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (sender, receiver) = tokio::sync::mpsc::channel(4);
    tokio::spawn(producer(sender));
    futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|event| (Ok(event), receiver))
    })
}
