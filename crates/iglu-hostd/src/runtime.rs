//! What hostd needs from whatever runs workspaces, in hostd's vocabulary.
//!
//! A runtime holds instances and runs the guest tools inside them. It makes
//! no decisions: which action a command takes in which state is in
//! [`crate::rules`], and [`crate::host`] puts the two together. So a second
//! runtime reuses every rule, and only has to say how to do each step.
//!
//! What the types can't say, such as "freezing a running instance leaves it
//! frozen" or "a new boot starts without secrets", is checked by the
//! conformance suite, which every runtime runs in its real environment.

use std::future::Future;
use std::time::Duration;

use bytes::Bytes;
use iglu_domain::capacity;
use iglu_domain::env::{BuiltImage, EnvSource, GuestUser};
use iglu_domain::guest::Interface;
use iglu_domain::id::{InstanceName, PrincipalId};
use iglu_domain::lifecycle::Provisioning;
use iglu_domain::port::GuestPort;
use iglu_domain::secret::FetchTokens;
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_proto::CreateSpec;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::guest::GuestCommand;

pub trait Runtime: Send + Sync + 'static {
    /// A byte stream to a port inside a guest.
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    /// Memory the host can still give to workspaces, which admission
    /// checks starts and thaws against.
    fn memory_available(&self) -> impl Future<Output = capacity::Bytes> + Send;

    /// Every instance that claims to be iglu's. Instances that don't are
    /// left out.
    fn instances(&self) -> impl Future<Output = Result<Vec<Claim>, RuntimeError>> + Send;

    /// One instance, if it exists. A foreign instance with the name is
    /// reported as [`Ownership::Foreign`].
    fn instance(
        &self,
        name: InstanceName,
    ) -> impl Future<Output = Result<Option<Claim>, RuntimeError>> + Send;

    /// Makes the environment's source usable for [`Runtime::create`] on this
    /// host.
    fn build(
        &self,
        source: &EnvSource,
        tokens: &FetchTokens,
    ) -> impl Future<Output = Result<BuiltImage, BuildFailure>> + Send;

    /// Creates a stopped instance from a built image, recording the spec's
    /// owner and user on it. The name is free.
    fn create(
        &self,
        name: InstanceName,
        spec: &CreateSpec,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send;

    /// Boots a stopped or failed instance. The boot starts with an empty
    /// runtime directory.
    fn start(&self, name: InstanceName) -> impl Future<Output = Result<(), RuntimeError>> + Send;

    /// Pauses every process of a running instance, and makes its memory
    /// cheap to hold where the runtime can.
    fn freeze(&self, name: InstanceName) -> impl Future<Output = Result<(), RuntimeError>> + Send;

    /// Resumes a frozen instance.
    fn thaw(&self, name: InstanceName) -> impl Future<Output = Result<(), RuntimeError>> + Send;

    /// Ends every process of a running or frozen instance: a frozen one is
    /// thawed first, or forced if it can't be; then gracefully within the
    /// stop timeout, then by force.
    fn stop(&self, name: InstanceName) -> impl Future<Output = Result<(), RuntimeError>> + Send;

    /// Removes a stopped instance and everything the runtime keeps for it.
    fn delete(&self, name: InstanceName) -> impl Future<Output = Result<(), RuntimeError>> + Send;

    /// Records on the instance that its repository is in place, so the
    /// record survives every iglu process restarting.
    fn mark_provisioned(
        &self,
        name: InstanceName,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send;

    /// Runs a guest tool command to completion as the workspace user.
    fn run(
        &self,
        guest: &Guest,
        command: &GuestCommand<'_>,
        timeout: Duration,
    ) -> impl Future<Output = Result<Output, RuntimeError>> + Send;

    /// Attaches to an open terminal session, on a terminal of `size`.
    fn attach(
        &self,
        guest: &Guest,
        session: &SessionName,
        size: TerminalSize,
    ) -> impl Future<Output = Result<Terminal, RuntimeError>> + Send;

    /// Reads one of the files the guest keeps for hosts. `None` when it
    /// doesn't exist.
    fn read(
        &self,
        guest: &Guest,
        file: GuestFile,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, RuntimeError>> + Send;

    /// Connects to a port the guest serves on its own loopback.
    fn connect(
        &self,
        guest: &Guest,
        port: GuestPort,
    ) -> impl Future<Output = Result<Self::Stream, RuntimeError>> + Send;
}

/// A runtime that keeps workspaces apart from the host and from each other:
/// a guest can't reach the host, its network or other guests, runs without
/// root, and can't make hostd read or write outside it. Only an isolating
/// runtime can serve iglud over the network (see [`crate::server`]).
///
/// The types can't check this promise. Each implementation backs it with
/// negative tests in its real environment.
pub trait Isolating: Runtime {}

/// An instance as the runtime reports it: iglu's, with the records hostd
/// left on it, or a reason it isn't.
pub type Claim = Result<Observed, Ownership>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observed {
    pub name: InstanceName,
    pub owner: PrincipalId,
    pub user: GuestUser,
    pub provisioning: Provisioning,
    pub state: State,
    /// The guest-tools interface recorded on the instance when it was
    /// created, if any: an instance from before it was recorded says nothing.
    pub guest_interface: Option<Interface>,
    /// Resident memory, when the runtime measures it.
    pub memory: Option<capacity::Bytes>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Ownership {
    #[error("not an iglu instance")]
    Foreign,
    #[error("iglu instance {0} has damaged records: {1}")]
    Damaged(String, &'static str),
}

/// Where an instance is in its lifecycle, before looking inside the guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Running {
        boot: BootId,
        /// Whether the guest has an address to reach the Internet from.
        has_address: bool,
    },
    Frozen,
    /// The runtime is in the middle of changing state.
    Transitioning,
    /// The runtime reports the instance as broken.
    Failed,
}

/// Identifies one boot of an instance: a restart gets a new one. What it is
/// is the runtime's business.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BootId(u64);

impl BootId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A running instance hostd may act inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guest {
    pub name: InstanceName,
    pub user: GuestUser,
    pub boot: BootId,
}

/// The largest guest file a host reads. Guest files are untrusted, so one
/// that's bigger is refused rather than read.
pub const MAX_GUEST_FILE: u64 = 256 * 1024;

/// Files the guest keeps for hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestFile {
    /// Exists once the guest has finished booting.
    Ready,
    /// The secrets delivery the guest holds.
    SecretsGeneration,
    /// Per-session attention status.
    Status,
    /// Exists once the boot has opened the workspace's columns.
    ColumnsOpened,
}

/// How a guest tool command ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// The end of stderr, for error messages.
    #[must_use]
    pub fn stderr_tail(&self) -> String {
        tail(&self.stderr, 2000)
    }
}

#[must_use]
pub fn tail(text: &str, max: usize) -> String {
    let trimmed = text.trim_end();
    if max == 0 {
        return String::new();
    }
    let start = trimmed
        .char_indices()
        .rev()
        .nth(max - 1)
        .map_or(0, |(i, _)| i);
    trimmed[start..].to_owned()
}

/// An attached terminal. Dropping `input` detaches; `output` ends when the
/// guest side closes. The session keeps running either way.
pub struct Terminal {
    pub input: mpsc::Sender<TerminalInput>,
    pub output: mpsc::Receiver<Bytes>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalInput {
    Bytes(Bytes),
    Resize(TerminalSize),
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("no such instance")]
    NotFound,
    #[error("the environment's image isn't on this host")]
    ImageMissing,
    #[error(transparent)]
    ImageIncompatible(#[from] iglu_domain::guest::Incompatible),
    #[error("{0}")]
    Failed(String),
}

impl RuntimeError {
    pub fn failed(error: impl std::fmt::Display) -> Self {
        Self::Failed(error.to_string())
    }
}

/// Why an environment couldn't be built, for its owner to read.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct BuildFailure(pub String);

#[cfg(test)]
mod tests {
    use super::tail;

    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("ab", 3), "ab");
        assert_eq!(tail("héllo\n", 4), "éllo");
    }
}
