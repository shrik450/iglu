//! Assembling what the API shows from stored records.

use iglu_api::{AccessGrant, AttentionView, Condition, ProjectView, RouteView, WorkspaceView};
use iglu_domain::attention::{most_urgent, urgency};
use iglu_domain::auth::Grant;
use iglu_domain::env::EnvName;
use iglu_domain::standing::{Facts, Health, Standing, standing};
use rusqlite::Connection;

use crate::config::Config;
use crate::db::{self, DbError};
use crate::model::{AttentionRecord, ProjectRecord, RouteRecord, WorkspaceRecord};

pub fn route_view(config: &Config, route: &RouteRecord) -> RouteView {
    RouteView {
        id: route.id,
        name: route.name.clone(),
        port: route.port,
        url: config.preview_origin(route.name.as_str()),
    }
}

pub fn project_view(project: ProjectRecord) -> ProjectView {
    ProjectView {
        id: project.id,
        name: project.name,
        origin: project.origin,
        repo: project.repo,
        environment: project.environment,
        opening: project.opening,
        agent: project.agent,
        ports: project.ports,
        idle: project.idle,
        revision: project.revision,
        created_at: project.created_at,
    }
}

/// A workspace as the API shows it, with its place in the list.
pub fn workspace_view(
    tx: &Connection,
    config: &Config,
    ws: WorkspaceRecord,
) -> Result<(Standing, WorkspaceView), DbError> {
    let attention = db::attention(tx, ws.id)?;
    let routes = db::routes(tx, ws.id)?;
    let columns = db::columns(tx, ws.id)?;
    let access = access(&db::grants(tx, ws.id)?);
    let image = db::revision_image(tx, ws.env_revision)?;
    let browser = image.as_ref().is_some_and(|image| image.browser);
    let agents = image
        .map(|image| image.agents.into_iter().map(|agent| agent.name).collect())
        .unwrap_or_default();
    let environment = db::environment_name_of_revision(tx, ws.env_revision)?.unwrap_or_else(|| {
        "unknown"
            .parse::<EnvName>()
            .expect("'unknown' is a DNS label")
    });
    let top = most_urgent(attention.iter().map(|record| (&record.status, record.seen)));
    let health = match ws.condition {
        Some(Condition::Error { .. } | Condition::RuntimeFailed) => Health::Trouble,
        Some(Condition::Capacity { .. } | Condition::HostOffline { .. }) | None => Health::Fine,
    };
    let phase = ws.phase();
    let standing = standing(Facts {
        phase,
        health,
        top: top.map(|(status, seen)| (status.state, seen, status.updated_at)),
        created_at: ws.created_at,
    });
    let top = top
        .and_then(|(status, _)| {
            attention.iter().find(|record| {
                record.status.session == status.session && record.status.thread == status.thread
            })
        })
        .map(attention_view);
    let view = WorkspaceView {
        id: ws.id,
        phase,
        name: ws.name,
        project: ws.project,
        checkout: ws.checkout,
        environment,
        desired: ws.desired,
        revision: ws.revision,
        condition: ws.condition,
        needs_you: standing.need,
        memory: ws.memory,
        observed_at: ws.observed_at,
        attention: top,
        threads: {
            let mut threads: Vec<&AttentionRecord> = attention.iter().collect();
            threads.sort_by_key(|record| {
                std::cmp::Reverse((
                    urgency(record.status.state, record.seen),
                    record.status.updated_at,
                ))
            });
            threads.into_iter().map(attention_view).collect()
        },
        columns: columns.into_iter().map(|column| column.spec).collect(),
        agents,
        browser,
        routes: routes
            .iter()
            .map(|route| route_view(config, route))
            .collect(),
        access,
        created_at: ws.created_at,
    };
    Ok((standing, view))
}

/// Grants by the workspace they're on, in the order they come.
fn access(grants: &[Grant]) -> Vec<AccessGrant> {
    let mut access: Vec<AccessGrant> = Vec::new();
    for grant in grants {
        match access.iter_mut().find(|a| a.workspace == grant.workspace) {
            Some(entry) => entry.permissions.push(grant.permission),
            None => access.push(AccessGrant {
                workspace: grant.workspace,
                permissions: vec![grant.permission],
            }),
        }
    }
    access
}

/// Views in standing order, most pressing first.
pub fn ordered(mut views: Vec<(Standing, WorkspaceView)>) -> Vec<WorkspaceView> {
    views.sort_by(|(a, _), (b, _)| b.cmp(a));
    views.into_iter().map(|(_, view)| view).collect()
}

pub fn attention_view(record: &AttentionRecord) -> AttentionView {
    AttentionView {
        session: record.status.session.clone(),
        thread: record.status.thread.clone(),
        title: record.status.title.clone(),
        state: record.status.state,
        summary: record.status.summary.clone(),
        updated_at: record.status.updated_at,
        seen: record.seen,
    }
}
