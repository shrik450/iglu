//! The state the guest tools keep under the workspace user's runtime
//! directory (`$XDG_RUNTIME_DIR`), which hosts read. `iglu-guest` and
//! `iglu-status` write these files; hostd reads them, relative to wherever
//! its runtime puts that directory.
//!
//! The runtime directory is per boot, so everything here starts empty when
//! a workspace starts.

use crate::attention::{AttentionState, Summary};

/// Written last by `iglu-guest install-secrets`: the secrets delivery the
/// guest holds, as a decimal number.
pub const SECRETS_GENERATION: &str = "iglu/secrets/generation";

/// The environment variable that marks a process as one of a local
/// workspace's, on the dev stack's local runtime, where every workspace's
/// processes are the same user's. Its value is the instance's name.
pub const LOCAL_WORKSPACE: &str = "IGLU_DEVHOST_WORKSPACE";

/// Per-session attention status, maintained by `iglu-status`: sessions,
/// each with its threads, each a [`StatusEntry`].
pub const STATUS: &str = "iglu/status/sessions.json";

/// One thread's entry in the status file. Hosts read it as untrusted and
/// parse it again.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StatusEntry {
    pub state: AttentionState,
    pub summary: Summary,
    pub title: Summary,
    /// Unix milliseconds, by the guest's clock.
    pub at: i64,
}

/// What `iglu-guest git-state` prints for a checkout; `null` without one.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitReport {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changed: u32,
    pub untracked: u32,
    pub conflicted: u32,
}

/// What `iglu-guest output` prints: a session's last lines.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutputReport {
    pub text: String,
    pub truncated: bool,
}

/// One entry of what `iglu-guest listeners` prints.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ListenerReport {
    pub port: u16,
    pub address: std::net::IpAddr,
    pub process: String,
    /// The zmx session the process runs in, if it can be told.
    pub session: Option<String>,
}

/// Written by `iglu-guest open --boot` once a boot has opened the
/// workspace's columns.
pub const COLUMNS_OPENED: &str = "iglu/columns-opened";

/// Where a workspace's processes reach iglu: a socket that only that
/// workspace's channel answers on, so whatever connects is that workspace.
/// The guest tools get its path in [`CHANNEL_ENV`] and pass it on to every
/// session, where the `iglu` command finds it.
pub const CHANNEL_ENV: &str = "IGLU_SOCKET";

/// The channel's path inside an Incus instance. It's on the root
/// filesystem: Incus binds the socket when the instance starts, and one
/// under `/run` is hidden by the tmpfs mounted there on a restart.
pub const INSTANCE_CHANNEL: &str = "/var/lib/iglu-api.sock";

/// Where a person's kept directory is: what their agents keep once it's set
/// up, shared by all of the person's workspaces on a host and by no one
/// else's. The runtime gives the guest tools its path in this variable, and
/// they pass it on to every session.
pub const KEPT_ENV: &str = "IGLU_KEPT";

/// The kept directory's path inside an Incus instance, where the person's
/// volume is mounted.
pub const INSTANCE_KEPT: &str = "/var/lib/iglu/kept";

/// Where the guest tools are inside an Incus instance, in its `bin`: the
/// host's own build, mounted read-only, so they're always the tools that
/// host speaks to, whatever iglu the environment was built with.
pub const INSTANCE_TOOLS: &str = "/var/lib/iglu/tools";

/// What agents and tools keep in the kept directory: each variable names a
/// directory of the program's own there, which it takes as its
/// configuration directory. Sign-in, onboarding, trust and settings are then made once,
/// for every workspace.
pub const KEPT_DIRS: [(&str, &str); 3] = [
    ("CLAUDE_CONFIG_DIR", CLAUDE_KEPT),
    ("CODEX_HOME", "codex"),
    ("GH_CONFIG_DIR", "gh"),
];

/// Claude Code's directory in the kept directory.
pub const CLAUDE_KEPT: &str = "claude";
