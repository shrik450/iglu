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
use iglu_domain::column::Argv;
use iglu_domain::env::{Arch, BuiltImage, EnvSource, GuestUser, ImageFingerprint};
use iglu_domain::id::{PrincipalId, WorkspaceId};
use iglu_domain::label::HostId;
use iglu_domain::lifecycle::Instance;
use iglu_domain::port::GuestPort;
use iglu_domain::repo::Checkout;
use iglu_domain::secret::{FetchTokens, SecretBundle};
use iglu_domain::terminal::SessionName;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 3;

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
    pub fn git(workspace: WorkspaceId) -> String {
        format!("/v1/workspaces/{workspace}/git")
    }

    #[must_use]
    pub fn listeners(workspace: WorkspaceId) -> String {
        format!("/v1/workspaces/{workspace}/listeners")
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
    /// The repository to clone. A workspace without one starts its sessions
    /// in the home directory.
    pub checkout: Option<Checkout>,
}

/// A terminal session to open, if it isn't open already.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSpec {
    pub name: SessionName,
    /// What it runs, as given. `None` runs the user's login shell.
    pub command: Option<Argv>,
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
    /// Opens the workspace's columns for this boot, then records that it did.
    /// A struct variant: the tag can't sit beside a bare list.
    OpenColumns {
        sessions: Vec<SessionSpec>,
    },
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command crosses HTTP as JSON; one that can't is a step no
    /// workspace ever gets past.
    #[test]
    fn every_command_survives_json() {
        let argv: Argv = vec![
            "claude".parse().expect("an argument"),
            "fix it".parse().expect("an argument"),
        ]
        .try_into()
        .expect("a command");
        let commands = [
            Command::Start,
            Command::Provision(ProvisionSpec { checkout: None }),
            Command::Provision(ProvisionSpec {
                checkout: Some(Checkout {
                    repo: "https://github.com/acme/app.git"
                        .parse()
                        .expect("a repository"),
                    branch: "fix".parse().expect("a branch"),
                    base: None,
                }),
            }),
            Command::OpenColumns {
                sessions: vec![
                    SessionSpec {
                        name: "shell".parse().expect("a session"),
                        command: None,
                    },
                    SessionSpec {
                        name: "claude".parse().expect("a session"),
                        command: Some(argv),
                    },
                ],
            },
            Command::Freeze,
            Command::Thaw,
            Command::Stop,
            Command::Delete,
        ];
        for command in commands {
            let json = serde_json::to_string(&command).expect("serializes");
            let back: Command = serde_json::from_str(&json).expect("deserializes");
            assert_eq!(back, command, "{json}");
        }
    }
}
