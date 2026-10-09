//! Projects: listing, adding, changing and removing them.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use iglu_api::{ChangeProject, CreateProject, ProjectView};
use iglu_domain::auth::Action;
use iglu_domain::env::EnvName;
use iglu_domain::id::{PrincipalId, ProjectId};
use iglu_domain::project::Origin;
use uuid::Uuid;

use crate::app::{ApiError, App, Caller, Problem, now};
use crate::db::{self, AddOutcome, ChangeOutcome, NewProject, ProjectChange, RemoveOutcome};
use crate::extract::{Body, Path};
use crate::views::project_view;

pub fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/v1/projects", get(list).post(create))
        .route("/v1/projects/{id}", get(show).put(change).delete(remove))
}

pub async fn views(app: &Arc<App>, owner: PrincipalId) -> Result<Vec<ProjectView>, ApiError> {
    let projects = app.db.call(move |tx| db::projects(tx, owner)).await?;
    Ok(projects.into_iter().map(project_view).collect())
}

async fn list(
    caller: Caller,
    State(app): State<Arc<App>>,
) -> Result<Json<Vec<ProjectView>>, ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageProject, owner)?;
    Ok(Json(views(&app, owner).await?))
}

async fn show(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<ProjectId>,
) -> Result<Json<ProjectView>, ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageProject, owner)?;
    let project = app
        .db
        .call(move |tx| db::project(tx, owner, id))
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(project_view(project)))
}

/// The ID of one of the owner's environments, by name.
fn environment_id(
    tx: &rusqlite::Connection,
    owner: PrincipalId,
    name: &EnvName,
) -> Result<Option<Uuid>, db::DbError> {
    Ok(db::environment(tx, owner, name)?.map(|env| env.id))
}

async fn create(
    caller: Caller,
    State(app): State<Arc<App>>,
    Body(request): Body<CreateProject>,
) -> Result<(StatusCode, Json<ProjectView>), ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageProject, owner)?;
    if request.name.is_none() && request.repo.is_none() {
        return Err(ApiError::BadRequest(
            "give the project a name or a repository".into(),
        ));
    }
    let id = ProjectId::from_uuid(Uuid::new_v4());
    let added = app
        .db
        .call(move |tx| {
            let Some(environment_id) = environment_id(tx, owner, &request.environment)? else {
                return Ok(Err(Problem::at(
                    "environment",
                    format!("no environment called {}", request.environment),
                )));
            };
            let new = NewProject {
                id,
                owner,
                name: request.name,
                origin: Origin::Added,
                repo: request.repo,
                environment_id,
                created_at: now(),
            };
            Ok(match db::add_project(tx, &new)? {
                AddOutcome::Added => Ok(db::project(tx, owner, id)?),
                AddOutcome::NameTaken => {
                    Err(Problem::at("name", "a project with that name exists"))
                }
            })
        })
        .await?
        .map_err(ApiError::Conflict)?
        .ok_or(ApiError::Internal)?;
    app.changed();
    Ok((StatusCode::CREATED, Json(project_view(added))))
}

async fn change(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<ProjectId>,
    Body(request): Body<ChangeProject>,
) -> Result<Json<ProjectView>, ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageProject, owner)?;
    let changed = app
        .db
        .call(move |tx| {
            let Some(environment_id) = environment_id(tx, owner, &request.environment)? else {
                return Ok(Err(ApiError::Conflict(Problem::at(
                    "environment",
                    format!("no environment called {}", request.environment),
                ))));
            };
            let change = ProjectChange {
                name: request.name,
                repo: request.repo,
                environment_id,
                opening: request.opening,
                agent: request.agent,
                ports: request.ports,
                idle: request.idle,
            };
            Ok(
                match db::change_project(tx, owner, id, request.expected_revision, &change)? {
                    ChangeOutcome::Done => db::project(tx, owner, id)?.ok_or(ApiError::NotFound),
                    ChangeOutcome::Stale => Err(ApiError::Conflict(
                        "the project changed since you loaded it".into(),
                    )),
                    ChangeOutcome::NameTaken => Err(ApiError::Conflict(Problem::at(
                        "name",
                        "a project with that name exists",
                    ))),
                    ChangeOutcome::Missing => Err(ApiError::NotFound),
                },
            )
        })
        .await??;
    app.changed();
    Ok(Json(project_view(changed)))
}

async fn remove(
    caller: Caller,
    State(app): State<Arc<App>>,
    Path(id): Path<ProjectId>,
) -> Result<StatusCode, ApiError> {
    let owner = caller.principal.id;
    caller.authorize(Action::ManageProject, owner)?;
    match app
        .db
        .call(move |tx| db::remove_project(tx, owner, id))
        .await?
    {
        RemoveOutcome::Done => {
            app.changed();
            Ok(StatusCode::NO_CONTENT)
        }
        RemoveOutcome::Builtin => Err(ApiError::Conflict(
            "the built-in project can't be removed".into(),
        )),
        RemoveOutcome::InUse => Err(ApiError::Conflict(
            "the project still has workspaces".into(),
        )),
        RemoveOutcome::Missing => Err(ApiError::NotFound),
    }
}
