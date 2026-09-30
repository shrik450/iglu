//! Performing lifecycle commands against Incus. Each command is idempotent.

use std::time::Duration;

use hyper::Method;
use iglu_domain::id::{InstanceName, WorkspaceId};
use iglu_domain::lifecycle::Instance;
use iglu_domain::secret::SecretBundle;
use iglu_proto::{Command, CommandError, CommandOutcome, CreateSpec, ErrorCode, ProvisionSpec};
use serde_json::{Value, json};

use crate::exec::{self, Program};
use crate::guestfs::paths;
use crate::incus::{FileOwner, IncusError, StateAction};
use crate::observe::{self, Owned, Ownership, Status, keys};
use crate::server::App;

pub async fn perform(app: &App, workspace: WorkspaceId, command: Command) -> CommandOutcome {
    let name = workspace.instance_name();
    let result = match command {
        Command::Create(spec) => create(app, name, spec).await,
        Command::Start => start(app, name).await,
        Command::DeliverSecrets(bundle) => deliver_secrets(app, name, &bundle).await,
        Command::Provision(spec) => provision(app, name, &spec).await,
        Command::Freeze => freeze(app, name).await,
        Command::Thaw => thaw(app, name).await,
        Command::Stop => stop(app, name).await,
        Command::Delete => delete(app, name).await,
    };
    match result {
        Ok(()) => match observe_one(app, name).await {
            Ok(instance) => CommandOutcome::Done { instance },
            Err(error) => CommandOutcome::Failed(error),
        },
        Err(error) => {
            tracing::warn!(%name, %error, "command failed");
            CommandOutcome::Failed(error)
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "an adapter for map_err, which passes errors by value"
)]
fn runtime(error: IncusError) -> CommandError {
    let code = if error.is_not_found() {
        ErrorCode::NotFound
    } else {
        ErrorCode::Runtime
    };
    CommandError::new(code, error.to_string())
}

/// Looks up an instance that must exist and be iglu's.
async fn owned(app: &App, name: InstanceName) -> Result<Owned, CommandError> {
    match app.incus.instance(name).await.map_err(runtime)? {
        Some(instance) => Owned::parse(instance).map_err(|error| match error {
            Ownership::Foreign => {
                CommandError::new(ErrorCode::Conflict, "the instance isn't iglu's")
            }
            Ownership::Damaged(..) => CommandError::new(ErrorCode::Conflict, error.to_string()),
        }),
        None => Err(CommandError::new(
            ErrorCode::NotFound,
            "no such workspace instance",
        )),
    }
}

pub async fn observe_one(app: &App, name: InstanceName) -> Result<Instance, CommandError> {
    match app.incus.instance(name).await.map_err(runtime)? {
        None => Ok(Instance::Absent),
        Some(instance) => match Owned::parse(instance) {
            Ok(owned) => Ok(observe::instance(&owned, &app.boots).await),
            Err(error) => Err(CommandError::new(ErrorCode::Conflict, error.to_string())),
        },
    }
}

fn mib(bytes: iglu_domain::capacity::Bytes) -> String {
    format!("{}MiB", bytes.get() / (1024 * 1024))
}

async fn create(app: &App, name: InstanceName, spec: CreateSpec) -> Result<(), CommandError> {
    if let Some(existing) = app.incus.instance(name).await.map_err(runtime)? {
        return match Owned::parse(existing) {
            Ok(_) => Ok(()),
            Err(error) => Err(CommandError::new(ErrorCode::Conflict, error.to_string())),
        };
    }
    match app
        .incus
        .get::<Value>(&format!("/1.0/images/{}", spec.image))
        .await
    {
        Ok(_) => {}
        Err(error) if error.is_not_found() => {
            return Err(CommandError::new(
                ErrorCode::ImageMissing,
                "the environment's image isn't on this host",
            ));
        }
        Err(error) => return Err(runtime(error)),
    }
    let guest_user = serde_json::to_string(&spec.user)
        .map_err(|e| CommandError::new(ErrorCode::Runtime, e.to_string()))?;
    let body = json!({
        "name": name.to_string(),
        "type": "container",
        "source": { "type": "image", "fingerprint": spec.image.to_string() },
        "profiles": [app.config.incus.profile],
        "start": false,
        "config": {
            "boot.autostart": "false",
            "security.nesting": "true",
            "security.idmap.isolated": "true",
            "limits.cpu": spec.limits.cpus.to_string(),
            "limits.memory": mib(spec.limits.memory),
            "limits.memory.swap": mib(spec.limits.swap),
            "limits.processes": spec.limits.processes.to_string(),
            keys::WORKSPACE: name.workspace().to_string(),
            keys::OWNER: spec.owner.to_string(),
            keys::GUEST_USER: guest_user,
        },
    });
    app.incus
        .run(
            Method::POST,
            "/1.0/instances",
            Some(&body),
            app.config.timeouts.operation(),
        )
        .await
        .map(|_| ())
        .map_err(runtime)
}

async fn start(app: &App, name: InstanceName) -> Result<(), CommandError> {
    let owned = owned(app, name).await?;
    match current_status(&owned) {
        Status::Running | Status::Frozen | Status::Transitioning => Ok(()),
        Status::Stopped | Status::Failed => app
            .incus
            .change_state(name, StateAction::Start, app.config.timeouts.operation())
            .await
            .map_err(runtime),
    }
}

fn current_status(owned: &Owned) -> Status {
    owned
        .state
        .as_ref()
        .map_or(Status::Transitioning, |s| observe::status(s.status_code))
}

fn running_pid(owned: &Owned) -> Result<i64, CommandError> {
    match (current_status(owned), owned.pid()) {
        (Status::Running, Some(pid)) => Ok(pid),
        (Status::Running, None)
        | (Status::Stopped | Status::Frozen | Status::Transitioning | Status::Failed, _) => Err(
            CommandError::new(ErrorCode::InvalidState, "the workspace isn't running"),
        ),
    }
}

/// Hands the bundle to `iglu-guest install-secrets`, which lays the files out
/// and records the generation last. Both run as the workspace user, in a
/// directory that user owns, so installing secrets grants no extra authority.
async fn deliver_secrets(
    app: &App,
    name: InstanceName,
    bundle: &SecretBundle,
) -> Result<(), CommandError> {
    let owned = owned(app, name).await?;
    let pid = running_pid(&owned)?;
    let payload = serde_json::to_vec(&InstallRequest { bundle })
        .map_err(|e| CommandError::new(ErrorCode::Runtime, e.to_string()))?;
    let user_only = FileOwner {
        uid: owned.user.uid.get(),
        gid: owned.user.gid.get(),
        mode: 0o600,
    };
    app.incus
        .put_file(name, paths::SECRETS_INCOMING, payload, user_only)
        .await
        .map_err(runtime)?;
    let program = Program::new(
        &owned.user,
        [
            paths::GUEST_TOOL,
            "install-secrets",
            paths::SECRETS_INCOMING,
        ],
    );
    let output = exec::capture(&app.incus, name, &program, app.config.timeouts.operation())
        .await
        .map_err(runtime)?;
    if !output.success() {
        return Err(CommandError::new(
            ErrorCode::GuestFailed,
            format!("installing secrets failed: {}", output.stderr_tail()),
        ));
    }
    app.boots.delivered(name, pid, bundle.generation);
    Ok(())
}

#[derive(serde::Serialize)]
struct InstallRequest<'a> {
    bundle: &'a SecretBundle,
}

async fn provision(
    app: &App,
    name: InstanceName,
    spec: &ProvisionSpec,
) -> Result<(), CommandError> {
    let owned = owned(app, name).await?;
    running_pid(&owned)?;
    let mut argv = vec![
        paths::GUEST_TOOL.to_owned(),
        "provision".into(),
        "--repo".into(),
        spec.repo.to_string(),
        "--branch".into(),
        spec.branch.to_string(),
    ];
    if let Some(base) = &spec.base {
        argv.extend(["--base".into(), base.to_string()]);
    }
    let program = Program::new(&owned.user, argv);
    let output = exec::capture(&app.incus, name, &program, app.config.timeouts.provision())
        .await
        .map_err(runtime)?;
    if !output.success() {
        return Err(CommandError::new(
            ErrorCode::GuestFailed,
            format!("cloning the repository failed: {}", output.stderr_tail()),
        ));
    }
    app.incus
        .update_instance(
            name,
            |instance| {
                instance["config"][keys::PROVISIONED] = json!("true");
            },
            app.config.timeouts.operation(),
        )
        .await
        .map_err(runtime)
}

async fn freeze(app: &App, name: InstanceName) -> Result<(), CommandError> {
    let owned = owned(app, name).await?;
    match current_status(&owned) {
        Status::Frozen => return Ok(()),
        Status::Running => {}
        Status::Stopped | Status::Transitioning | Status::Failed => {
            return Err(CommandError::new(
                ErrorCode::InvalidState,
                "only a running workspace can be frozen",
            ));
        }
    }
    app.incus
        .change_state(name, StateAction::Freeze, app.config.timeouts.operation())
        .await
        .map_err(runtime)?;
    crate::reclaim::reclaim(name).await;
    Ok(())
}

async fn thaw(app: &App, name: InstanceName) -> Result<(), CommandError> {
    let owned = owned(app, name).await?;
    match current_status(&owned) {
        Status::Frozen => app
            .incus
            .change_state(name, StateAction::Unfreeze, app.config.timeouts.operation())
            .await
            .map_err(runtime),
        Status::Running => Ok(()),
        Status::Stopped | Status::Transitioning | Status::Failed => Err(CommandError::new(
            ErrorCode::InvalidState,
            "the workspace isn't frozen",
        )),
    }
}

async fn stop(app: &App, name: InstanceName) -> Result<(), CommandError> {
    let owned = owned(app, name).await?;
    stop_owned(app, &owned).await
}

async fn stop_owned(app: &App, owned: &Owned) -> Result<(), CommandError> {
    let name = owned.name;
    let timeouts = &app.config.timeouts;
    match current_status(owned) {
        Status::Stopped => return Ok(()),
        Status::Frozen => {
            // A frozen cgroup can't run its shutdown; thaw it first.
            app.incus
                .change_state(name, StateAction::Unfreeze, timeouts.operation())
                .await
                .map_err(runtime)?;
        }
        Status::Running | Status::Transitioning | Status::Failed => {}
    }
    let grace = StateAction::Stop {
        timeout_secs: timeouts.stop_secs,
    };
    let wait = Duration::from_secs(u64::from(timeouts.stop_secs) + 10);
    if let Err(error) = app.incus.change_state(name, grace, wait).await {
        tracing::info!(%name, %error, "graceful stop failed; forcing");
        app.incus
            .change_state(name, StateAction::ForceStop, timeouts.operation())
            .await
            .map_err(runtime)?;
    }
    app.boots.forget(name);
    Ok(())
}

async fn delete(app: &App, name: InstanceName) -> Result<(), CommandError> {
    let owned = match app.incus.instance(name).await.map_err(runtime)? {
        None => return Ok(()),
        Some(instance) => Owned::parse(instance)
            .map_err(|e| CommandError::new(ErrorCode::Conflict, e.to_string()))?,
    };
    match current_status(&owned) {
        Status::Stopped => {}
        Status::Running | Status::Frozen | Status::Transitioning | Status::Failed => {
            if let Err(error) = stop_owned(app, &owned).await {
                tracing::info!(%name, %error, "stop before delete failed; forcing");
                app.incus
                    .change_state(
                        name,
                        StateAction::ForceStop,
                        app.config.timeouts.operation(),
                    )
                    .await
                    .map_err(runtime)?;
            }
        }
    }
    app.incus
        .run(
            Method::DELETE,
            &format!("/1.0/instances/{name}"),
            None,
            app.config.timeouts.operation(),
        )
        .await
        .map_err(runtime)?;
    crate::tunnel::remove_sockets(app, name).await;
    app.boots.forget(name);
    Ok(())
}
