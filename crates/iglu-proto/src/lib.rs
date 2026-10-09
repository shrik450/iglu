//! The protocol between iglud (client) and hostd (server).
//!
//! HTTP/1.1 over TLS with a bearer token. JSON bodies are these types, so
//! deserializing a message parses every value in it. Streams (terminals and
//! port tunnels) upgrade the connection.
//!
//! There is one protocol version. iglud refuses hosts that report another,
//! and both sides change together.

use iglu_domain::attention::SessionStatus;
use iglu_domain::capacity::Bytes;
use iglu_domain::env::{Arch, BuiltImage, EnvSource, GuestUser, ImageFingerprint};
use iglu_domain::id::{PrincipalId, WorkspaceId};
use iglu_domain::label::HostId;
use iglu_domain::lifecycle::Instance;
use iglu_domain::port::GuestPort;
use iglu_domain::repo::{BranchName, RepoUrl};
use iglu_domain::secret::{FetchTokens, SecretBundle};
use iglu_domain::terminal::{SessionName, TerminalSize};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 2;

/// Paths, so client and server can't drift.
pub mod path {
    use super::{GuestPort, SessionName, WorkspaceId};

    pub const HOST: &str = "/v1/host";
    pub const INVENTORY: &str = "/v1/inventory";
    pub const BUILDS: &str = "/v1/builds";

    #[must_use]
    pub fn commands(workspace: WorkspaceId) -> String {
        format!("/v1/workspaces/{workspace}/commands")
    }

    #[must_use]
    pub fn terminals(workspace: WorkspaceId) -> String {
        format!("/v1/workspaces/{workspace}/terminals")
    }

    #[must_use]
    pub fn terminal(workspace: WorkspaceId, session: &SessionName) -> String {
        format!("/v1/workspaces/{workspace}/terminals/{session}")
    }

    #[must_use]
    pub fn attach(workspace: WorkspaceId, session: &SessionName) -> String {
        format!("/v1/workspaces/{workspace}/terminals/{session}/attach")
    }

    #[must_use]
    pub fn tunnel(workspace: WorkspaceId, port: GuestPort) -> String {
        format!("/v1/workspaces/{workspace}/ports/{port}/tunnel")
    }
}

/// The `Upgrade` protocol name for raw port tunnels.
pub const TUNNEL_UPGRADE: &str = "iglu-tunnel";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostReport {
    pub host: HostId,
    pub protocol: u32,
    pub arch: Arch,
    /// Linux `MemAvailable`: what admission checks against.
    pub memory_available: Bytes,
}

/// Everything the reconciler needs about every iglu instance, in one call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub host: HostReport,
    pub workspaces: Vec<InstanceReport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceReport {
    pub workspace: WorkspaceId,
    /// The owner recorded on the instance when it was created.
    pub owner: PrincipalId,
    pub instance: Instance,
    /// Resident memory, when the instance is running or frozen.
    pub memory: Option<Bytes>,
    /// Latest status per terminal session, as the guest reported it.
    pub sessions: Vec<SessionStatus>,
}

/// Limits applied to a workspace's instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub cpus: u16,
    pub memory: Bytes,
    pub processes: u32,
    /// How much memory may move to swap when the workspace freezes.
    pub swap: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSpec {
    pub owner: PrincipalId,
    pub image: ImageFingerprint,
    pub user: GuestUser,
    pub limits: Limits,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisionSpec {
    pub repo: RepoUrl,
    pub branch: BranchName,
    /// Where a new branch starts. `None` means the remote's default branch.
    pub base: Option<BranchName>,
}

/// One lifecycle step, with the payload it needs. Every command is idempotent:
/// repeating one that already took effect succeeds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Create(CreateSpec),
    Start,
    DeliverSecrets(SecretBundle),
    Provision(ProvisionSpec),
    Freeze,
    Thaw,
    Stop,
    Delete,
}

/// A failed command, classified so the control plane can decide what to do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandError {
    pub code: ErrorCode,
    /// Safe to show the owner. Never contains secrets.
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "snake_case"))]
pub enum ErrorCode {
    NotFound,
    /// An instance with this workspace's name exists but isn't iglu's.
    Conflict,
    ImageMissing,
    /// The environment's image was built for a different version of iglu.
    ImageIncompatible,
    /// The instance isn't in a state where the command makes sense.
    InvalidState,
    /// A command inside the guest failed, such as `git clone`.
    GuestFailed,
    Timeout,
    Runtime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CommandOutcome {
    Done { instance: Instance },
    Failed(CommandError),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInfo {
    pub name: SessionName,
    pub clients: u32,
}

/// Query parameters for attaching to a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachParams {
    pub cols: u16,
    pub rows: u16,
}

/// Text frames a terminal client sends. Binary frames are input bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalControl {
    Resize(TerminalSize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildRequest {
    pub source: EnvSource,
    /// The owner's tokens for fetching private flakes.
    pub tokens: FetchTokens,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BuildOutcome {
    Built(BuiltImage),
    Failed {
        /// The end of the build log, for the owner to diagnose their flake.
        log_tail: String,
    },
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}
