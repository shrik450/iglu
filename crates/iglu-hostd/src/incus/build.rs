//! Building environment images with the host's Nix daemon and importing them
//! into Incus.
//!
//! Evaluation runs as an unprivileged build account; the daemon sandboxes the
//! builds. The image output is a directory with `metadata.tar.xz`,
//! `rootfs.squashfs` and `iglu.json`, produced by the workspace module.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use hyper::Method;
use iglu_domain::env::{Arch, BuiltImage, EnvSource, GuestUser, ImageFingerprint};
use iglu_domain::guest::{self, INTERFACE, Incompatible, Interface};
use iglu_domain::secret::FetchTokens;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::client::Incus;
use crate::config;

/// `iglu.json` in the image output, written by the workspace module.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageManifest {
    arch: Arch,
    user: GuestUser,
    /// The guest-tools interface the image's tools speak. Images from
    /// before it was recorded don't say.
    #[serde(default)]
    guest_interface: Option<Interface>,
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("{0}")]
    Failed(String),
    #[error("the build produced an unusable image: {0}")]
    BadOutput(String),
    #[error(transparent)]
    Incompatible(#[from] Incompatible),
    #[error("importing the image failed: {0}")]
    Import(String),
}

pub async fn build(
    incus: &Incus,
    settings: &config::Incus,
    build: &config::Build,
    source: &EnvSource,
    tokens: &FetchTokens,
    timeout: Duration,
) -> Result<BuiltImage, BuildError> {
    let store_path = nix_build(build, source, tokens).await?;
    let manifest_bytes = tokio::fs::read(store_path.join("iglu.json"))
        .await
        .map_err(|e| BuildError::BadOutput(format!("iglu.json: {e}")))?;
    let manifest: ImageManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| BuildError::BadOutput(format!("iglu.json: {e}")))?;
    guest::compatible(manifest.guest_interface)?;
    let metadata = store_path.join("metadata.tar.xz");
    let rootfs = store_path.join("rootfs.squashfs");
    let fingerprint = fingerprint(&metadata, &rootfs).await?;
    import(incus, settings, &fingerprint, &metadata, &rootfs).await?;
    record_interface(incus, &fingerprint, timeout).await?;
    Ok(BuiltImage {
        fingerprint,
        arch: manifest.arch,
        user: manifest.user,
        store_path: store_path.display().to_string(),
    })
}

async fn nix_build(
    build: &config::Build,
    source: &EnvSource,
    tokens: &FetchTokens,
) -> Result<PathBuf, BuildError> {
    let mut command = Command::new(&build.nix);
    command
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "build",
            "--no-link",
            "--print-out-paths",
        ])
        .arg(source.image_installable())
        .env_clear()
        .env("HOME", &build.home)
        .env("PATH", "/run/current-system/sw/bin")
        .env("NIX_REMOTE", "daemon")
        .uid(build.uid)
        .gid(build.gid)
        .current_dir(&build.home)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    if let Ok(certs) = std::env::var("SSL_CERT_FILE") {
        command.env("SSL_CERT_FILE", certs);
    }
    // In the environment rather than the arguments, which anyone on the host
    // can read, and only for this build, so nothing is left on disk. The
    // build account is shared, though: what one owner's token fetches lands
    // in the host's store and Nix's fetch cache, where other owners' builds
    // could reuse it.
    if let Some(setting) = tokens.nix_setting() {
        command.env("NIX_CONFIG", setting);
    }
    let output = tokio::time::timeout(Duration::from_secs(build.timeout_secs), command.output())
        .await
        .map_err(|_| {
            BuildError::Failed(format!(
                "the build took longer than {}s",
                build.timeout_secs
            ))
        })?
        .map_err(|e| BuildError::Failed(format!("couldn't run nix: {e}")))?;
    if !output.status.success() {
        return Err(BuildError::Failed(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let path = stdout
        .lines()
        .rev()
        .find(|line| line.starts_with("/nix/store/"))
        .ok_or_else(|| BuildError::BadOutput("nix printed no output path".into()))?;
    Ok(PathBuf::from(path.trim()))
}

/// Incus fingerprints a split image as the SHA-256 of its metadata tarball
/// followed by its rootfs.
async fn fingerprint(metadata: &Path, rootfs: &Path) -> Result<ImageFingerprint, BuildError> {
    let mut hasher = Sha256::new();
    for path in [metadata, rootfs] {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| BuildError::BadOutput(format!("{}: {e}", path.display())))?;
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let read = file
                .read(&mut buffer)
                .await
                .map_err(|e| BuildError::BadOutput(e.to_string()))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }
    hex::encode(hasher.finalize())
        .parse()
        .map_err(|_| BuildError::BadOutput("fingerprint".into()))
}

async fn import(
    incus: &Incus,
    settings: &config::Incus,
    fingerprint: &ImageFingerprint,
    metadata: &Path,
    rootfs: &Path,
) -> Result<(), BuildError> {
    match incus
        .get::<serde_json::Value>(&format!("/1.0/images/{fingerprint}"))
        .await
    {
        Ok(_) => return Ok(()),
        Err(error) if error.is_not_found() => {}
        Err(error) => return Err(BuildError::Import(error.to_string())),
    }
    let mut command = Command::new(&settings.binary);
    command.args(["image", "import"]).arg(metadata).arg(rootfs);
    if let Some(project) = &settings.project {
        command.args(["--project", project]);
    }
    let output = command
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| BuildError::Import(e.to_string()))?;
    if !output.status.success() {
        return Err(BuildError::Import(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    incus
        .get::<serde_json::Value>(&format!("/1.0/images/{fingerprint}"))
        .await
        .map(|_| ())
        .map_err(|e| {
            BuildError::Import(format!(
                "imported, but not under the expected fingerprint: {e}"
            ))
        })
}

/// The image property recording which guest-tools interface an image's
/// tools speak, so creating from an image can check it.
pub const INTERFACE_PROPERTY: &str = "iglu.guest-interface";

/// The interface an imported image records, if it records one.
pub fn recorded_interface(image: &serde_json::Value) -> Option<Interface> {
    image["properties"][INTERFACE_PROPERTY]
        .as_str()?
        .parse()
        .ok()
        .map(Interface::new)
}

/// Records on the image that its tools speak this host's interface, which
/// the build just checked.
async fn record_interface(
    incus: &Incus,
    fingerprint: &ImageFingerprint,
    timeout: Duration,
) -> Result<(), BuildError> {
    let path = format!("/1.0/images/{fingerprint}");
    let mut image: serde_json::Value = incus
        .get(&path)
        .await
        .map_err(|e| BuildError::Import(e.to_string()))?;
    image["properties"][INTERFACE_PROPERTY] = serde_json::json!(INTERFACE.to_string());
    let put = serde_json::json!({
        "auto_update": image["auto_update"],
        "expires_at": image["expires_at"],
        "profiles": image["profiles"],
        "properties": image["properties"],
        "public": image["public"],
    });
    incus
        .run(Method::PUT, &path, Some(&put), timeout)
        .await
        .map(|_| ())
        .map_err(|e| BuildError::Import(format!("recording the image's interface: {e}")))
}
