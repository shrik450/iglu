//! hostd's configuration, written by the NixOS module as JSON and parsed
//! into domain types at startup. [`Auth`] and [`Timeouts`] are shared with
//! other hosts built on this crate.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iglu_domain::label::HostId;
use serde::Deserialize;
use serde::de::DeserializeOwned;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host_id: HostId,
    pub listen: SocketAddr,
    pub tls: Tls,
    pub incus: Incus,
    pub build: Build,
    /// Where hostd keeps runtime files such as proxy sockets.
    pub runtime_dir: PathBuf,
    #[serde(default)]
    pub timeouts: Timeouts,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    pub certificate: PathBuf,
    pub key: PathBuf,
    /// The CA whose certificates may call this host: the one that signs
    /// iglud's client certificate, and nothing else, or any certificate it
    /// signed could command the host.
    pub clients: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Incus {
    pub socket: PathBuf,
    /// The `incus` CLI, used only to import images.
    pub binary: PathBuf,
    #[serde(default)]
    pub project: Option<String>,
    /// The profile every workspace instance uses: root disk and NIC.
    pub profile: String,
    /// The storage pool each person's kept volume is on.
    pub pool: String,
    /// The managed bridge workspaces attach to.
    pub network: String,
    /// The ACL hostd maintains on that bridge.
    pub acl: String,
    /// The guest tools, a directory with their static build in its `bin`,
    /// mounted read-only into every instance.
    pub guest_tools: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub nix: PathBuf,
    /// Evaluation runs as this unprivileged account; the Nix daemon builds.
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
    #[serde(default = "default_build_timeout")]
    pub timeout_secs: u64,
}

const fn default_build_timeout() -> u64 {
    3600
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeouts {
    pub operation_secs: u64,
    pub stop_secs: u32,
    pub provision_secs: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            operation_secs: 120,
            stop_secs: 30,
            provision_secs: 1800,
        }
    }
}

impl Timeouts {
    #[must_use]
    pub const fn operation(&self) -> Duration {
        Duration::from_secs(self.operation_secs)
    }

    #[must_use]
    pub const fn provision(&self) -> Duration {
        Duration::from_secs(self.provision_secs)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {0}: {1}")]
    Read(PathBuf, #[source] std::io::Error),
    #[error("parsing {0}: {1}")]
    Parse(PathBuf, #[source] serde_json::Error),
}

/// Reads a JSON configuration file, parsing every value in it.
///
/// # Errors
///
/// When the file can't be read or doesn't parse.
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<T, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Read(path.into(), e))?;
    serde_json::from_str(&text).map_err(|e| ConfigError::Parse(path.into(), e))
}
