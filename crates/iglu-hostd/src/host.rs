//! hostd's shell over a runtime: observe what the runtime reports, decide
//! with [`crate::rules`], and act through the runtime.

use std::collections::HashMap;
use std::sync::Mutex;

use iglu_domain::capacity::Bytes;
use iglu_domain::env::EnvSource;
use iglu_domain::id::{InstanceName, WorkspaceId};
use iglu_domain::lifecycle::{Instance, SecretsGeneration};
use iglu_domain::port::GuestPort;
use iglu_domain::secret::{FetchTokens, SecretBundle};
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_proto::{
    BuildOutcome, Command, CommandError, CommandOutcome, ErrorCode, InstanceReport, ProvisionSpec,
    TerminalInfo,
};

use crate::config::Timeouts;
use crate::guest::{self, GuestCommand};
use crate::rules::{self, BootFacts, Step};
use crate::runtime::{
    BootId, Guest, GuestFile, Observed, Output, Ownership, Runtime, RuntimeError, Terminal, tail,
};

impl From<RuntimeError> for CommandError {
    fn from(error: RuntimeError) -> Self {
        let code = match error {
            RuntimeError::NotFound => ErrorCode::NotFound,
            RuntimeError::ImageMissing => ErrorCode::ImageMissing,
            RuntimeError::ImageIncompatible(_) => ErrorCode::ImageIncompatible,
            RuntimeError::Failed(_) => ErrorCode::Runtime,
        };
        Self::new(code, error.to_string())
    }
}

pub struct Host<R> {
    runtime: R,
    boots: BootCache,
    timeouts: Timeouts,
}

impl<R: Runtime> Host<R> {
    pub fn new(runtime: R, timeouts: Timeouts) -> Self {
        Self {
            runtime,
            boots: BootCache::default(),
            timeouts,
        }
    }

    pub const fn runtime(&self) -> &R {
        &self.runtime
    }

    pub async fn memory_available(&self) -> Bytes {
        self.runtime.memory_available().await
    }

    /// Performs one protocol command and reports the instance afterwards.
    pub async fn perform(&self, workspace: WorkspaceId, command: Command) -> CommandOutcome {
        let name = workspace.instance_name();
        let result = match &command {
            Command::Create(spec) => self.lifecycle(name, Step::Create(spec)).await,
            Command::Start => self.lifecycle(name, Step::Start).await,
            Command::DeliverSecrets(bundle) => self.deliver_secrets(name, bundle).await,
            Command::Provision(spec) => self.provision(name, spec).await,
            Command::Freeze => self.lifecycle(name, Step::Freeze).await,
            Command::Thaw => self.lifecycle(name, Step::Thaw).await,
            Command::Stop => self.lifecycle(name, Step::Stop).await,
            Command::Delete => self.lifecycle(name, Step::Delete).await,
        };
        match result {
            Ok(()) => match self.observe(name).await {
                Ok(instance) => CommandOutcome::Done { instance },
                Err(error) => CommandOutcome::Failed(error),
            },
            Err(error) => {
                tracing::warn!(%name, %error, "command failed");
                CommandOutcome::Failed(error)
            }
        }
    }

    async fn lifecycle(&self, name: InstanceName, command: Step<'_>) -> Result<(), CommandError> {
        let found = self.runtime.instance(name).await?;
        for step in rules::decide(command, found.as_ref())? {
            match step {
                Step::Create(spec) => self.runtime.create(name, spec).await?,
                Step::Start => self.runtime.start(name).await?,
                Step::Freeze => self.runtime.freeze(name).await?,
                Step::Thaw => self.runtime.thaw(name).await?,
                Step::Stop => {
                    self.runtime.stop(name).await?;
                    self.boots.forget(name);
                }
                Step::Delete => {
                    self.runtime.delete(name).await?;
                    self.boots.forget(name);
                }
            }
        }
        Ok(())
    }

    /// The running guest of a workspace's instance.
    ///
    /// # Errors
    ///
    /// When the instance is missing, isn't iglu's, or isn't running.
    pub async fn guest(&self, name: InstanceName) -> Result<Guest, CommandError> {
        rules::guest(self.runtime.instance(name).await?.as_ref())
    }

    async fn tools_guest(&self, name: InstanceName) -> Result<Guest, CommandError> {
        rules::tools_guest(self.runtime.instance(name).await?.as_ref())
    }

    async fn run(
        &self,
        guest: &Guest,
        command: &GuestCommand<'_>,
        failing: &str,
    ) -> Result<Output, CommandError> {
        let timeout = match command {
            GuestCommand::Provision(_) => self.timeouts.provision(),
            GuestCommand::InstallSecrets(_) | GuestCommand::Sessions | GuestCommand::Close(_) => {
                self.timeouts.operation()
            }
        };
        let output = self.runtime.run(guest, command, timeout).await?;
        if output.success {
            Ok(output)
        } else {
            Err(CommandError::new(
                ErrorCode::GuestFailed,
                format!("{failing}: {}", output.stderr_tail()),
            ))
        }
    }

    async fn deliver_secrets(
        &self,
        name: InstanceName,
        bundle: &SecretBundle,
    ) -> Result<(), CommandError> {
        let guest = self.tools_guest(name).await?;
        self.run(
            &guest,
            &GuestCommand::InstallSecrets(bundle),
            "installing secrets failed",
        )
        .await?;
        self.boots.delivered(name, guest.boot, bundle.generation);
        Ok(())
    }

    async fn provision(
        &self,
        name: InstanceName,
        spec: &ProvisionSpec,
    ) -> Result<(), CommandError> {
        let guest = self.tools_guest(name).await?;
        self.run(
            &guest,
            &GuestCommand::Provision(spec),
            "cloning the repository failed",
        )
        .await?;
        Ok(self.runtime.mark_provisioned(name).await?)
    }

    /// What the control plane sees of an instance.
    ///
    /// # Errors
    ///
    /// When the runtime fails, or the instance isn't iglu's.
    pub async fn observe(&self, name: InstanceName) -> Result<Instance, CommandError> {
        match self.runtime.instance(name).await? {
            None => Ok(Instance::Absent),
            Some(Ok(observed)) => Ok(rules::instance(&observed, self.facts(&observed).await)),
            Some(Err(ownership)) => Err(CommandError::new(
                ErrorCode::Conflict,
                ownership.to_string(),
            )),
        }
    }

    /// What the guest says about its current boot, read once per boot where
    /// it can't change within one.
    async fn facts(&self, observed: &Observed) -> BootFacts {
        let Some(guest) = rules::running(observed) else {
            return BootFacts::default();
        };
        let boot = guest.boot;
        let mut facts = self.boots.get(guest.name, boot);
        if !facts.ready {
            facts.ready = matches!(
                self.runtime.read(&guest, GuestFile::Ready).await,
                Ok(Some(_))
            );
        }
        if facts.ready && facts.secrets.is_none() {
            facts.secrets = match self
                .runtime
                .read(&guest, GuestFile::SecretsGeneration)
                .await
            {
                Ok(Some(bytes)) => guest::parse_generation(&bytes),
                Ok(None) | Err(_) => None,
            };
        }
        self.boots.learn(guest.name, boot, facts)
    }

    /// Everything the reconciler needs about every iglu instance.
    ///
    /// # Errors
    ///
    /// When the runtime can't list its instances.
    pub async fn inventory(&self) -> Result<Vec<InstanceReport>, CommandError> {
        let mut reports = Vec::new();
        for claim in self.runtime.instances().await? {
            match claim {
                Ok(observed) => reports.push(self.report(&observed).await),
                Err(Ownership::Foreign) => {}
                Err(error @ Ownership::Damaged(..)) => {
                    tracing::warn!(%error, "skipping an instance");
                }
            }
        }
        Ok(reports)
    }

    async fn report(&self, observed: &Observed) -> InstanceReport {
        let instance = rules::instance(observed, self.facts(observed).await);
        let sessions = match rules::running(observed) {
            Some(guest) => match self.runtime.read(&guest, GuestFile::Status).await {
                Ok(Some(bytes)) => guest::parse_statuses(&bytes),
                Ok(None) | Err(_) => Vec::new(),
            },
            None => Vec::new(),
        };
        InstanceReport {
            workspace: observed.name.workspace(),
            owner: observed.owner,
            instance,
            memory: observed.memory,
            sessions,
        }
    }

    /// The workspace's terminal sessions.
    ///
    /// # Errors
    ///
    /// When the workspace isn't running, or the guest tool fails.
    pub async fn terminals(
        &self,
        workspace: WorkspaceId,
    ) -> Result<Vec<TerminalInfo>, CommandError> {
        let guest = self.guest(workspace.instance_name()).await?;
        let output = self
            .run(&guest, &GuestCommand::Sessions, "listing sessions failed")
            .await?;
        guest::parse_sessions(&output.stdout).map_err(|e| {
            CommandError::new(
                ErrorCode::GuestFailed,
                format!("unreadable session list: {e}"),
            )
        })
    }

    /// Ends a terminal session and everything running in it.
    ///
    /// # Errors
    ///
    /// When the workspace isn't running, or the guest tool fails.
    pub async fn close_terminal(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
    ) -> Result<(), CommandError> {
        let guest = self.guest(workspace.instance_name()).await?;
        self.run(
            &guest,
            &GuestCommand::Close(session),
            "closing the session failed",
        )
        .await?;
        Ok(())
    }

    /// Attaches to a terminal session, creating it on first attach.
    ///
    /// # Errors
    ///
    /// When the workspace isn't running, or the runtime can't attach.
    pub async fn attach(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
        size: TerminalSize,
    ) -> Result<Terminal, CommandError> {
        let guest = self.guest(workspace.instance_name()).await?;
        Ok(self.runtime.attach(&guest, session, size).await?)
    }

    /// Connects to a port the guest serves on its loopback.
    ///
    /// # Errors
    ///
    /// When the workspace isn't running, or the runtime can't connect.
    pub async fn connect(
        &self,
        workspace: WorkspaceId,
        port: GuestPort,
    ) -> Result<R::Stream, CommandError> {
        let guest = self.guest(workspace.instance_name()).await?;
        Ok(self.runtime.connect(&guest, port).await?)
    }

    /// Builds an environment for this host, reporting a failure for its owner.
    pub async fn build(&self, source: &EnvSource, tokens: &FetchTokens) -> BuildOutcome {
        match self.runtime.build(source, tokens).await {
            Ok(image) => BuildOutcome::Built(image),
            Err(error) => {
                tracing::warn!(%source, %error, "environment build failed");
                BuildOutcome::Failed {
                    log_tail: tail(&error.to_string(), 8000),
                }
            }
        }
    }
}

/// [`BootFacts`] per instance, for its current boot only. Rebuilt from
/// scratch if hostd restarts.
#[derive(Default)]
struct BootCache(Mutex<HashMap<InstanceName, (BootId, BootFacts)>>);

impl BootCache {
    fn get(&self, name: InstanceName, boot: BootId) -> BootFacts {
        let cache = self
            .0
            .lock()
            .expect("the boot cache lock is never held across a panic");
        match cache.get(&name) {
            Some((cached, facts)) if *cached == boot => *facts,
            Some(_) | None => BootFacts::default(),
        }
    }

    /// Adds what was just learned about a boot to what's known, under one
    /// lock, so a read that started before a delivery can't undo it.
    fn learn(&self, name: InstanceName, boot: BootId, learned: BootFacts) -> BootFacts {
        let mut cache = self
            .0
            .lock()
            .expect("the boot cache lock is never held across a panic");
        let known = match cache.get(&name) {
            Some((cached, facts)) if *cached == boot => *facts,
            Some(_) | None => BootFacts::default(),
        };
        let merged = known.merge(learned);
        cache.insert(name, (boot, merged));
        merged
    }

    /// Records a secrets delivery that just succeeded.
    fn delivered(&self, name: InstanceName, boot: BootId, generation: SecretsGeneration) {
        self.learn(
            name,
            boot,
            BootFacts {
                ready: true,
                secrets: Some(generation),
            },
        );
    }

    fn forget(&self, name: InstanceName) {
        self.0
            .lock()
            .expect("the boot cache lock is never held across a panic")
            .remove(&name);
    }
}
