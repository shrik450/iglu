//! iglud's configuration, written by the NixOS module as JSON.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use iglu_domain::auth::{Issuer, SignInPolicy};
use iglu_domain::capacity::Bytes;
use iglu_domain::label::HostId;
use serde::Deserialize;
use url::Url;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Plain HTTP; TLS terminates in front of iglud.
    pub listen: SocketAddr,
    /// The console's public origin, such as `https://iglu.example.org`.
    pub console_origin: Url,
    /// Previews live at `<route>.<preview_domain>`.
    pub preview_domain: String,
    pub database: PathBuf,
    /// 32 random bytes, hex-encoded, that encrypt stored secrets.
    pub secret_key_file: PathBuf,
    pub console_assets: PathBuf,
    pub oidc: Oidc,
    pub sign_in: SignInPolicy,
    pub hosts: Vec<Host>,
    #[serde(default)]
    pub workspaces: WorkspaceDefaults,
    #[serde(default)]
    pub sessions: SessionPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Oidc {
    pub issuer: Issuer,
    pub console: OidcClient,
    /// The preview gateway is its own relying party.
    pub preview: OidcClient,
    /// The service identity iglud uses to call hosts (client credentials).
    pub worker: WorkerClient,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerClient {
    pub client_id: String,
    pub client_secret_file: PathBuf,
    /// The audience iglud asks for, and hosts require (`auth.audience` in
    /// hostd's configuration).
    pub audience: String,
}

impl WorkerClient {
    pub fn secret(&self) -> anyhow::Result<String> {
        read_secret(&self.client_secret_file)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcClient {
    pub client_id: String,
    pub client_secret_file: PathBuf,
}

impl OidcClient {
    pub fn secret(&self) -> anyhow::Result<String> {
        read_secret(&self.client_secret_file)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub id: HostId,
    pub url: Url,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceDefaults {
    pub cpus: u16,
    pub memory: Bytes,
    pub swap: Bytes,
    pub processes: u32,
    /// What admission charges a workspace when it starts or thaws.
    pub reservation: Bytes,
    /// Memory the host always keeps free.
    pub headroom: Bytes,
}

impl Default for WorkspaceDefaults {
    fn default() -> Self {
        Self {
            cpus: 4,
            memory: Bytes::gib(8),
            swap: Bytes::gib(8),
            processes: 8192,
            reservation: Bytes::gib(1),
            headroom: Bytes::gib(1),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPolicy {
    pub absolute_hours: u32,
    pub idle_minutes: u32,
    pub cli_days: u32,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self {
            absolute_hours: 8,
            idle_minutes: 60,
            cli_days: 30,
        }
    }
}

pub fn read_secret(path: &Path) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    Ok(text.trim().to_owned())
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let config: Self = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
        anyhow::ensure!(!config.hosts.is_empty(), "configure at least one host");
        anyhow::ensure!(
            config
                .preview_domain
                .split('.')
                .all(|l| l.parse::<iglu_domain::label::DnsLabel>().is_ok()),
            "preview_domain must be a lowercase domain name"
        );
        Ok(config)
    }

    /// The console's origin as browsers serialize it in `Origin` headers.
    pub fn console_origin_string(&self) -> String {
        self.console_origin.origin().ascii_serialization()
    }

    pub fn preview_origin(&self, label: &str) -> String {
        format!("https://{label}.{}", self.preview_domain)
    }
}
