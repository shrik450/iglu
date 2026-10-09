//! iglud's configuration, written by the NixOS module as JSON.

use std::net::SocketAddr;
use std::num::{NonZeroU32, NonZeroUsize};
use std::path::{Path, PathBuf};

use iglu_domain::auth::{Issuer, SignInPolicy};
use iglu_domain::capacity::Bytes;
use iglu_domain::idle::IdlePolicy;
use iglu_domain::label::HostId;
use iglu_domain::time::Millis;
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
    #[serde(default)]
    pub idle: IdleSettings,
    /// Copies of the database; none when absent.
    #[serde(default)]
    pub backups: Option<Backups>,
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
    /// An RFC 8707 resource to ask for. Without one, the identity provider
    /// picks the audience, usually the worker's own client ID; hosts accept
    /// only the audience in their `auth.audience`.
    #[serde(default)]
    pub resource: Option<Url>,
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
    pub url: HostUrl,
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
pub struct IdleSettings {
    /// Minutes unused before a running workspace freezes; `None` never.
    pub freeze_after_minutes: Option<u32>,
    /// Minutes unused before a frozen workspace stops; `None` never.
    pub stop_after_minutes: Option<u32>,
}

impl Default for IdleSettings {
    fn default() -> Self {
        Self {
            freeze_after_minutes: Some(120),
            stop_after_minutes: None,
        }
    }
}

impl IdleSettings {
    pub fn policy(&self) -> IdlePolicy {
        let minutes = |m: u32| Millis::from_secs(m.saturating_mul(60));
        IdlePolicy {
            freeze_after: self.freeze_after_minutes.map(minutes),
            stop_after: self.stop_after_minutes.map(minutes),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
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

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backups {
    pub directory: PathBuf,
    /// Minutes between copies. One is also taken at startup, before migrating.
    pub interval_minutes: NonZeroU32,
    /// How many copies to keep, newest first.
    pub keep: NonZeroUsize,
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

/// Where a host serves hostd. iglud sends it service tokens and secrets, so
/// it's HTTPS, or plain HTTP only to this machine's loopback, where a dev
/// host listens.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct HostUrl(Url);

impl HostUrl {
    pub fn as_url(&self) -> &Url {
        &self.0
    }
}

impl TryFrom<String> for HostUrl {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let url = Url::parse(&value).map_err(|e| format!("{value}: {e}"))?;
        let loopback = match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(_)) | None => false,
        };
        match url.scheme() {
            "https" => Ok(Self(url)),
            "http" if loopback => Ok(Self(url)),
            _ => Err(format!(
                "{value}: a host must be https://, or http:// to a loopback address such as 127.0.0.1"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_https_or_plain_http_on_loopback_only() {
        for good in [
            "https://host-1.lan:7443",
            "https://10.0.0.5:7443",
            "http://127.0.0.1:7200",
            "http://[::1]:7200",
        ] {
            assert!(HostUrl::try_from(good.to_owned()).is_ok(), "{good}");
        }
        for bad in [
            "http://10.0.0.5:7200",
            "http://host-1.lan:7200",
            "http://localhost:7200",
            "ftp://127.0.0.1",
            "not a url",
        ] {
            assert!(HostUrl::try_from(bad.to_owned()).is_err(), "{bad}");
        }
    }
}
