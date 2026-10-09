//! iglu-devhost's configuration: a JSON file, parsed into domain types at
//! startup. `just dev` writes it.

use std::path::{Path, PathBuf};

use iglu_domain::capacity::Bytes;
use iglu_domain::label::HostId;
use iglu_hostd::config::{Auth, Timeouts};
use iglu_hostd::server::Loopback;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host_id: HostId,
    /// Plain HTTP, so only on loopback.
    pub listen: Loopback,
    pub auth: Auth,
    pub runtime: Settings,
    #[serde(default)]
    pub timeouts: Timeouts,
}

/// What the local runtime needs.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Workspace homes and records, and the environments recorded here.
    pub state_dir: AbsolutePath,
    /// Where each running workspace's `XDG_RUNTIME_DIR` goes. Terminal
    /// sessions put Unix sockets there, whose paths are limited to about
    /// 100 bytes, so this must be a short path.
    pub runtime_dir: AbsolutePath,
    /// The directory with `iglu-guest` and `iglu-status`.
    pub guest_tools: AbsolutePath,
    /// What the host reports as available to workspaces. The local runtime
    /// doesn't meter memory, so this is a setting, which lets you try how
    /// the console waits for room.
    pub memory_available: Bytes,
    /// The shell terminal sessions run; your account's login shell when unset.
    #[serde(default)]
    pub shell: Option<AbsolutePath>,
    /// The agents every environment gets here. Local workspaces use this
    /// machine's tools, so agents are this machine's, not the flake's.
    #[serde(default)]
    pub agents: Vec<iglu_domain::agent::AgentSpec>,
    /// A CA certificate that workspaces' Git trusts for `https://*.localhost`
    /// only, such as the dev CA, so workspaces can clone the dev stack's
    /// fixture repositories at `git.localhost`. Other hosts keep Git's own
    /// trust.
    #[serde(default)]
    pub localhost_ca: Option<AbsolutePath>,
}

/// An absolute path in UTF-8, so it can become a guest path.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct AbsolutePath(PathBuf);

impl AbsolutePath {
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl TryFrom<String> for AbsolutePath {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let path = PathBuf::from(&value);
        if path.is_absolute() {
            Ok(Self(path))
        } else {
            Err(format!("{value} isn't an absolute path"))
        }
    }
}
