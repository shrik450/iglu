//! The reconciler: observe every host, decide each workspace's next step
//! with the core's `plan()`, perform it, and record what happened.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iglu_domain::capacity::admit;
use iglu_domain::id::WorkspaceId;
use iglu_domain::lifecycle::{
    AdoptReason, Blocker, Desired, DesiredState, Effect, Instance, Plan, Wait, plan,
};
use iglu_domain::secret::{SecretValue, bundle};
use iglu_proto::{
    Command, CommandError, CommandOutcome, CreateSpec, ErrorCode, InstanceReport, Inventory,
    Limits, ProvisionSpec,
};

use crate::app::{App, now};
use crate::db;
use crate::hosts::HostClient;
use crate::model::{Condition, PrincipalRecord, WorkspaceRecord};

const INTERVAL: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(300);

#[derive(Default)]
struct Backoff {
    failures: u32,
    until: Option<Instant>,
}

#[derive(Default)]
pub struct Reconciler {
    busy: Mutex<HashSet<WorkspaceId>>,
    backoff: Mutex<HashMap<WorkspaceId, Backoff>>,
    pub kick: tokio::sync::Notify,
}

impl Reconciler {
    fn try_claim(&self, id: WorkspaceId) -> bool {
        let ready = self
            .backoff
            .lock()
            .expect("the backoff lock is never held across a panic")
            .get(&id)
            .and_then(|b| b.until)
            .is_none_or(|until| Instant::now() >= until);
        ready
            && self
                .busy
                .lock()
                .expect("the busy lock is never held across a panic")
                .insert(id)
    }

    fn release(&self, id: WorkspaceId, succeeded: bool) {
        self.busy
            .lock()
            .expect("the busy lock is never held across a panic")
            .remove(&id);
        let mut backoff = self
            .backoff
            .lock()
            .expect("the backoff lock is never held across a panic");
        if succeeded {
            backoff.remove(&id);
        } else {
            let entry = backoff.entry(id).or_default();
            entry.failures = entry.failures.saturating_add(1);
            let delay =
                Duration::from_secs(2u64.saturating_pow(entry.failures.min(9))).min(MAX_BACKOFF);
            entry.until = Some(Instant::now() + delay);
        }
    }
}

pub async fn run(app: Arc<App>) {
    let reconciler = app.reconciler.clone();
    let mut last_cleanup = Instant::now();
    loop {
        for host in &app.hosts {
            match host.inventory().await {
                Ok(inventory) => {
                    app.host_seen
                        .lock()
                        .expect("the host lock is never held across a panic")
                        .insert(host.id.clone(), now());
                    if let Err(error) = reconcile_host(&app, &reconciler, host, inventory).await {
                        tracing::error!(host = %host.id, %error, "reconciling failed");
                    }
                }
                Err(error) => {
                    tracing::warn!(host = %host.id, %error, "host unreachable");
                    mark_offline(&app, host).await;
                }
            }
        }
        if last_cleanup.elapsed() > Duration::from_secs(300) {
            let _ = app.db.call(|tx| db::expire_sessions(tx, now())).await;
            last_cleanup = Instant::now();
        }
        tokio::select! {
            () = tokio::time::sleep(INTERVAL) => {}
            () = reconciler.kick.notified() => {}
        }
    }
}

async fn mark_offline(app: &App, host: &HostClient) {
    let last_seen = app
        .host_seen
        .lock()
        .expect("the host lock is never held across a panic")
        .get(&host.id)
        .copied();
    let host_id = host.id.clone();
    let result = app
        .db
        .call(move |tx| {
            let mut changed = false;
            for ws in db::live_workspaces(tx)?
                .into_iter()
                .filter(|ws| ws.host == host_id)
            {
                let offline = Condition::HostOffline { last_seen };
                if ws.condition.as_ref() != Some(&offline) {
                    db::set_condition(tx, ws.id, Some(&offline))?;
                    changed = true;
                }
            }
            Ok(changed)
        })
        .await;
    if matches!(result, Ok(true)) {
        app.changed();
    }
}

async fn reconcile_host(
    app: &Arc<App>,
    reconciler: &Arc<Reconciler>,
    host: &Arc<HostClient>,
    inventory: Inventory,
) -> anyhow::Result<()> {
    let host_id = host.id.clone();
    let reports: HashMap<WorkspaceId, InstanceReport> = inventory
        .workspaces
        .into_iter()
        .map(|report| (report.workspace, report))
        .collect();
    let known: HashSet<WorkspaceId> = app
        .db
        .call(|tx| {
            Ok(db::live_workspaces(tx)?
                .into_iter()
                .map(|ws| ws.id)
                .collect())
        })
        .await?;
    for orphan in reports.keys().filter(|id| !known.contains(id)) {
        tracing::debug!(workspace = %orphan, "the host has an instance with no live workspace");
    }

    let workspaces: Vec<WorkspaceRecord> = app
        .db
        .call(move |tx| {
            Ok(db::live_workspaces(tx)?
                .into_iter()
                .filter(|ws| ws.host == host_id)
                .collect())
        })
        .await?;
    let capacity = admit(
        inventory.host.memory_available,
        app.config.workspaces.reservation,
        app.config.workspaces.headroom,
    );
    let mut changed = false;

    for ws in workspaces {
        let report = reports.get(&ws.id);
        let instance = report.map_or(Instance::Absent, |r| r.instance);
        let (principal, observed) = observe(app, &ws, instance, report).await?;
        changed |= observed;
        let Some(principal) = principal else { continue };
        let desired = Desired {
            state: ws.desired,
            secrets: principal.secrets_generation,
        };
        let next = plan(desired, instance, capacity);
        changed |= act(app, reconciler, host, ws, principal, next).await?;
    }
    if changed {
        app.changed();
    }
    Ok(())
}

/// Records what the host reports about a workspace, and loads its owner.
/// Returns whether anything the console shows changed.
async fn observe(
    app: &App,
    ws: &WorkspaceRecord,
    instance: Instance,
    report: Option<&InstanceReport>,
) -> anyhow::Result<(Option<PrincipalRecord>, bool)> {
    let memory = report.and_then(|r| r.memory);
    let sessions = report.map(|r| r.sessions.clone()).unwrap_or_default();
    let owner = ws.owner;
    let id = ws.id;
    let observed_changed = ws.observed != Some(instance) || ws.memory != memory;
    let clear_offline = matches!(ws.condition, Some(Condition::HostOffline { .. }));

    let (principal, attention_changed) = app
        .db
        .call(move |tx| {
            if observed_changed {
                db::record_observation(tx, id, &instance, memory, now())?;
            }
            if clear_offline {
                db::set_condition(tx, id, None)?;
            }
            let attention_changed = db::sync_attention(tx, id, &sessions)?;
            Ok((db::principal(tx, owner)?, attention_changed))
        })
        .await?;
    Ok((
        principal,
        observed_changed || attention_changed || clear_offline,
    ))
}

/// Carries out a plan: records conditions and adoptions, or starts the
/// effect in the background. Returns whether anything the console shows
/// changed.
async fn act(
    app: &Arc<App>,
    reconciler: &Arc<Reconciler>,
    host: &Arc<HostClient>,
    ws: WorkspaceRecord,
    principal: PrincipalRecord,
    next: Plan,
) -> anyhow::Result<bool> {
    let id = ws.id;
    match next {
        Plan::Stable if ws.desired == DesiredState::Deleted => {
            app.db
                .call(move |tx| {
                    db::mark_deleted(tx, id, now())?;
                    db::add_activity(
                        tx,
                        Some(id),
                        None,
                        "deleted",
                        "the workspace was deleted",
                        now(),
                    )
                })
                .await?;
            Ok(true)
        }
        Plan::Stable => {
            let resolved = matches!(
                ws.condition,
                Some(Condition::Capacity { .. } | Condition::RuntimeFailed)
            );
            if resolved {
                app.db
                    .call(move |tx| db::set_condition(tx, id, None))
                    .await?;
            }
            Ok(resolved)
        }
        Plan::Wait(Wait::Capacity(short)) => {
            set_condition(app, &ws, Condition::from_capacity(short)).await
        }
        Plan::Wait(Wait::Booting | Wait::Transition) => Ok(false),
        Plan::Blocked(Blocker::RuntimeFailed) => {
            set_condition(app, &ws, Some(Condition::RuntimeFailed)).await
        }
        Plan::Adopt(state, reason) => {
            let detail = match reason {
                AdoptReason::FrozenStateLost => {
                    "the host restarted, so the frozen workspace is now stopped"
                }
            };
            app.db
                .call(move |tx| {
                    db::set_desired(tx, id, state, None)?;
                    db::add_activity(tx, Some(id), None, "adopted", detail, now())
                })
                .await?;
            Ok(true)
        }
        Plan::Perform(effect) => {
            if reconciler.try_claim(id) {
                let app = app.clone();
                let reconciler = reconciler.clone();
                let host = host.clone();
                tokio::spawn(async move {
                    let succeeded = perform(&app, &host, ws, &principal, effect).await;
                    reconciler.release(id, succeeded);
                    app.changed();
                    reconciler.kick.notify_one();
                });
            }
            Ok(false)
        }
    }
}

/// Stores a workspace's condition if it differs. Returns whether it did.
async fn set_condition(
    app: &App,
    ws: &WorkspaceRecord,
    condition: Option<Condition>,
) -> anyhow::Result<bool> {
    if ws.condition == condition {
        return Ok(false);
    }
    let id = ws.id;
    app.db
        .call(move |tx| db::set_condition(tx, id, condition.as_ref()))
        .await?;
    Ok(true)
}

const fn effect_name(effect: Effect) -> &'static str {
    match effect {
        Effect::Create => "created",
        Effect::Start => "started",
        Effect::DeliverSecrets => "secrets-delivered",
        Effect::Provision => "provisioned",
        Effect::Freeze => "frozen",
        Effect::Thaw => "thawed",
        Effect::Stop => "stopped",
        Effect::Delete => "removed",
    }
}

async fn perform(
    app: &App,
    host: &HostClient,
    ws: WorkspaceRecord,
    principal: &PrincipalRecord,
    effect: Effect,
) -> bool {
    let id = ws.id;
    let outcome = match command_for(app, &ws, principal, effect).await {
        Ok(command) => host
            .command(id, &command)
            .await
            .map_err(crate::hosts::HostError::into_command_error),
        Err(error) => Err(error),
    };
    let result = match outcome {
        Ok(CommandOutcome::Done { instance }) => Ok(instance),
        Ok(CommandOutcome::Failed(error)) | Err(error) => Err(error),
    };
    let succeeded = result.is_ok();
    let stored = app
        .db
        .call(move |tx| {
            match &result {
                Ok(instance) => {
                    db::record_observation(tx, id, instance, None, now())?;
                    db::set_condition(tx, id, None)?;
                    if !matches!(effect, Effect::DeliverSecrets) {
                        db::add_activity(tx, Some(id), None, effect_name(effect), "", now())?;
                    }
                }
                Err(error) => {
                    let condition = Condition::Error {
                        code: error.code,
                        message: error.message.clone(),
                        at: now(),
                    };
                    db::set_condition(tx, id, Some(&condition))?;
                    db::add_activity(
                        tx,
                        Some(id),
                        None,
                        "error",
                        &format!("{}: {}", effect_name(effect), error.message),
                        now(),
                    )?;
                }
            }
            Ok(())
        })
        .await;
    if let Err(error) = stored {
        tracing::error!(workspace = %id, %error, "couldn't record a command result");
    }
    succeeded
}

async fn command_for(
    app: &App,
    ws: &WorkspaceRecord,
    principal: &PrincipalRecord,
    effect: Effect,
) -> Result<Command, CommandError> {
    let internal = |e: db::DbError| CommandError::new(ErrorCode::Runtime, e.to_string());
    Ok(match effect {
        Effect::Create => {
            let revision = ws.env_revision;
            let image = app
                .db
                .call(move |tx| db::revision_image(tx, revision))
                .await
                .map_err(internal)?
                .ok_or_else(|| {
                    CommandError::new(
                        ErrorCode::ImageMissing,
                        "the environment revision has no image",
                    )
                })?;
            let defaults = &app.config.workspaces;
            Command::Create(CreateSpec {
                owner: ws.owner,
                image: image.fingerprint,
                user: image.user,
                limits: Limits {
                    cpus: defaults.cpus,
                    memory: defaults.memory,
                    processes: defaults.processes,
                    swap: defaults.swap,
                },
            })
        }
        Effect::Start => Command::Start,
        Effect::DeliverSecrets => {
            let owner = ws.owner;
            let sealed = app
                .db
                .call(move |tx| db::sealed_secrets(tx, owner))
                .await
                .map_err(internal)?;
            let context = owner.to_string();
            let mut opened = Vec::with_capacity(sealed.len());
            for secret in sealed {
                let plaintext = app
                    .sealer
                    .open(&secret.nonce, &secret.ciphertext, &context)
                    .map_err(|e| CommandError::new(ErrorCode::Runtime, e.to_string()))?;
                let value = String::from_utf8(plaintext)
                    .ok()
                    .and_then(|text| SecretValue::try_from(text).ok())
                    .ok_or_else(|| {
                        CommandError::new(ErrorCode::Runtime, "a stored secret is unreadable")
                    })?;
                opened.push((secret.target, value));
            }
            let bundle = bundle(principal.secrets_generation, opened)
                .map_err(|e| CommandError::new(ErrorCode::InvalidState, e.to_string()))?;
            Command::DeliverSecrets(bundle)
        }
        Effect::Provision => Command::Provision(ProvisionSpec {
            repo: ws.repo.clone(),
            branch: ws.branch.clone(),
            base: ws.base.clone(),
        }),
        Effect::Freeze => Command::Freeze,
        Effect::Thaw => Command::Thaw,
        Effect::Stop => Command::Stop,
        Effect::Delete => Command::Delete,
    })
}
