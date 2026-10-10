//! hostd's shell over a runtime: observe what the runtime reports, decide
//! with [`crate::rules`], and act through the runtime.

use std::collections::HashMap;
use std::sync::Mutex;

use iglu_domain::capacity::Bytes;
use iglu_domain::env::{EnvSource, GuestPath};
use iglu_domain::git::GitState;
use iglu_domain::guest::OutputReport;
use iglu_domain::id::{InstanceName, WorkspaceId};
use iglu_domain::lifecycle::{Instance, SecretsGeneration};
use iglu_domain::listener::Listener;
use iglu_domain::pasted::FileName;
use iglu_domain::port::GuestPort;
use iglu_domain::secret::{FetchTokens, SecretBundle};
use iglu_domain::terminal::{self, OutputLines, SessionName, TerminalInput, TerminalSize};
use iglu_proto::{
    BuildOutcome, Command, CommandError, CommandOutcome, ErrorCode, InstanceReport, ProvisionSpec,
    SessionSpec, TerminalInfo, TerminalOutput,
};

use crate::channel::Channels;
use crate::config::Timeouts;
use crate::guest::{self, GuestCommand, Opening};
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
    channels: Channels,
}

impl<R: Runtime> Host<R> {
    pub fn new(runtime: R, timeouts: Timeouts) -> Self {
        Self {
            runtime,
            boots: BootCache::default(),
            timeouts,
            channels: Channels::default(),
        }
    }

    pub const fn channels(&self) -> &Channels {
        &self.channels
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
            Command::OpenColumns { sessions } => self.open_columns(name, sessions).await,
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

    /// The running guest, for a command its guest tools must know: an
    /// instance from another iglu is refused with what to do about it.
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
            GuestCommand::InstallSecrets(_)
            | GuestCommand::Open(..)
            | GuestCommand::Sessions
            | GuestCommand::Listeners
            | GuestCommand::GitState
            | GuestCommand::Close(_)
            | GuestCommand::Output(..)
            | GuestCommand::Input(..)
            | GuestCommand::Keep(..) => self.timeouts.operation(),
        };
        let output = self.runtime.run(guest, command, timeout).await?;
        if output.success {
            Ok(output)
        } else {
            Err(CommandError::new(
                ErrorCode::GuestFailed,
                format!("{failing}: {}", guest_message(&output.stderr_tail())),
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

    /// Opens the workspace's columns for this boot. A column whose program
    /// fails to start still counts as opened: it shows as ended, and a
    /// broken dev server mustn't keep the workspace from coming up.
    async fn open_columns(
        &self,
        name: InstanceName,
        sessions: &[SessionSpec],
    ) -> Result<(), CommandError> {
        let guest = self.tools_guest(name).await?;
        self.run(
            &guest,
            &GuestCommand::Open(sessions, Opening::Boot),
            "opening the columns failed",
        )
        .await?;
        self.boots.columns_opened(name, guest.boot);
        Ok(())
    }

    /// Opens one more terminal session in a running workspace.
    ///
    /// # Errors
    ///
    /// When the workspace isn't running, or the guest tool fails.
    pub async fn open_terminal(
        &self,
        workspace: WorkspaceId,
        session: &SessionSpec,
    ) -> Result<(), CommandError> {
        let guest = self.guest(workspace.instance_name()).await?;
        self.run(
            &guest,
            &GuestCommand::Open(std::slice::from_ref(session), Opening::Column),
            "opening the session failed",
        )
        .await?;
        Ok(())
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
        if facts.ready && !facts.columns_opened {
            facts.columns_opened = matches!(
                self.runtime.read(&guest, GuestFile::ColumnsOpened).await,
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
    /// Listing them is also when hostd starts listening for each new
    /// instance's channel, and stops for each one that's gone.
    pub async fn inventory(&self) -> Result<Vec<InstanceReport>, CommandError> {
        let mut reports = Vec::new();
        let mut channels = Vec::new();
        for claim in self.runtime.instances().await? {
            match claim {
                Ok(observed) => {
                    channels.push((observed.name, self.runtime.channel(observed.name)));
                    reports.push(self.report(&observed).await);
                }
                Err(Ownership::Foreign) => {}
                Err(error @ Ownership::Damaged(..)) => {
                    tracing::warn!(%error, "skipping an instance");
                }
            }
        }
        self.channels.listen(channels);
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

    /// What the workspace user has listening.
    ///
    /// # Errors
    ///
    /// When the workspace isn't running, or the guest tool fails.
    pub async fn listeners(&self, workspace: WorkspaceId) -> Result<Vec<Listener>, CommandError> {
        let guest = self.guest(workspace.instance_name()).await?;
        let output = self
            .run(&guest, &GuestCommand::Listeners, "listing listeners failed")
            .await?;
        Ok(guest::parse_listeners(&output.stdout))
    }

    /// Where the workspace's checkout stands; `None` without a repository.
    ///
    /// # Errors
    ///
    /// When the workspace isn't running, or the guest tool fails.
    pub async fn git_state(
        &self,
        workspace: WorkspaceId,
    ) -> Result<Option<GitState>, CommandError> {
        let guest = self.guest(workspace.instance_name()).await?;
        let output = self
            .run(
                &guest,
                &GuestCommand::GitState,
                "reading the Git state failed",
            )
            .await?;
        Ok(guest::parse_git_state(&output.stdout))
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

    /// A session's last lines. What the guest prints is untrusted, so it's
    /// cut to size again here.
    ///
    /// # Errors
    ///
    /// When the workspace isn't running, or the session isn't open.
    pub async fn output(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
        lines: OutputLines,
    ) -> Result<TerminalOutput, CommandError> {
        let guest = self.tools_guest(workspace.instance_name()).await?;
        let output = self
            .run(
                &guest,
                &GuestCommand::Output(session, lines),
                "reading the session failed",
            )
            .await?;
        let printed: OutputReport = serde_json::from_str(&output.stdout).map_err(|e| {
            CommandError::new(ErrorCode::GuestFailed, format!("unreadable output: {e}"))
        })?;
        let (text, cut) = terminal::last_lines(&printed.text, lines);
        Ok(TerminalOutput {
            text,
            truncated: printed.truncated || cut,
        })
    }

    /// Types into an open session.
    ///
    /// # Errors
    ///
    /// When the workspace isn't running, or the session isn't open.
    pub async fn input(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
        text: &TerminalInput,
    ) -> Result<(), CommandError> {
        let guest = self.tools_guest(workspace.instance_name()).await?;
        self.run(
            &guest,
            &GuestCommand::Input(session, text),
            "typing into the session failed",
        )
        .await?;
        Ok(())
    }

    /// Keeps a file pasted on one of the workspace's terminals, and says
    /// where it is. What the guest prints is untrusted: it must be a path.
    ///
    /// # Errors
    ///
    /// When the workspace isn't running, or the guest can't keep the file.
    pub async fn keep(
        &self,
        workspace: WorkspaceId,
        name: &FileName,
        bytes: &[u8],
    ) -> Result<GuestPath, CommandError> {
        let guest = self.tools_guest(workspace.instance_name()).await?;
        let output = self
            .run(
                &guest,
                &GuestCommand::Keep(name, bytes),
                "keeping the file failed",
            )
            .await?;
        output
            .stdout
            .trim_end()
            .parse()
            .map_err(|e| CommandError::new(ErrorCode::GuestFailed, format!("unreadable path: {e}")))
    }

    /// Attaches to an open terminal session.
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
                columns_opened: false,
            },
        );
    }

    /// Records that a boot opened its columns.
    fn columns_opened(&self, name: InstanceName, boot: BootId) {
        self.learn(
            name,
            boot,
            BootFacts {
                ready: true,
                secrets: None,
                columns_opened: true,
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

/// A guest tool's error without the name it prefixes its messages with,
/// which says nothing to the person reading `cloning the repository failed`.
fn guest_message(stderr: &str) -> &str {
    stderr
        .strip_prefix("iglu-guest: ")
        .or_else(|| stderr.strip_prefix("iglu-status: "))
        .unwrap_or(stderr)
}

#[cfg(test)]
mod tests {
    use super::guest_message;

    #[test]
    fn a_guest_tools_name_is_left_out() {
        assert_eq!(
            guest_message("iglu-guest: git clone failed: x"),
            "git clone failed: x"
        );
        assert_eq!(guest_message("bash: oops"), "bash: oops");
    }
}
