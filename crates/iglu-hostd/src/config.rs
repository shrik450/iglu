//! hostd's configuration, written by the NixOS module as JSON and parsed
//! into domain types at startup.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use iglu_domain::auth::{Issuer, Subject};
use iglu_domain::label::HostId;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host_id: HostId,
    pub listen: SocketAddr,
    pub tls: Tls,
    pub auth: Auth,
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
}

/// Which bearer tokens hostd accepts: IdP-issued access tokens for this
/// audience, from one of the listed service subjects.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    pub issuer: Issuer,
    pub audience: String,
    pub subjects: Vec<Subject>,
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
    /// The managed bridge workspaces attach to.
    pub network: String,
    /// The ACL hostd maintains on that bridge.
    pub acl: String,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_field_names,
    reason = "the unit belongs in the configuration keys operators write"
)]
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
    pub const fn operation(&self) -> Duration {
        Duration::from_secs(self.operation_secs)
    }

    pub const fn provision(&self) -> Duration {
        Duration::from_secs(self.provision_secs)
    }
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let config: Self = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
        anyhow::ensure!(
            !config.auth.subjects.is_empty(),
            "auth.subjects must name at least one service subject"
        );
        Ok(config)
    }
}
