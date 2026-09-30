//! Assembling what the API shows from stored records.

use iglu_domain::attention::most_urgent;
use iglu_domain::env::EnvName;
use rusqlite::Connection;

use crate::config::Config;
use crate::db::{self, DbError};
use crate::model::{AttentionView, RouteRecord, RouteView, WorkspaceRecord, WorkspaceView};

pub fn route_view(config: &Config, route: &RouteRecord) -> RouteView {
    RouteView {
        id: route.id,
        name: route.name.clone(),
        port: route.port,
        url: config.preview_origin(route.name.as_str()),
    }
}

pub fn workspace_view(
    tx: &Connection,
    config: &Config,
    ws: WorkspaceRecord,
) -> Result<WorkspaceView, DbError> {
    let attention = db::attention(tx, ws.id)?;
    let routes = db::routes(tx, ws.id)?;
    let environment = db::environment_name_of_revision(tx, ws.env_revision)?.unwrap_or_else(|| {
        "unknown"
            .parse::<EnvName>()
            .expect("'unknown' is a DNS label")
    });
    let top = most_urgent(attention.iter().map(|record| (&record.status, record.seen)))
        .and_then(|(status, _)| {
            attention
                .iter()
                .find(|record| record.status.session == status.session)
        })
        .map(AttentionView::from);
    Ok(WorkspaceView {
        id: ws.id,
        phase: ws.phase(),
        name: ws.name,
        repo: ws.repo,
        branch: ws.branch,
        environment,
        desired: ws.desired,
        revision: ws.revision,
        condition: ws.condition,
        memory: ws.memory,
        observed_at: ws.observed_at,
        attention: top,
        sessions: attention.iter().map(AttentionView::from).collect(),
        routes: routes
            .iter()
            .map(|route| route_view(config, route))
            .collect(),
        created_at: ws.created_at,
    })
}
