//! Idle workspaces: when each was last used, putting unused ones to sleep,
//! and thawing frozen ones that get opened. The rules are the core's.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use iglu_domain::id::WorkspaceId;
use iglu_domain::idle::{self, Busy, IdleStep};
use iglu_domain::lifecycle::{DesiredState, Phase};
use iglu_domain::time::Timestamp;

use crate::app::{App, now};
use crate::db::{self, DbError};
use crate::model::WorkspaceRecord;

/// When each workspace was last used, since this iglud started. Nothing is
/// stored: after a restart every workspace counts as just used, which errs
/// toward keeping it awake.
pub struct Usage {
    started: Timestamp,
    last: Mutex<HashMap<WorkspaceId, Timestamp>>,
}

impl Usage {
    pub fn new(started: Timestamp) -> Self {
        Self {
            started,
            last: Mutex::default(),
        }
    }

    pub fn used(&self, workspace: WorkspaceId) {
        self.last
            .lock()
            .expect("the usage lock is never held across a panic")
            .insert(workspace, now());
    }

    fn last(&self, ws: &WorkspaceRecord) -> Timestamp {
        self.last
            .lock()
            .expect("the usage lock is never held across a panic")
            .get(&ws.id)
            .copied()
            .unwrap_or(self.started)
            .max(ws.created_at)
    }
}

/// Puts a workspace to sleep if the policy says it's been unused too long.
/// Returns whether it did.
pub async fn apply(app: &App, ws: &WorkspaceRecord, busy: Busy) -> Result<bool, DbError> {
    if busy == Busy::Yes {
        app.usage.used(ws.id);
        return Ok(false);
    }
    let project = ws.project;
    let rule = app
        .db
        .call(move |tx| db::project_idle(tx, project))
        .await?
        .unwrap_or_default();
    let step = idle::decide(
        rule.policy(app.config.idle.policy()),
        now(),
        ws.phase(),
        app.usage.last(ws),
        busy,
    );
    let Some(step) = step else {
        return Ok(false);
    };
    let (desired, kind) = match step {
        IdleStep::Freeze => (DesiredState::Frozen, "idle-froze"),
        IdleStep::Stop => (DesiredState::Stopped, "idle-stopped"),
    };
    let (id, revision) = (ws.id, ws.revision);
    app.db
        .call(move |tx| {
            let set = db::set_desired(tx, id, desired, revision)?.is_some();
            if set {
                db::add_activity(tx, Some(id), None, kind, "", now())?;
            }
            Ok(set)
        })
        .await
}

/// Opens a workspace for use: thaws it if it's frozen and waits, at most
/// `bound`, for it to run. Returns whether it's running.
pub async fn thaw(app: &App, ws: &WorkspaceRecord, bound: Duration) -> bool {
    app.usage.used(ws.id);
    if ws.phase() == Phase::Running {
        return true;
    }
    if let Some(desired) = idle::thaw_on_open(ws.desired) {
        let (id, revision) = (ws.id, ws.revision);
        let thawed = app
            .db
            .call(move |tx| {
                let set = db::set_desired(tx, id, desired, revision)?.is_some();
                if set {
                    db::add_activity(tx, Some(id), None, "thawed-on-open", "", now())?;
                }
                Ok(set)
            })
            .await;
        if matches!(thawed, Ok(true)) {
            app.kick();
            app.changed();
        }
    } else if ws.desired != DesiredState::Running {
        return false;
    }
    let deadline = Instant::now() + bound;
    let id = ws.id;
    loop {
        let phase = app
            .db
            .call(move |tx| db::workspace(tx, id))
            .await
            .ok()
            .flatten()
            .map(|ws| ws.phase());
        match phase {
            Some(Phase::Running) => return true,
            None | Some(Phase::Stopping | Phase::Stopped | Phase::Deleting | Phase::Deleted) => {
                return false;
            }
            Some(Phase::Creating | Phase::Starting | Phase::Freezing | Phase::Frozen) => {
                if Instant::now() > deadline {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}
