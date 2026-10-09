//! What every runtime owes hostd that the types can't say, checked through
//! hostd's own commands against a runtime in its real environment: real
//! Incus in the VM test, real processes for the local runtime.
//!
//! Each check works on a workspace of its own and deletes it afterwards.
//! A new runtime is done when it passes every check here, and, if it's to
//! be [`crate::runtime::Isolating`], its own negative isolation tests.

use std::future::Future;
use std::time::{Duration, Instant};

use bytes::Bytes;
use iglu_domain::attention::AttentionState;
use iglu_domain::capacity;
use iglu_domain::column::{Arg, Argv};
use iglu_domain::env::{BuiltImage, EnvSource};
use iglu_domain::guest as guest_tools;
use iglu_domain::id::{PrincipalId, WorkspaceId};
use iglu_domain::lifecycle::{
    Columns, Instance, Present, Provisioning, Readiness, Running, Runtime as Status,
    SecretsGeneration,
};
use iglu_domain::port::GuestPort;
use iglu_domain::repo::Checkout;
use iglu_domain::secret::{FetchTokens, SecretTarget, SecretValue, bundle};
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_proto::{
    BuildOutcome, Command, CommandOutcome, CreateSpec, ErrorCode, InstanceReport, Limits,
    ProvisionSpec, SessionSpec,
};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::guest::parse_generation;
use crate::host::Host;
use crate::runtime::{Guest, GuestFile, Runtime, TerminalInput};

/// What the suite needs from the environment it runs in.
pub struct Fixture {
    /// An environment the runtime can build here.
    pub source: EnvSource,
    /// How long a started workspace may take to finish booting.
    pub boot_timeout: Duration,
}

/// One check's result.
#[derive(Debug)]
pub struct Outcome {
    pub check: &'static str,
    pub result: Result<(), String>,
}

/// Runs every check, in order, and reports each.
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
                "creating is idempotent and records the interface",
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
                "provisioning outlives a restart",
                Suite::provisioning_outlives_a_restart,
            )
            .await,
    );
    outcomes.push(
        suite
            .check("terminals run in the workspace", Suite::terminals)
            .await,
    );
    outcomes.push(
        suite
            .check(
                "columns open once a boot, with their arguments exactly as given",
                Suite::opens_columns,
            )
            .await,
    );
    outcomes.push(
        suite
            .check("stopping a frozen workspace ends it", Suite::stops_frozen)
            .await,
    );
    outcomes.push(
        suite
            .check(
                "tunnels reach the guest's ports, which show as listening",
                Suite::tunnels,
            )
            .await,
    );
    outcomes.push(
        suite
            .check("attention reaches the inventory", Suite::attention)
            .await,
    );
    outcomes.push(
        suite
            .check(
                "caffeinate keeps a terminal working while its command runs",
                Suite::caffeinate,
            )
            .await,
    );
    outcomes.push(
        suite
            .check("a large bundle arrives whole", Suite::large_bundle)
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
}

fn ensure(condition: bool, what: impl FnOnce() -> String) -> Checked {
    if condition { Ok(()) } else { Err(what()) }
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

    fn spec(&self) -> CreateSpec {
        CreateSpec {
            owner: PrincipalId::from_uuid(Uuid::new_v4()),
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
        let recorded = match self
            .host
            .runtime()
            .instance(workspace.instance_name())
            .await
        {
            Ok(Some(Ok(observed))) => observed.guest_interface,
            other => return Err(format!("the created instance: {other:?}")),
        };
        ensure(recorded == Some(guest_tools::INTERFACE), || {
            format!("the instance records interface {recorded:?}")
        })?;
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
        let session: SessionName = session.parse().map_err(|e| format!("{e}"))?;
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
        let mut terminal = self
            .host
            .attach(workspace, &session, TerminalSize::DEFAULT)
            .await
            .map_err(|e| e.to_string())?;
        terminal
            .input
            .send(TerminalInput::Bytes(Bytes::from(format!("{line}\r"))))
            .await
            .map_err(|_| "the terminal closed before any input".to_owned())?;
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !String::from_utf8_lossy(&seen).contains(expect) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, terminal.output.recv()).await {
                Ok(Some(bytes)) => seen.extend_from_slice(&bytes),
                Ok(None) | Err(_) => {
                    return Err(format!(
                        "{line:?} never showed {expect:?}: {:?}",
                        String::from_utf8_lossy(&seen)
                    ));
                }
            }
        }
        // Let the shell take the line before detaching.
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(())
    }

    async fn running_workspace(&self, workspace: WorkspaceId) -> Checked {
        self.perform(workspace, Command::Create(self.spec()))
            .await?;
        self.boot(workspace).await.map(|_| ())
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
            .host
            .attach(workspace, session, TerminalSize::DEFAULT)
            .await
            .map_err(|e| e.to_string())?;
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !String::from_utf8_lossy(&seen).contains(expect) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, terminal.output.recv()).await {
                Ok(Some(bytes)) => seen.extend_from_slice(&bytes),
                Ok(None) | Err(_) => {
                    return Err(format!(
                        "{session} never showed {expect:?}: {:?}",
                        String::from_utf8_lossy(&seen)
                    ));
                }
            }
        }
        Ok(String::from_utf8_lossy(&seen).into_owned())
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
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let inventory = self.host.inventory().await.map_err(|e| e.to_string())?;
            let reported = inventory
                .iter()
                .filter(|report| report.workspace == workspace)
                .flat_map(|report| &report.sessions)
                .any(|status| {
                    status.session == session
                        && status.state == AttentionState::Waiting
                        && status.summary.as_str() == "conformance-question"
                });
            if reported {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("the inventory never showed it: {inventory:?}"));
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
                .flat_map(|report| &report.sessions)
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
