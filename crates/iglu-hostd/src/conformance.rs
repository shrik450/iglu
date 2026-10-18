//! What every runtime owes hostd that the types can't say, checked through
//! hostd's own commands against a runtime in its real environment: real
//! Incus in the VM test, real processes for the local runtime.
//!
//! Each check works on a workspace of its own and deletes it afterwards.
//! A new runtime is done when it passes every check here, and, if it's to
//! be [`crate::runtime::Isolating`], its own negative isolation tests.

use std::fmt::Write as _;
use std::future::Future;
use std::time::{Duration, Instant};

use bytes::Bytes;
use iglu_domain::attention::{AttentionState, SessionStatus};
use iglu_domain::capacity;
use iglu_domain::column::{Arg, Argv};
use iglu_domain::env::{BuiltImage, EnvSource, ImageFingerprint, Uid};
use iglu_domain::id::{PrincipalId, WorkspaceId};
use iglu_domain::lifecycle::{
    Columns, Instance, Present, Provisioning, Readiness, Running, Runtime as Status,
    SecretsGeneration,
};
use iglu_domain::port::GuestPort;
use iglu_domain::repo::Checkout;
use iglu_domain::secret::{FetchTokens, SecretTarget, SecretValue, bundle};
use iglu_domain::terminal::{OutputLines, SessionName, TerminalSize};
use iglu_domain::time::Millis;
use iglu_proto::{
    BuildOutcome, Command, CommandOutcome, CreateSpec, ErrorCode, InstanceReport, Limits,
    ProvisionSpec, PruneImages, SessionSpec, UpgradeSpec,
};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::guest::parse_generation;
use crate::host::Host;
use crate::runtime::{Guest, GuestFile, Runtime, Terminal, TerminalInput};

/// What the suite needs from the environment it runs in.
pub struct Fixture {
    /// An environment the runtime can build here.
    pub source: EnvSource,
    /// Another, with the same account, that workspaces update to.
    pub next: EnvSource,
    /// How long a started workspace may take to finish booting.
    pub boot_timeout: Duration,
    pub guests: Guests,
}

/// Whether a runtime's guests run programs. A simulated runtime's answer
/// the guest tools' commands without running anything, so it's held to the
/// checks that need no program in a guest: the lifecycle, secrets,
/// provisioning and images. The rest need shells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guests {
    Programs,
    Modelled,
}

/// One check's result.
#[derive(Debug)]
pub struct Outcome {
    pub check: &'static str,
    pub result: Result<(), String>,
}

/// Runs every check, in order, and reports each.
#[expect(
    clippy::too_many_lines,
    reason = "a flat list of the checks, each a few lines once formatted"
)]
pub async fn run<R: Runtime>(host: &Host<R>, fixture: &Fixture) -> Vec<Outcome> {
    let suite = match Suite::new(host, fixture).await {
        Ok(suite) => suite,
        Err(failed) => return vec![failed],
    };
    let mut outcomes = vec![Outcome {
        check: "builds the environment",
        result: Ok(()),
    }];
    outcomes.push(
        suite
            .check(
                "creating is idempotent and lists the instance with its owner",
                Suite::creates,
            )
            .await,
    );
    outcomes.push(
        suite
            .check("creating needs the image", Suite::needs_the_image)
            .await,
    );
    outcomes.push(
        suite
            .check("commands need the instance", Suite::needs_the_instance)
            .await,
    );
    outcomes.push(
        suite
            .check("the lifecycle follows the commands", Suite::lifecycle)
            .await,
    );
    outcomes.push(
        suite
            .check("secrets last one boot", Suite::secrets_last_one_boot)
            .await,
    );
    outcomes.push(
        suite
            .check(
                "an older delivery of secrets leaves the newer ones, and counts as delivered",
                Suite::older_secrets_are_superseded,
            )
            .await,
    );
    outcomes.push(
        suite
            .check(
                "provisioning outlives a restart",
                Suite::provisioning_outlives_a_restart,
            )
            .await,
    );
    outcomes.extend(
        suite
            .with_programs("terminals run in the workspace", Suite::terminals)
            .await,
    );
    outcomes.extend(
        suite
            .with_programs(
                "terminals take typing and show what they print, without a client",
                Suite::typing,
            )
            .await,
    );
    outcomes.extend(
        suite
            .with_programs(
                "a resize reaches the program, at any size a terminal may have",
                Suite::resizes,
            )
            .await,
    );
    outcomes.extend(
        suite
            .with_programs(
                "output arrives whole and in order to a slow client, and typing reaches the terminal after",
                Suite::floods,
            )
            .await,
    );
    outcomes.extend(
        suite
            .with_programs(
                "sessions opened and attached at once work apart, and every client of one sees it",
                Suite::at_once,
            )
            .await,
    );
    outcomes.extend(
        suite
            .with_programs(
                "columns open once a boot, with their arguments exactly as given",
                Suite::opens_columns,
            )
            .await,
    );
    outcomes.extend(
        suite
            .with_programs("stopping a frozen workspace ends it", Suite::stops_frozen)
            .await,
    );
    outcomes.extend(
        suite
            .with_programs(
                "tunnels reach the guest's ports, which show as listening",
                Suite::tunnels,
            )
            .await,
    );
    outcomes.extend(
        suite
            .with_programs(
                "threads reach the inventory, go with their session and outlast a freeze",
                Suite::attention,
            )
            .await,
    );
    outcomes.extend(
        suite
            .with_programs(
                "caffeinate keeps a terminal working while its command runs",
                Suite::caffeinate,
            )
            .await,
    );
    outcomes.extend(
        suite
            .with_programs("a large bundle arrives whole", Suite::large_bundle)
            .await,
    );
    outcomes.extend(
        suite
            .with_programs(
                "agents keep their configuration for every workspace of the person's, and no one else's",
                Suite::keeps_per_person,
            )
            .await,
    );
    outcomes.push(
        suite
            .check(
                "a failed clone leaves provisioning pending",
                Suite::failed_clone,
            )
            .await,
    );
    outcomes.extend(
        suite
            .with_programs(
                "updating upgrades it in place to the next image from any settled state, keeping its files",
                |suite, workspace| suite.updates(workspace, &fixture.next),
            )
            .await,
    );
    // Last: it removes the image the other checks use.
    outcomes.push(
        suite
            .check(
                "an image goes once nothing keeps it and it isn't new",
                Suite::prunes_images,
            )
            .await,
    );
    outcomes
}

/// Runs a runtime's own checks, such as its isolation tests, inside a
/// running workspace made for them, and deletes it afterwards.
pub async fn inside<R, F, Fut>(host: &Host<R>, fixture: &Fixture, checks: F) -> Vec<Outcome>
where
    R: Runtime,
    F: FnOnce(Guest) -> Fut,
    Fut: Future<Output = Vec<Outcome>>,
{
    let suite = match Suite::new(host, fixture).await {
        Ok(suite) => suite,
        Err(failed) => return vec![failed],
    };
    let workspace = WorkspaceId::from_uuid(Uuid::new_v4());
    let guest = async {
        suite
            .perform(workspace, Command::Create(suite.spec()))
            .await?;
        suite.boot(workspace).await?;
        suite.guest(workspace).await
    }
    .await;
    let mut outcomes = match guest {
        Ok(guest) => checks(guest).await,
        Err(why) => vec![Outcome {
            check: "starts a workspace to check inside",
            result: Err(why),
        }],
    };
    if let Err(why) = suite.perform(workspace, Command::Delete).await {
        outcomes.push(Outcome {
            check: "deletes the workspace it checked inside",
            result: Err(why),
        });
    }
    outcomes
}

type Checked = Result<(), String>;

struct Suite<'a, R> {
    host: &'a Host<R>,
    image: BuiltImage,
    boot_timeout: Duration,
    guests: Guests,
    /// Who owns the workspaces the checks make, one person for the run, so
    /// the run makes one kept directory.
    owner: PrincipalId,
}

fn ensure(condition: bool, what: impl FnOnce() -> String) -> Checked {
    if condition { Ok(()) } else { Err(what()) }
}

/// Types a line into an attached terminal.
async fn type_line(terminal: &Terminal, line: &str) -> Checked {
    terminal
        .input
        .send(TerminalInput::Bytes(Bytes::from(format!("{line}\r"))))
        .await
        .map_err(|_| format!("the terminal closed before {line:?}"))
}

/// Reads a terminal until `expect` shows, and returns all it read; on
/// giving up, returns that in the error.
async fn shown(terminal: &mut Terminal, expect: &str, within: Duration) -> Result<String, String> {
    let mut seen = Vec::new();
    let deadline = Instant::now() + within;
    while !String::from_utf8_lossy(&seen).contains(expect) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, terminal.output.recv()).await {
            Ok(Some(bytes)) => seen.extend_from_slice(&bytes),
            Ok(None) | Err(_) => return Err(String::from_utf8_lossy(&seen).into_owned()),
        }
    }
    Ok(String::from_utf8_lossy(&seen).into_owned())
}

fn running(instance: Instance) -> Option<Running> {
    match instance {
        Instance::Present(Present {
            runtime: Status::Running(running),
            ..
        }) => Some(running),
        Instance::Absent | Instance::Present(_) => None,
    }
}

fn status(instance: Instance) -> Option<Status> {
    match instance {
        Instance::Present(present) => Some(present.runtime),
        Instance::Absent => None,
    }
}

impl<'a, R: Runtime> Suite<'a, R> {
    async fn new(host: &'a Host<R>, fixture: &Fixture) -> Result<Self, Outcome> {
        match host.build(&fixture.source, &FetchTokens::default()).await {
            BuildOutcome::Built(image) => Ok(Self {
                host,
                image,
                boot_timeout: fixture.boot_timeout,
                guests: fixture.guests,
                owner: PrincipalId::from_uuid(Uuid::new_v4()),
            }),
            BuildOutcome::Failed { log_tail } => Err(Outcome {
                check: "builds the environment",
                result: Err(log_tail),
            }),
        }
    }

    /// Runs one check on a workspace of its own, and deletes it afterwards
    /// whatever happened.
    async fn check<'s, F, Fut>(&'s self, name: &'static str, check: F) -> Outcome
    where
        F: FnOnce(&'s Self, WorkspaceId) -> Fut,
        Fut: Future<Output = Checked>,
    {
        let workspace = WorkspaceId::from_uuid(Uuid::new_v4());
        let result = check(self, workspace).await;
        let cleaned = self.perform(workspace, Command::Delete).await;
        Outcome {
            check: name,
            result: result.and(cleaned.map(|_| ())),
        }
    }

    /// Runs a check that needs programs in the guest, unless the runtime's
    /// guests run none.
    async fn with_programs<'s, F, Fut>(&'s self, name: &'static str, check: F) -> Option<Outcome>
    where
        F: FnOnce(&'s Self, WorkspaceId) -> Fut,
        Fut: Future<Output = Checked>,
    {
        match self.guests {
            Guests::Programs => Some(self.check(name, check).await),
            Guests::Modelled => None,
        }
    }

    fn spec(&self) -> CreateSpec {
        self.spec_for(self.owner)
    }

    fn spec_for(&self, owner: PrincipalId) -> CreateSpec {
        CreateSpec {
            owner,
            image: self.image.fingerprint.clone(),
            user: self.image.user.clone(),
            limits: Limits {
                cpus: 1,
                memory: capacity::Bytes::new(1 << 30),
                processes: 4096,
                swap: capacity::Bytes::new(1 << 30),
            },
        }
    }

    async fn perform(&self, workspace: WorkspaceId, command: Command) -> Result<Instance, String> {
        let what = format!("{command:?}");
        match self.host.perform(workspace, command).await {
            CommandOutcome::Done { instance } => Ok(instance),
            CommandOutcome::Failed(error) => Err(format!("{what} failed: {error}")),
        }
    }

    async fn refused(&self, workspace: WorkspaceId, command: Command, code: ErrorCode) -> Checked {
        let what = format!("{command:?}");
        match self.host.perform(workspace, command).await {
            CommandOutcome::Failed(error) if error.code == code => Ok(()),
            outcome @ (CommandOutcome::Done { .. } | CommandOutcome::Failed(_)) => {
                Err(format!("{what}: expected {code:?}, got {outcome:?}"))
            }
        }
    }

    async fn expect_status(
        &self,
        workspace: WorkspaceId,
        command: Command,
        expected: fn(Status) -> bool,
    ) -> Checked {
        let what = format!("{command:?}");
        let instance = self.perform(workspace, command).await?;
        ensure(status(instance).is_some_and(expected), || {
            format!("after {what}: {instance:?}")
        })
    }

    /// Starts the workspace and waits until it has finished booting.
    async fn boot(&self, workspace: WorkspaceId) -> Result<Running, String> {
        self.perform(workspace, Command::Start).await?;
        let deadline = Instant::now() + self.boot_timeout;
        loop {
            let instance = self
                .host
                .observe(workspace.instance_name())
                .await
                .map_err(|e| e.to_string())?;
            if let Some(running) = running(instance)
                && running.readiness >= Readiness::System
            {
                return Ok(running);
            }
            if Instant::now() > deadline {
                return Err(format!("not booted in time: {instance:?}"));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn guest(&self, workspace: WorkspaceId) -> Result<Guest, String> {
        self.host
            .guest(workspace.instance_name())
            .await
            .map_err(|e| e.to_string())
    }

    async fn creates(&self, workspace: WorkspaceId) -> Checked {
        let spec = self.spec();
        for _ in 0..2 {
            let instance = self
                .perform(workspace, Command::Create(spec.clone()))
                .await?;
            ensure(
                instance
                    == Instance::Present(Present {
                        runtime: Status::Stopped,
                        provisioning: Provisioning::Pending,
                    }),
                || format!("created: {instance:?}"),
            )?;
        }
        let inventory = self.host.inventory().await.map_err(|e| e.to_string())?;
        ensure(
            inventory
                .iter()
                .any(|report| report.workspace == workspace && report.owner == spec.owner),
            || "the inventory doesn't list it with its owner".into(),
        )?;
        for _ in 0..2 {
            let instance = self.perform(workspace, Command::Delete).await?;
            ensure(instance == Instance::Absent, || {
                format!("deleted: {instance:?}")
            })?;
        }
        Ok(())
    }

    async fn needs_the_image(&self, workspace: WorkspaceId) -> Checked {
        let mut spec = self.spec();
        spec.image = "0"
            .repeat(64)
            .parse()
            .expect("64 hex digits are a fingerprint");
        self.refused(workspace, Command::Create(spec), ErrorCode::ImageMissing)
            .await?;
        let instance = self.perform(workspace, Command::Delete).await?;
        ensure(instance == Instance::Absent, || {
            format!("a refused create left {instance:?}")
        })
    }

    async fn needs_the_instance(&self, workspace: WorkspaceId) -> Checked {
        for command in [
            Command::Start,
            Command::Freeze,
            Command::Thaw,
            Command::Stop,
        ] {
            self.refused(workspace, command, ErrorCode::NotFound)
                .await?;
        }
        Ok(())
    }

    async fn lifecycle(&self, workspace: WorkspaceId) -> Checked {
        self.perform(workspace, Command::Create(self.spec()))
            .await?;
        self.refused(workspace, Command::Freeze, ErrorCode::InvalidState)
            .await?;
        self.refused(workspace, Command::Thaw, ErrorCode::InvalidState)
            .await?;
        self.boot(workspace).await?;
        let is_running = |s| matches!(s, Status::Running(_));
        let is_frozen = |s| matches!(s, Status::Frozen);
        let is_stopped = |s| matches!(s, Status::Stopped);
        self.expect_status(workspace, Command::Start, is_running)
            .await?;
        for _ in 0..2 {
            self.expect_status(workspace, Command::Freeze, is_frozen)
                .await?;
        }
        let terminals = self.host.terminals(workspace).await;
        ensure(
            terminals
                .as_ref()
                .is_err_and(|e| e.code == ErrorCode::InvalidState),
            || format!("a frozen workspace listed its terminals: {terminals:?}"),
        )?;
        for _ in 0..2 {
            self.expect_status(workspace, Command::Thaw, is_running)
                .await?;
        }
        self.expect_status(workspace, Command::Freeze, is_frozen)
            .await?;
        // A runtime stops a frozen instance as it is.
        for _ in 0..2 {
            self.expect_status(workspace, Command::Stop, is_stopped)
                .await?;
        }
        self.boot(workspace).await?;
        self.expect_status(workspace, Command::Freeze, is_frozen)
            .await?;
        let instance = self.perform(workspace, Command::Delete).await?;
        ensure(instance == Instance::Absent, || {
            format!("deleting a frozen workspace left {instance:?}")
        })
    }

    async fn delivered(&self, workspace: WorkspaceId) -> Result<Option<SecretsGeneration>, String> {
        let guest = self.guest(workspace).await?;
        let bytes = self
            .host
            .runtime()
            .read(&guest, GuestFile::SecretsGeneration)
            .await
            .map_err(|e| e.to_string())?;
        Ok(bytes.as_deref().and_then(parse_generation))
    }

    async fn secrets_last_one_boot(&self, workspace: WorkspaceId) -> Checked {
        self.perform(workspace, Command::Create(self.spec()))
            .await?;
        let first = self.boot(workspace).await?;
        ensure(first.secrets.is_none(), || {
            format!("a new boot came with secrets: {first:?}")
        })?;
        let generation = SecretsGeneration::from_u64(3);
        let secrets = bundle(generation, []).map_err(|e| e.to_string())?;
        let delivered = self
            .perform(workspace, Command::DeliverSecrets(secrets))
            .await?;
        ensure(
            running(delivered).and_then(|r| r.secrets) == Some(generation),
            || format!("after delivering: {delivered:?}"),
        )?;
        let held = self.delivered(workspace).await?;
        ensure(held == Some(generation), || {
            format!("the guest holds {held:?}")
        })?;
        self.perform(workspace, Command::Stop).await?;
        let second = self.boot(workspace).await?;
        let held = self.delivered(workspace).await?;
        ensure(second.secrets.is_none() && held.is_none(), || {
            format!("secrets outlived a restart: {second:?}, the guest holds {held:?}")
        })
    }

    /// A delivery older than the one the guest holds, landing late, leaves
    /// the newer secrets in place and counts as delivered: no error, and so
    /// nothing to retry.
    async fn older_secrets_are_superseded(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        let newer = SecretsGeneration::from_u64(5);
        let older = SecretsGeneration::from_u64(3);
        for generation in [newer, older] {
            let secrets = bundle(generation, []).map_err(|e| e.to_string())?;
            let instance = self
                .perform(workspace, Command::DeliverSecrets(secrets))
                .await?;
            ensure(
                running(instance).and_then(|r| r.secrets) == Some(newer),
                || format!("after delivering {generation:?}: {instance:?}"),
            )?;
        }
        let held = self.delivered(workspace).await?;
        ensure(held == Some(newer), || format!("the guest holds {held:?}"))
    }

    async fn provisioning_outlives_a_restart(&self, workspace: WorkspaceId) -> Checked {
        self.perform(workspace, Command::Create(self.spec()))
            .await?;
        self.host
            .runtime()
            .mark_provisioned(workspace.instance_name())
            .await
            .map_err(|e| e.to_string())?;
        self.boot(workspace).await?;
        let instance = self.perform(workspace, Command::Stop).await?;
        ensure(
            instance
                == Instance::Present(Present {
                    runtime: Status::Stopped,
                    provisioning: Provisioning::Complete,
                }),
            || format!("after a restart: {instance:?}"),
        )
    }

    /// Types `line` into a session, creating it if needed, and waits for
    /// `expect` in what the terminal shows. Detaches afterwards; the session
    /// and whatever it runs carry on. Plain enough for any shell.
    async fn shell(
        &self,
        workspace: WorkspaceId,
        session: &str,
        line: &str,
        expect: &str,
    ) -> Result<(), String> {
        let session = self.open_shell(workspace, session).await?;
        let mut terminal = self
            .attach(workspace, &session, TerminalSize::DEFAULT)
            .await?;
        type_line(&terminal, line).await?;
        shown(&mut terminal, expect, Duration::from_secs(30))
            .await
            .map_err(|seen| format!("{line:?} never showed {expect:?}: {seen:?}"))?;
        // Let the shell take the line before detaching.
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(())
    }

    /// Opens a session running the user's shell.
    async fn open_shell(&self, workspace: WorkspaceId, name: &str) -> Result<SessionName, String> {
        let session: SessionName = name.parse().map_err(|e| format!("{e}"))?;
        self.host
            .open_terminal(
                workspace,
                &SessionSpec {
                    name: session.clone(),
                    command: None,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(session)
    }

    async fn attach(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
        size: TerminalSize,
    ) -> Result<Terminal, String> {
        self.host
            .attach(workspace, session, size)
            .await
            .map_err(|e| e.to_string())
    }

    async fn running_workspace(&self, workspace: WorkspaceId) -> Checked {
        self.perform(workspace, Command::Create(self.spec()))
            .await?;
        self.boot(workspace).await.map(|_| ())
    }

    async fn prunes_images(&self, workspace: WorkspaceId) -> Checked {
        let image = self.image.fingerprint.clone();
        let prune = |keep: Vec<ImageFingerprint>, older_than: Millis| {
            let request = PruneImages { keep, older_than };
            async move {
                self.host
                    .prune_images(&request)
                    .await
                    .map(|pruned| pruned.removed)
                    .map_err(|e| e.to_string())
            }
        };
        let removed = prune(Vec::new(), Millis::from_secs(3600)).await?;
        ensure(!removed.contains(&image), || {
            format!("a new image was removed: {removed:?}")
        })?;
        let removed = prune(vec![image.clone()], Millis::from_secs(0)).await?;
        ensure(!removed.contains(&image), || {
            format!("a kept image was removed: {removed:?}")
        })?;
        let removed = prune(Vec::new(), Millis::from_secs(0)).await?;
        ensure(removed.contains(&image), || {
            format!("the image wasn't removed: {removed:?}")
        })?;
        self.refused(
            workspace,
            Command::Create(self.spec()),
            ErrorCode::ImageMissing,
        )
        .await
    }

    async fn image_of(&self, workspace: WorkspaceId) -> Result<Option<ImageFingerprint>, String> {
        let reports = self.host.inventory().await.map_err(|e| e.to_string())?;
        Ok(reports
            .into_iter()
            .find(|r| r.workspace == workspace)
            .and_then(|r| r.image))
    }

    async fn updates(&self, workspace: WorkspaceId, next: &EnvSource) -> Checked {
        let next = match self.host.build(next, &FetchTokens::default()).await {
            BuildOutcome::Built(image) => image,
            BuildOutcome::Failed { log_tail } => {
                return Err(format!("building the next environment: {log_tail}"));
            }
        };
        ensure(next.fingerprint != self.image.fingerprint, || {
            "the next environment is the same image".into()
        })?;
        self.running_workspace(workspace).await?;
        self.shell(
            workspace,
            "home",
            "echo here > ~/update-mark && echo wr''ote",
            "wrote",
        )
        .await?;
        let first = self.image_of(workspace).await?;
        ensure(
            first
                .as_ref()
                .is_none_or(|first| *first == self.image.fingerprint),
            || format!("created from {first:?}, not {}", self.image.fingerprint),
        )?;
        // Another account would leave its files someone else's.
        let mut stranger = next.user.clone();
        stranger.uid = Uid::try_from(stranger.uid.get() + 1).map_err(|e| e.to_string())?;
        self.refused(
            workspace,
            Command::Upgrade(UpgradeSpec {
                image: next.fingerprint.clone(),
                user: stranger,
            }),
            ErrorCode::InvalidState,
        )
        .await?;
        // Stopped, it's started to be upgraded and restarted into the new
        // system; asked again, it's done already, and still running.
        self.perform(workspace, Command::Stop).await?;
        let upgrade = Command::Upgrade(UpgradeSpec {
            image: next.fingerprint.clone(),
            user: next.user.clone(),
        });
        for _ in 0..2 {
            let instance = self.perform(workspace, upgrade.clone()).await?;
            ensure(matches!(status(instance), Some(Status::Running(_))), || {
                format!("not running after upgrading: {instance:?}")
            })?;
        }
        let now = self.image_of(workspace).await?;
        ensure(now.as_ref() == Some(&next.fingerprint), || {
            format!("it runs {now:?}, not {}", next.fingerprint)
        })?;
        self.boot(workspace).await?;
        self.shell(workspace, "home", "cat ~/update-mark", "here")
            .await
    }

    /// What one of a person's workspaces keeps, their others see, live;
    /// another person's don't. The words the checks wait for are split in
    /// what's typed, so only the output has them whole.
    async fn keeps_per_person(&self, workspace: WorkspaceId) -> Checked {
        let same = WorkspaceId::from_uuid(Uuid::new_v4());
        let other = WorkspaceId::from_uuid(Uuid::new_v4());
        let result = async {
            self.running_workspace(workspace).await?;
            self.shell(
                workspace,
                "keeps",
                r#"printf 'kept-%s\n' yes > "$CLAUDE_CONFIG_DIR/mark" && test -d "$CODEX_HOME" && echo wr''ote"#,
                "wrote",
            )
            .await?;
            self.perform(same, Command::Create(self.spec())).await?;
            self.boot(same).await?;
            self.shell(same, "keeps", r#"cat "$CLAUDE_CONFIG_DIR/mark""#, "kept-yes")
                .await?;
            let stranger = PrincipalId::from_uuid(Uuid::new_v4());
            self.perform(other, Command::Create(self.spec_for(stranger)))
                .await?;
            self.boot(other).await?;
            self.shell(
                other,
                "keeps",
                r#"test -e "$CLAUDE_CONFIG_DIR/mark" || echo mi''ssing"#,
                "missing",
            )
            .await
        }
        .await;
        let cleaned = [
            self.perform(same, Command::Delete).await,
            self.perform(other, Command::Delete).await,
        ];
        cleaned
            .into_iter()
            .try_for_each(|done| done.map(|_| ()))
            .and(result)
    }

    async fn deliver(
        &self,
        workspace: WorkspaceId,
        secrets: impl IntoIterator<Item = (SecretTarget, SecretValue)>,
    ) -> Checked {
        let generation = SecretsGeneration::from_u64(1);
        let secrets = bundle(generation, secrets).map_err(|e| e.to_string())?;
        let delivered = self
            .perform(workspace, Command::DeliverSecrets(secrets))
            .await?;
        ensure(
            running(delivered).and_then(|r| r.secrets) == Some(generation),
            || format!("after delivering: {delivered:?}"),
        )
    }

    async fn sessions(&self, workspace: WorkspaceId) -> Result<Vec<SessionName>, String> {
        let listed = self
            .host
            .terminals(workspace)
            .await
            .map_err(|e| e.to_string())?;
        Ok(listed.into_iter().map(|t| t.name).collect())
    }

    async fn terminals(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        let token = (
            SecretTarget::Env {
                name: "CONFORMANCE_TOKEN".parse().expect("a valid variable name"),
            },
            SecretValue::try_from("hunter2".to_owned()).expect("a valid secret"),
        );
        self.deliver(workspace, [token]).await?;
        self.shell(
            workspace,
            "conformance",
            "echo token=$CONFORMANCE_TOKEN",
            "token=hunter2",
        )
        .await?;
        let session: SessionName = "conformance".parse().expect("a valid session name");
        let listed = self.sessions(workspace).await?;
        ensure(listed.contains(&session), || {
            format!("the session didn't outlive detaching: {listed:?}")
        })?;
        self.host
            .close_terminal(workspace, &session)
            .await
            .map_err(|e| e.to_string())?;
        let listed = self.sessions(workspace).await?;
        ensure(!listed.contains(&session), || {
            format!("the closed session is still listed: {listed:?}")
        })?;
        // A client on its way in as the column closed finds nothing, and
        // opens nothing: the session stays closed.
        if let Ok(mut terminal) = self
            .host
            .attach(workspace, &session, TerminalSize::DEFAULT)
            .await
        {
            while let Ok(Some(_)) =
                tokio::time::timeout(Duration::from_secs(10), terminal.output.recv()).await
            {
            }
        }
        let listed = self.sessions(workspace).await?;
        ensure(!listed.contains(&session), || {
            format!("attaching opened the closed session again: {listed:?}")
        })
    }

    async fn typing(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        let session: SessionName = "typed".parse().expect("a valid session name");
        self.host
            .open_terminal(
                workspace,
                &SessionSpec {
                    name: session.clone(),
                    command: None,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        // What's typed arrives exactly, a final newline included; the
        // command as echoed can't be mistaken for what it prints.
        let line: iglu_domain::terminal::TerminalInput =
            "printf 'a%sb\\n' typed\r\n".parse().expect("valid input");
        self.host
            .input(workspace, &session, &line)
            .await
            .map_err(|e| e.to_string())?;
        let lines = OutputLines::try_from(50).expect("valid lines");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = self
                .host
                .output(workspace, &session, lines)
                .await
                .map_err(|e| e.to_string())?;
            if output.text.lines().any(|l| l == "atypedb") {
                break;
            }
            ensure(Instant::now() < deadline, || {
                format!("the typed command never printed: {:?}", output.text)
            })?;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        self.host
            .close_terminal(workspace, &session)
            .await
            .map_err(|e| e.to_string())?;
        // A closed session can't be read or typed into, and typing doesn't
        // open it again.
        ensure(
            self.host.output(workspace, &session, lines).await.is_err(),
            || "a closed session could still be read".to_owned(),
        )?;
        ensure(
            self.host.input(workspace, &session, &line).await.is_err(),
            || "a closed session could still be typed into".to_owned(),
        )?;
        let listed = self.sessions(workspace).await?;
        ensure(!listed.contains(&session), || {
            format!("typing opened the closed session again: {listed:?}")
        })
    }

    /// Waits for `expect` in what a session's terminal shows, without typing.
    async fn screen(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
        expect: &str,
    ) -> Result<String, String> {
        let mut terminal = self
            .attach(workspace, session, TerminalSize::DEFAULT)
            .await?;
        shown(&mut terminal, expect, Duration::from_secs(30))
            .await
            .map_err(|seen| format!("{session} never showed {expect:?}: {seen:?}"))
    }

    /// A resize reaches the program in the session, at the smallest and
    /// largest sizes a terminal may have. Typing and resizes can travel
    /// apart, so the program is asked until it sees the size.
    async fn resizes(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        let session = self.open_shell(workspace, "resized").await?;
        let mut terminal = self
            .attach(workspace, &session, TerminalSize::DEFAULT)
            .await?;
        let sizes = [(80, 24), (2, 1), (1000, 1000), (132, 43)];
        for (index, (cols, rows)) in sizes.into_iter().enumerate() {
            let size = TerminalSize::new(cols, rows).expect("a size a terminal may have");
            // The first is the size attached at.
            if index > 0 {
                terminal
                    .input
                    .send(TerminalInput::Resize(size))
                    .await
                    .map_err(|_| format!("the terminal closed before resizing to {cols}x{rows}"))?;
            }
            let expect = format!("size-{rows}x{cols}");
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                type_line(&terminal, r#"printf 'size-%s\n' "$(stty size | tr ' ' x)""#).await?;
                match shown(&mut terminal, &expect, Duration::from_secs(2)).await {
                    Ok(_) => break,
                    Err(seen) if Instant::now() >= deadline => {
                        return Err(format!("the program never saw {cols}x{rows}: {seen:?}"));
                    }
                    Err(_) => {}
                }
            }
        }
        Ok(())
    }

    /// Output arrives whole and in order however much a program prints,
    /// though the client reads slowly at first, and typing reaches the
    /// terminal after: one long copy once wedged a terminal until a reload.
    async fn floods(&self, workspace: WorkspaceId) -> Checked {
        const LINES: u32 = 300_000;
        const LONG: usize = 300_000;
        self.running_workspace(workspace).await?;
        let session = self.open_shell(workspace, "flood").await?;
        let mut terminal = self
            .attach(workspace, &session, TerminalSize::DEFAULT)
            .await?;
        type_line(
            &terminal,
            &format!(
                "seq 1 {LINES}; head -c {LONG} /dev/zero | tr '\\0' A; printf '\\nflood-%s\\n' done"
            ),
        )
        .await?;
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(180);
        // Slow enough that the program has to wait for the client.
        let slow_until = Instant::now() + Duration::from_secs(2);
        // Only the prompt follows the mark, so it's in the last few KiB.
        while !String::from_utf8_lossy(&seen[seen.len().saturating_sub(4096)..])
            .contains("flood-done")
        {
            if Instant::now() < slow_until {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, terminal.output.recv()).await {
                Ok(Some(bytes)) => seen.extend_from_slice(&bytes),
                Ok(None) | Err(_) => {
                    return Err(format!(
                        "the output stopped after {} bytes: {:?}",
                        seen.len(),
                        String::from_utf8_lossy(&seen[seen.len().saturating_sub(300)..])
                    ));
                }
            }
        }
        // Terminals turn each newline into CR LF.
        let text = String::from_utf8_lossy(&seen).replace('\r', "");
        let lines = (1..=LINES).fold(String::new(), |mut lines, n| {
            let _ = writeln!(lines, "{n}");
            lines
        });
        let long = "A".repeat(LONG);
        // Shells write their own modes just before a program's output, so
        // it's found whole, from its first line to the mark.
        ensure(text.contains(&format!("{lines}{long}\nflood-done")), || {
            format!(
                "{} bytes arrived, but not as printed: every line in order {}, the long line whole {}",
                seen.len(),
                text.contains(&lines),
                text.contains(&format!("{long}\nflood-done")),
            )
        })?;
        type_line(&terminal, "printf 'after-%s\\n' flood").await?;
        shown(&mut terminal, "after-flood", Duration::from_secs(30))
            .await
            .map(|_| ())
            .map_err(|seen| format!("typing after the flood never showed: {seen:?}"))
    }

    /// Sessions opened and attached at once each work, apart from each
    /// other, and every client of a session sees what it prints, as a
    /// second browser tab does.
    async fn at_once(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        let (left, right) = tokio::join!(
            self.open_shell(workspace, "left"),
            self.open_shell(workspace, "right")
        );
        let (left, right) = (left?, right?);
        let (one, other, again) = tokio::join!(
            self.attach(workspace, &left, TerminalSize::DEFAULT),
            self.attach(workspace, &right, TerminalSize::DEFAULT),
            self.attach(workspace, &left, TerminalSize::DEFAULT),
        );
        let (mut one, mut other, mut again) = (one?, other?, again?);
        type_line(&one, "printf 'from-%s\\n' left").await?;
        type_line(&other, "printf 'from-%s\\n' right").await?;
        let wait = Duration::from_secs(30);
        let (one, other, again) = tokio::join!(
            shown(&mut one, "from-left", wait),
            shown(&mut other, "from-right", wait),
            shown(&mut again, "from-left", wait),
        );
        let one = one.map_err(|seen| format!("left never showed its output: {seen:?}"))?;
        let other = other.map_err(|seen| format!("right never showed its output: {seen:?}"))?;
        again.map_err(|seen| format!("left's second client never saw its output: {seen:?}"))?;
        ensure(
            !one.contains("from-right") && !other.contains("from-left"),
            || format!("the sessions' output crossed: left {one:?}, right {other:?}"),
        )
    }

    async fn opens_columns(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        // Everything a shell would mangle, so a shell anywhere on the way shows.
        let tricky = [
            "a b",
            "$HOME",
            "x;y",
            "it's \"quoted\"",
            "back\\slash",
            "line one\nline two",
            "✓ ünïcode",
        ];
        let program = [
            "/bin/sh",
            "-c",
            r#"printf 'SUM:'; printf '%s\0' "$@" | cksum; sleep 600"#,
            "sh",
        ];
        let argv = |args: &[&str]| -> Argv {
            Argv::try_from(
                args.iter()
                    .map(|a| a.parse::<Arg>().expect("a valid argument"))
                    .collect::<Vec<_>>(),
            )
            .expect("a valid command")
        };
        let name = |n: &str| -> SessionName { n.parse().expect("a valid session name") };
        let columns = vec![
            SessionSpec {
                name: name("argv"),
                command: Some(argv(&[program.as_slice(), tricky.as_slice()].concat())),
            },
            SessionSpec {
                name: name("quits"),
                command: Some(argv(&["/bin/sh", "-c", "exit 3"])),
            },
            SessionSpec {
                name: name("shell"),
                command: None,
            },
        ];
        let opened = self
            .perform(
                workspace,
                Command::OpenColumns {
                    sessions: columns.clone(),
                },
            )
            .await?;
        ensure(
            running(opened).map(|r| r.columns) == Some(Columns::Opened),
            || format!("after opening the columns: {opened:?}"),
        )?;
        // Opening again leaves the open ones alone.
        self.perform(workspace, Command::OpenColumns { sessions: columns })
            .await?;
        let listed = self.sessions(workspace).await?;
        ensure(
            listed.iter().filter(|s| s.as_str() == "argv").count() == 1
                && listed.iter().any(|s| s.as_str() == "shell"),
            || format!("after opening twice: {listed:?}"),
        )?;
        let joined: Vec<u8> = tricky
            .iter()
            .flat_map(|a| a.bytes().chain(std::iter::once(0)))
            .collect();
        let expected = format!("SUM:{} {}", posix_cksum(&joined), joined.len());
        self.screen(workspace, &name("argv"), &expected).await?;
        // A new boot opens them again.
        self.perform(workspace, Command::Stop).await?;
        let rebooted = self.boot(workspace).await?;
        ensure(rebooted.columns == Columns::Pending, || {
            format!("a new boot already counted as opened: {rebooted:?}")
        })
    }

    async fn stops_frozen(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        self.shell(workspace, "sleeper", "echo started-$((6*7))", "started-42")
            .await?;
        self.expect_status(workspace, Command::Freeze, |s| matches!(s, Status::Frozen))
            .await?;
        self.expect_status(workspace, Command::Stop, |s| matches!(s, Status::Stopped))
            .await?;
        self.boot(workspace).await?;
        let listed = self.sessions(workspace).await?;
        ensure(listed.is_empty(), || {
            format!("sessions outlived stopping a frozen workspace: {listed:?}")
        })
    }

    async fn tunnels(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        // Local workspaces share this machine's ports, so pick one unlikely to be taken.
        let port = 20_000 + Uuid::new_v4().as_u128() % 20_000;
        let port = u16::try_from(port)
            .ok()
            .and_then(|p| GuestPort::try_from(p).ok())
            .ok_or("a port in range")?;
        self.shell(
            workspace,
            "server",
            &format!("printf conformance-pong | nc -l 127.0.0.1 {port}"),
            "nc -l",
        )
        .await?;
        self.listening(workspace, port).await?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let heard = match self.host.connect(workspace, port).await {
                Ok(mut stream) => {
                    let mut seen = Vec::new();
                    let mut buffer = [0u8; 256];
                    let read = tokio::time::timeout(Duration::from_secs(5), async {
                        while !String::from_utf8_lossy(&seen).contains("conformance-pong") {
                            match stream.read(&mut buffer).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => seen.extend_from_slice(&buffer[..n]),
                            }
                        }
                    })
                    .await;
                    read.is_ok() && String::from_utf8_lossy(&seen).contains("conformance-pong")
                }
                Err(_) => false,
            };
            if heard {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("nothing came back through port {port}"));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Waits for the port to show as a reachable listener, in the `server`
    /// column it runs in.
    async fn listening(&self, workspace: WorkspaceId, port: GuestPort) -> Checked {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let listeners = self
                .host
                .listeners(workspace)
                .await
                .map_err(|e| e.to_string())?;
            let found = listeners.iter().find(|l| l.port == port);
            if let Some(listener) = found
                && listener.reachable
                && listener.column.as_ref().map(SessionName::as_str) == Some("server")
            {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "port {port} never showed as listening in server: {listeners:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// A thread shows in the inventory; closing its session drops it, and a
    /// session opened again under the name starts with none.
    async fn attention(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        self.shell(
            workspace,
            "agent",
            "iglu-status set waiting conformance-question && echo reported-$((6*7))",
            "reported-42",
        )
        .await?;
        let session: SessionName = "agent".parse().expect("a valid session name");
        self.until_reported(workspace, "the waiting thread", |statuses| {
            statuses.iter().any(|status| {
                status.session == session
                    && status.state == AttentionState::Waiting
                    && status.summary.as_str() == "conformance-question"
            })
        })
        .await?;
        self.host
            .close_terminal(workspace, &session)
            .await
            .map_err(|e| e.to_string())?;
        let gone = |statuses: &[SessionStatus]| statuses.iter().all(|s| s.session != session);
        self.until_reported(workspace, "no thread after closing", gone)
            .await?;
        self.shell(workspace, "agent", "echo again-$((6*7))", "again-42")
            .await?;
        self.until_reported(workspace, "no thread in the new session", gone)
            .await?;
        // A frozen guest can't be read, which isn't the same as having no
        // threads: they're there again once it thaws.
        self.shell(
            workspace,
            "agent",
            "iglu-status set done held-over && echo held-$((6*7))",
            "held-42",
        )
        .await?;
        let held = |statuses: &[SessionStatus]| {
            statuses
                .iter()
                .any(|s| s.session == session && s.summary.as_str() == "held-over")
        };
        self.until_reported(workspace, "the done thread", held)
            .await?;
        self.perform(workspace, Command::Freeze).await?;
        let inventory = self.host.inventory().await.map_err(|e| e.to_string())?;
        let frozen = inventory
            .iter()
            .find(|report| report.workspace == workspace);
        ensure(
            frozen.is_some_and(|report| report.sessions.is_none()),
            || format!("a frozen guest's threads read as known: {frozen:?}"),
        )?;
        self.perform(workspace, Command::Thaw).await?;
        self.until_reported(workspace, "the done thread after thawing", held)
            .await
    }

    /// Waits until the host reads the workspace's threads and they pass.
    async fn until_reported(
        &self,
        workspace: WorkspaceId,
        what: &str,
        passes: impl Fn(&[SessionStatus]) -> bool,
    ) -> Checked {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let inventory = self.host.inventory().await.map_err(|e| e.to_string())?;
            let read = inventory
                .iter()
                .find(|report| report.workspace == workspace)
                .and_then(|report| report.sessions.as_deref());
            if read.is_some_and(&passes) {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("never {what}: {inventory:?}"));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// A command run under `caffeinate` shows as a working thread of its
    /// terminal while it runs, and only then.
    async fn caffeinate(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        self.shell(
            workspace,
            "build",
            "caffeinate sh -c 'echo awake-$((6*7)); sleep 5'",
            "awake-42",
        )
        .await?;
        let session: SessionName = "build".parse().expect("a valid session name");
        let awake = |inventory: &[InstanceReport]| {
            inventory
                .iter()
                .filter(|report| report.workspace == workspace)
                .filter_map(|report| report.sessions.as_ref())
                .flatten()
                .any(|status| {
                    status.session == session
                        && status.state == AttentionState::Working
                        && status.thread.as_str().starts_with("caffeinate-")
                })
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut seen = false;
        loop {
            let inventory = self.host.inventory().await.map_err(|e| e.to_string())?;
            match (seen, awake(&inventory)) {
                (false, true) => seen = true,
                (true, false) => return Ok(()),
                (false | true, _) => {}
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "caffeinate {}: {inventory:?}",
                    if seen {
                        "never cleared its thread"
                    } else {
                        "never showed as working"
                    }
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn large_bundle(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        // Bigger than any pipe or socket buffer: 17 files of the largest
        // secret, a little over 1 MiB.
        let files = 17;
        let secrets = (0..files).map(|i| {
            let fill = char::from(b'a' + u8::try_from(i).expect("fewer than 26 files"));
            (
                SecretTarget::File {
                    path: format!(".conformance/big-{i:02}")
                        .parse()
                        .expect("a valid home path"),
                },
                SecretValue::try_from(fill.to_string().repeat(SecretValue::MAX_BYTES))
                    .expect("a secret of the largest size"),
            )
        });
        self.deliver(workspace, secrets).await?;
        let total = files * SecretValue::MAX_BYTES;
        self.shell(
            workspace,
            "count",
            "cat ~/.conformance/big-* | wc -c",
            &total.to_string(),
        )
        .await
    }

    async fn failed_clone(&self, workspace: WorkspaceId) -> Checked {
        self.running_workspace(workspace).await?;
        let spec = ProvisionSpec {
            checkout: Some(Checkout {
                repo: "https://conformance.invalid/missing.git"
                    .parse()
                    .expect("a valid repository URL"),
                branch: "conformance".parse().expect("a valid branch"),
                base: None,
            }),
        };
        self.refused(workspace, Command::Provision(spec), ErrorCode::GuestFailed)
            .await?;
        let instance = self
            .host
            .observe(workspace.instance_name())
            .await
            .map_err(|e| e.to_string())?;
        ensure(
            matches!(
                instance,
                Instance::Present(Present {
                    provisioning: Provisioning::Pending,
                    ..
                })
            ),
            || format!("after a failed clone: {instance:?}"),
        )
    }
}

/// POSIX `cksum`'s CRC, which GNU and BSD `cksum` both print by default.
fn posix_cksum(data: &[u8]) -> u32 {
    fn feed(crc: u32, byte: u8) -> u32 {
        (0..8).fold(crc ^ (u32::from(byte) << 24), |crc, _| {
            if crc & 0x8000_0000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x04C1_1DB7
            }
        })
    }
    let crc = data.iter().fold(0, |crc, &byte| feed(crc, byte));
    let length = u64::try_from(data.len()).unwrap_or(u64::MAX).to_le_bytes();
    let used = length
        .iter()
        .rposition(|&b| b != 0)
        .map_or(0, |last| last + 1);
    !length[..used]
        .iter()
        .fold(crc, |crc, &byte| feed(crc, byte))
}

#[cfg(test)]
mod tests {
    use super::posix_cksum;

    #[test]
    fn cksum_matches_the_posix_check_values() {
        assert_eq!(posix_cksum(b""), 4_294_967_295);
        assert_eq!(posix_cksum(b"123456789"), 930_766_865);
    }
}
