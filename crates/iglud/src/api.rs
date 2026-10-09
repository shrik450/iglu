//! The versioned JSON API the console and CLI share.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post, put};
use futures_util::Stream;
use iglu_api::{
    ActivityEntry, AddColumn, BuildStarted, ColumnStatus, CreateEnvironment, CreateWorkspace,
    EnvironmentView, LabelColumn, ListenerView, LiveView, Me, PublishPort, PutLayout, PutSecret,
    RenameWorkspace, RouteView, SecretView, SetDesiredState, Snapshot, WorkspaceView,
};
use iglu_domain::agent::{self, Prompt};
use iglu_domain::auth::Action;
use iglu_domain::column::{self, ColumnKind, ColumnSpec, ColumnWidth};
use iglu_domain::env::EnvName;
use iglu_domain::git;
use iglu_domain::id::{EnvRevisionId, PrincipalId, ProjectId, RouteId, SecretId, WorkspaceId};
use iglu_domain::label::{HostId, RouteName, WorkspaceName};
use iglu_domain::lifecycle::{DesiredState, Revision, allow_transition};
use iglu_domain::names;
use iglu_domain::port::GuestPort;
use iglu_domain::project;
use iglu_domain::secret::{FetchTokens, SecretName};
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_proto::{BuildOutcome, SessionSpec};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::app::{ApiError, App, Caller, Problem, no_such_method, no_such_path, now, open_secrets};
use crate::crypto::{self, Binding};
use crate::db::{self, NewRoute, RenameOutcome, SealedSecret};
use crate::extract::{Body, Path, Query, Upgrade};
use crate::model::{ColumnRecord, RouteRecord, WorkspaceRecord};
use crate::views::{ordered, route_view, workspace_view};

mod projects;

pub fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/v1/me", get(me))
        .route(
            "/v1/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route("/v1/workspaces/{id}", get(get_workspace))
        .route("/v1/workspaces/{id}/desired-state", put(set_desired_state))
        .route("/v1/workspaces/{id}/name", put(rename_workspace))
        .route("/v1/workspaces/{id}/seen", post(mark_seen))
        .route("/v1/workspaces/{id}/activity", get(activity))
        .route(
            "/v1/workspaces/{id}/columns",
            get(list_columns).post(add_column),
        )
        .route("/v1/workspaces/{id}/layout", put(put_layout))
        .route(
            "/v1/workspaces/{id}/columns/{session}",
            delete(close_column),
        )
        .route(
            "/v1/workspaces/{id}/columns/{session}/label",
            put(label_column),
        )
        .route(
            "/v1/workspaces/{id}/columns/{session}/restart",
            post(restart_column),
        )
        .route(
            "/v1/workspaces/{id}/columns/{session}/attach",
            get(attach_column),
        )
        .route("/v1/workspaces/{id}/live", get(live))
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
        .route("/v1/cli/token", post(crate::login::cli_token))
        .merge(projects::router())
        .route("/v1/{*rest}", any(no_such_path))
        .method_not_allowed_fallback(no_such_method)
}

async fn me(caller: Caller, State(app): State<Arc<App>>) -> Result<Json<Me>, ApiError> {
    Ok(Json(Me {
        id: caller.principal.id,
        name: caller.principal.name,
        email: caller.principal.email,
        csrf_token: caller.csrf_token,
        preview_domain: app.config.preview_domain.clone(),
        boot: app.boot,
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
        .await?
        .1)
}

/// The owner's workspaces, most pressing first.
pub async fn views_for(app: &Arc<App>, owner: PrincipalId) -> Result<Vec<WorkspaceView>, ApiError> {
    let app2 = app.clone();
    Ok(ordered(
        app.db
            .call(move |tx| {
                db::workspaces(tx, owner)?
                    .into_iter()
                    .map(|ws| workspace_view(tx, &app2.config, ws))
                    .collect()
            })
            .await?,
    ))
}

async fn environment_views(
    app: &Arc<App>,
    owner: PrincipalId,
) -> Result<Vec<EnvironmentView>, ApiError> {
    let envs = app.db.call(move |tx| db::environments(tx, owner)).await?;
    Ok(envs
        .into_iter()
        .map(|(env, latest)| EnvironmentView {
            name: env.name,
            source: env.source,
            latest,
        })
        .collect())
}

async fn snapshot(app: &Arc<App>, owner: PrincipalId) -> Result<Snapshot, ApiError> {
    Ok(Snapshot {
        boot: app.boot,
        workspaces: views_for(app, owner).await?,
        projects: projects::views(app, owner).await?,
        environments: environment_views(app, owner).await?,
    })
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
    Body(request): Body<CreateWorkspace>,
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
    let fresh = Fresh {
        id: WorkspaceId::from_uuid(Uuid::new_v4()),
        host: host.id.clone(),
        entropy: crypto::random_u64()?,
        key,
        request_hash,
    };

    let created = app
        .db
        .call(move |tx| {
            if let Some(key) = &fresh.key
                && let Some((existing, hash)) = db::workspace_by_create_key(tx, owner, key)?
            {
                return Ok(if hash == fresh.request_hash {
                    Ok(existing)
                } else {
                    Err("Idempotency-Key reused with a different request".into())
                });
            }
            insert_requested(tx, owner, request, &fresh)
        })
        .await?;
    let record = created.map_err(ApiError::Conflict)?;
    app.usage.used(record.id);
    app.kick();
    app.changed();
    let view = view(&app, record).await?;
    Ok((StatusCode::ACCEPTED, Json(view)).into_response())
}

/// What the shell decided for a new workspace before the transaction.
struct Fresh {
    id: WorkspaceId,
    host: HostId,
    entropy: u64,
    key: Option<String>,
    request_hash: String,
}

/// Records a requested workspace in its project, with its columns.
fn insert_requested(
    tx: &rusqlite::Connection,
    owner: PrincipalId,
    request: CreateWorkspace,
    fresh: &Fresh,
) -> Result<Result<WorkspaceRecord, Problem>, db::DbError> {
    let Some(project) = db::project(tx, owner, request.project)? else {
        return Ok(Err(Problem::at("project", "no such project")));
    };
    let Some((revision, image)) = db::latest_ready_revision(tx, project.environment_id)? else {
        return Ok(Err(format!(
            "the environment {} has no built image yet",
            project.environment
        )
        .into()));
    };
    let started = match project::start(
        &project.opening,
        &image.agents,
        request.agent.as_ref().or(project.agent.as_ref()),
        request.prompt.as_ref(),
    ) {
        Ok(started) => started,
        Err(refused) => return Ok(Err(refused.to_string().into())),
    };
    let name = match choose_name(
        tx,
        owner,
        request.name,
        request.prompt.as_ref(),
        fresh.entropy,
    )? {
        Ok(name) => name,
        Err(refused) => return Ok(Err(refused)),
    };
    let checkout =
        match project::checkout(project.repo.as_ref(), &name, request.branch, request.base) {
            Ok(checkout) => checkout,
            Err(refused) => return Ok(Err(refused.to_string().into())),
        };
    let record = WorkspaceRecord {
        id: fresh.id,
        owner,
        host: fresh.host.clone(),
        env_revision: revision,
        name,
        project: project.id,
        checkout,
        desired: DesiredState::Running,
        revision: Revision::INITIAL,
        observed: None,
        observed_at: None,
        memory: None,
        condition: None,
        created_at: now(),
    };
    db::insert_workspace(tx, &record, fresh.key.as_deref(), &fresh.request_hash)?;
    let columns: Vec<ColumnRecord> = started
        .columns
        .into_iter()
        .map(|spec| ColumnRecord {
            prompt: request
                .prompt
                .clone()
                .filter(|_| started.prompted.as_ref() == Some(&spec.name)),
            spec,
        })
        .collect();
    db::replace_columns(tx, fresh.id, &columns)?;
    let what = match &record.checkout {
        Some(checkout) => format!("{} on {}", checkout.branch, checkout.repo),
        None => format!("in {}", project.name),
    };
    db::add_activity(tx, Some(fresh.id), Some(owner), "requested", &what, now())?;
    for (n, port) in (0u64..).zip(project.ports.ports()) {
        publish_in(
            tx,
            fresh.id,
            owner,
            None,
            *port,
            fresh.entropy.wrapping_add(n),
        )?;
    }
    Ok(Ok(record))
}

/// Publishes a port of a workspace at a new generated preview name, or
/// returns the route it already has.
fn publish_in(
    tx: &rusqlite::Connection,
    workspace: WorkspaceId,
    owner: PrincipalId,
    actor: Option<PrincipalId>,
    port: GuestPort,
    entropy: u64,
) -> Result<RouteRecord, db::DbError> {
    let mut attempt = 0;
    loop {
        let name = RouteName::try_from(names::candidate(entropy, attempt))
            .expect("generated names are route names");
        match db::create_route(
            tx,
            RouteId::from_uuid(Uuid::new_v4()),
            workspace,
            owner,
            &name,
            port,
            now(),
        )? {
            NewRoute::Created(route) => {
                db::add_activity(
                    tx,
                    Some(workspace),
                    actor,
                    "published",
                    &format!("port {} as {}", route.port, route.name),
                    now(),
                )?;
                return Ok(route);
            }
            NewRoute::Exists(route) => return Ok(route),
            NewRoute::NameTaken => attempt += 1,
        }
    }
}

/// The name asked for if it's free; else one from the prompt; else the
/// first unused generated name.
fn choose_name(
    tx: &rusqlite::Connection,
    owner: PrincipalId,
    asked: Option<WorkspaceName>,
    prompt: Option<&Prompt>,
    entropy: u64,
) -> Result<Result<WorkspaceName, Problem>, db::DbError> {
    let free = |name: &WorkspaceName| -> Result<bool, db::DbError> {
        Ok(!db::workspace_name_taken(tx, owner, name.as_str())?)
    };
    if let Some(name) = asked {
        return Ok(if free(&name)? {
            Ok(name)
        } else {
            Err(Problem::at("name", "a workspace with that name exists"))
        });
    }
    if let Some(prompt) = prompt {
        for attempt in 0..20 {
            let Some(label) = names::from_prompt(prompt.as_str(), attempt) else {
                break;
            };
            if let Ok(name) = WorkspaceName::try_from(label)
                && free(&name)?
            {
                return Ok(Ok(name));
            }
        }
    }
    let mut attempt = 0;
    loop {
        let candidate = WorkspaceName::try_from(names::candidate(entropy, attempt))
            .expect("generated names start with a letter");
        if free(&candidate)? {
            return Ok(Ok(candidate));
        }
        attempt += 1;
    }
}

async fn set_desired_state(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
    Body(request): Body<SetDesiredState>,
) -> Result<Json<WorkspaceView>, ApiError> {
    let action = match request.state {
        DesiredState::Deleted => Action::DeleteWorkspace,
        DesiredState::Running | DesiredState::Frozen | DesiredState::Stopped => {
            Action::OperateWorkspace
        }
    };
    let ws = owned_workspace(&app, &caller, id, action).await?;
    allow_transition(ws.phase(), request.state)
        .map_err(|e| ApiError::Conflict(e.to_string().into()))?;
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
    app.usage.used(id);
    app.kick();
    app.changed();
    Ok(Json(view(&app, updated).await?))
}

async fn rename_workspace(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
    Body(request): Body<RenameWorkspace>,
) -> Result<Json<WorkspaceView>, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::OperateWorkspace).await?;
    let actor = caller.principal.id;
    let owner = ws.owner;
    let renamed = app
        .db
        .call(move |tx| {
            let outcome = db::rename_workspace(tx, id, owner, &request.name)?;
            if outcome == RenameOutcome::Done && ws.name != request.name {
                db::add_activity(
                    tx,
                    Some(id),
                    Some(actor),
                    "renamed",
                    &format!("{} to {}", ws.name, request.name),
                    now(),
                )?;
            }
            Ok((outcome, db::workspace(tx, id)?))
        })
        .await?;
    let updated = match renamed {
        (RenameOutcome::Done, Some(updated)) => updated,
        (RenameOutcome::NameTaken, _) => {
            return Err(ApiError::Conflict(Problem::at(
                "name",
                "another workspace already has that name",
            )));
        }
        (RenameOutcome::Missing | RenameOutcome::Done, _) => return Err(ApiError::NotFound),
    };
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
    // Looking at a workspace is using it.
    app.usage.used(id);
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

pub fn host_error(error: crate::hosts::HostError) -> ApiError {
    let error = error.into_command_error();
    match error.code {
        iglu_proto::ErrorCode::NotFound | iglu_proto::ErrorCode::InvalidState => {
            ApiError::Conflict("the workspace isn't running".into())
        }
        iglu_proto::ErrorCode::Conflict
        | iglu_proto::ErrorCode::ImageMissing
        | iglu_proto::ErrorCode::ImageIncompatible
        | iglu_proto::ErrorCode::GuestFailed
        | iglu_proto::ErrorCode::Timeout
        | iglu_proto::ErrorCode::Runtime => ApiError::Unavailable(error.message),
    }
}

/// The workspace's columns, with whether each one's session is open.
async fn list_columns(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
) -> Result<Json<Vec<ColumnStatus>>, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::ViewWorkspace).await?;
    let asked: Vec<SessionName> = app
        .db
        .call(move |tx| db::columns(tx, id))
        .await?
        .into_iter()
        .map(|column| column.spec.name)
        .collect();
    let open: Vec<(SessionName, u32)> = host_for(&app, &ws)?
        .terminals(id)
        .await
        .map_err(host_error)?
        .into_iter()
        .map(|t| (t.name, t.clients))
        .collect();
    Ok(Json(
        column::join(&asked, &open)
            .into_iter()
            .map(|(name, state, clients)| ColumnStatus {
                name,
                state,
                clients,
            })
            .collect(),
    ))
}

/// What a column runs, resolving an agent through the workspace's environment.
async fn session_for(
    app: &App,
    ws: &WorkspaceRecord,
    spec: &ColumnSpec,
    prompt: Option<&Prompt>,
) -> Result<SessionSpec, ApiError> {
    let command = match &spec.kind {
        ColumnKind::Shell => None,
        ColumnKind::Server { command } => Some(command.clone()),
        ColumnKind::Agent { agent } => {
            let revision = ws.env_revision;
            let agents = app
                .db
                .call(move |tx| db::revision_image(tx, revision))
                .await?
                .map(|image| image.agents)
                .unwrap_or_default();
            Some(
                agent::find(&agents, agent)
                    .and_then(|found| agent::command(found, prompt))
                    .map_err(|e| ApiError::Conflict(e.to_string().into()))?,
            )
        }
    };
    Ok(SessionSpec {
        name: spec.name.clone(),
        command,
    })
}

async fn add_column(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
    Body(request): Body<AddColumn>,
) -> Result<(StatusCode, Json<ColumnSpec>), ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::OperateWorkspace).await?;
    let host = host_for(&app, &ws)?;
    let _editing = app.column_edits.lock(id).await;
    let columns = app.db.call(move |tx| db::columns(tx, id)).await?;
    let open = host.terminals(id).await.map_err(host_error)?;
    let taken: Vec<SessionName> = columns
        .iter()
        .map(|c| c.spec.name.clone())
        .chain(open.into_iter().map(|t| t.name))
        .collect();
    let name = match request.name {
        Some(name) if taken.contains(&name) => {
            return Err(ApiError::Conflict(Problem::at(
                "name",
                format!("{name} is already a column"),
            )));
        }
        Some(name) => name,
        None => column::free_name(request.kind.default_name(), &taken),
    };
    let spec = ColumnSpec {
        name,
        kind: request.kind,
        width: request.width.unwrap_or(ColumnWidth::Half),
        label: None,
    };
    host.open_terminal(id, &session_for(&app, &ws, &spec, None).await?)
        .await
        .map_err(host_error)?;
    // Read again: other changes may have landed while the session opened.
    let added = spec.clone();
    let after = request.after;
    app.db
        .call(move |tx| {
            let mut columns = db::columns(tx, id)?;
            let at = after
                .and_then(|after| columns.iter().position(|c| c.spec.name == after))
                .map_or(columns.len(), |i| i + 1);
            columns.insert(
                at,
                ColumnRecord {
                    spec: added,
                    prompt: None,
                },
            );
            db::replace_columns(tx, id, &columns)
        })
        .await?;
    app.changed();
    Ok((StatusCode::CREATED, Json(spec)))
}

async fn put_layout(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
    Body(request): Body<PutLayout>,
) -> Result<StatusCode, ApiError> {
    owned_workspace(&app, &caller, id, Action::OperateWorkspace).await?;
    let layout: Vec<(SessionName, ColumnWidth)> = request
        .columns
        .into_iter()
        .map(|entry| (entry.name, entry.width))
        .collect();
    let arranged = app
        .db
        .call(move |tx| {
            let columns = db::columns(tx, id)?;
            let specs: Vec<ColumnSpec> = columns.iter().map(|c| c.spec.clone()).collect();
            Ok(match column::arrange(&specs, &layout) {
                Ok(arranged) => {
                    let records: Vec<ColumnRecord> = arranged
                        .into_iter()
                        .map(|spec| ColumnRecord {
                            prompt: columns
                                .iter()
                                .find(|c| c.spec.name == spec.name)
                                .and_then(|c| c.prompt.clone()),
                            spec,
                        })
                        .collect();
                    db::replace_columns(tx, id, &records)?;
                    Ok(())
                }
                Err(mismatch) => Err(mismatch),
            })
        })
        .await?;
    arranged.map_err(|e| ApiError::Conflict(e.to_string().into()))?;
    app.changed();
    Ok(StatusCode::NO_CONTENT)
}

async fn label_column(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path((id, name)): Path<(WorkspaceId, SessionName)>,
    Body(request): Body<LabelColumn>,
) -> Result<StatusCode, ApiError> {
    owned_workspace(&app, &caller, id, Action::OperateWorkspace).await?;
    let found = app
        .db
        .call(move |tx| db::label_column(tx, id, &name, request.label.as_ref()))
        .await?;
    if !found {
        return Err(ApiError::NotFound);
    }
    app.changed();
    Ok(StatusCode::NO_CONTENT)
}

async fn restart_column(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path((id, name)): Path<(WorkspaceId, SessionName)>,
) -> Result<StatusCode, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::OperateWorkspace).await?;
    let lookup = name.clone();
    let column = app
        .db
        .call(move |tx| db::columns(tx, id))
        .await?
        .into_iter()
        .find(|c| c.spec.name == lookup)
        .ok_or(ApiError::NotFound)?;
    // A column that never opened still has its prompt; this is its next try.
    let session = session_for(&app, &ws, &column.spec, column.prompt.as_ref()).await?;
    host_for(&app, &ws)?
        .open_terminal(id, &session)
        .await
        .map_err(host_error)?;
    app.db
        .call(move |tx| db::clear_prompts(tx, id, &[name]))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ends a column's session and everything running in it, and drops the column.
async fn close_column(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path((id, name)): Path<(WorkspaceId, SessionName)>,
) -> Result<StatusCode, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::OperateWorkspace).await?;
    let host = host_for(&app, &ws)?;
    let _editing = app.column_edits.lock(id).await;
    let open = host.terminals(id).await.map_err(host_error)?;
    if open.iter().any(|t| t.name == name) {
        host.close_terminal(id, &name).await.map_err(host_error)?;
    }
    app.db
        .call(move |tx| {
            let mut columns = db::columns(tx, id)?;
            columns.retain(|c| c.spec.name != name);
            db::replace_columns(tx, id, &columns)
        })
        .await?;
    app.changed();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AttachQuery {
    cols: u16,
    rows: u16,
}

async fn attach_column(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path((id, session)): Path<(WorkspaceId, SessionName)>,
    Query(query): Query<AttachQuery>,
    Upgrade(upgrade): Upgrade,
) -> Result<Response, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::OperateWorkspace).await?;
    let host = host_for(&app, &ws)?;
    let size = TerminalSize::new(query.cols, query.rows)?;
    Ok(upgrade.on_upgrade(move |socket| {
        crate::terminal::relay(app, caller, ws, host, session, size, socket)
    }))
}

/// What's going on inside a running workspace now: what's listening, each
/// with its preview if published, and where the checkout stands.
async fn live(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<WorkspaceId>,
) -> Result<Json<LiveView>, ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::ViewWorkspace).await?;
    // The console asks while the workspace is open in front of someone.
    app.usage.used(id);
    let host = host_for(&app, &ws)?;
    let (listeners, git) = tokio::join!(host.listeners(id), host.git_state(id));
    let listeners = listeners.map_err(host_error)?;
    let git = git.map_err(host_error)?;
    let routes = app.db.call(move |tx| db::routes(tx, id)).await?;
    Ok(Json(LiveView {
        listeners: listeners
            .into_iter()
            .map(|listener| ListenerView {
                route: routes
                    .iter()
                    .find(|r| r.port == listener.port)
                    .map(|r| r.id),
                listener,
            })
            .collect(),
        unsaved: git.as_ref().and_then(git::unsaved),
        git,
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
    Body(request): Body<PublishPort>,
) -> Result<(StatusCode, Json<RouteView>), ApiError> {
    let ws = owned_workspace(&app, &caller, id, Action::PublishRoute).await?;
    let entropy = crypto::random_u64()?;
    let owner = ws.owner;
    let actor = caller.principal.id;
    let route = app
        .db
        .call(move |tx| publish_in(tx, id, owner, Some(actor), request.port, entropy))
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
        .call(move |tx| db::delete_route(tx, id, route))
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
    Ok(Json(environment_views(&app, caller.principal.id).await?))
}

async fn create_environment(
    caller: Caller,
    State(app): State<Arc<App>>,
    Body(request): Body<CreateEnvironment>,
) -> Result<(StatusCode, Json<BuildStarted>), ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageEnvironment, owner)?;
    let name = request.name.clone();
    let created = app
        .db
        .call(move |tx| {
            let id = Uuid::new_v4();
            let created =
                db::create_environment(tx, id, owner, &request.name, &request.source, now())?;
            if created {
                // Everyone with an environment has somewhere to start a workspace.
                db::ensure_builtin(tx, owner, id, ProjectId::from_uuid(Uuid::new_v4()), now())?;
            }
            Ok(created)
        })
        .await?;
    if created {
        app.changed();
    }
    if !created {
        return Err(ApiError::Conflict(Problem::at(
            "name",
            "an environment with that name exists",
        )));
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
    let tokens = FetchTokens::from_secrets(open_secrets(app, owner).await?);
    let app = app.clone();
    tokio::spawn(async move {
        let outcome = host.build(&env.source, tokens).await;
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
    Body(request): Body<PutSecret>,
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
                return Ok(Err(Problem::at(
                    "target",
                    format!("{other} already goes there"),
                )));
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

/// Server-sent events: a snapshot of everything the caller sees, whenever
/// anything changes.
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
            match snapshot(&app, caller.principal.id).await {
                Ok(snapshot) => match serde_json::to_string(&snapshot) {
                    Ok(data) => {
                        if sender.send(Event::default().data(data)).await.is_err() {
                            break;
                        }
                    }
                    Err(error) => tracing::warn!(%error, "couldn't serialize the event snapshot"),
                },
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
