//! Building environment images with the host's Nix daemon and importing them
//! into Incus.
//!
//! Evaluation runs as an unprivileged build account; the daemon sandboxes the
//! builds. The image output is a directory with `metadata.tar.xz`,
//! `rootfs.squashfs` and `iglu.json`, produced by the workspace module.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use hyper::Method;
use iglu_domain::env::{Arch, BuiltImage, EnvSource, GuestUser, ImageFingerprint};
use iglu_domain::guest::{self, INTERFACE, Incompatible, Interface};
use iglu_domain::secret::FetchTokens;
use iglu_domain::time::Timestamp;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::client::{Incus, IncusError};
use crate::config;

/// The interface alone, read before the rest of `iglu.json`: an image from
/// another iglu may say what this one doesn't know, and should be told to
/// update rather than that.
#[derive(Deserialize)]
struct Speaks {
    /// The guest-tools interface the image's tools speak. Images from
    /// before it was recorded don't say.
    #[serde(default)]
    guest_interface: Option<Interface>,
}

/// `iglu.json` in the image output, written by the workspace module.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageManifest {
    arch: Arch,
    user: GuestUser,
    /// Checked through [`Speaks`] before the rest is read; here so it isn't
    /// an unknown field.
    #[serde(default)]
    #[expect(dead_code, reason = "read through Speaks")]
    guest_interface: Option<Interface>,
    /// Whether the environment has the browser, `iglu.browser.enable`.
    #[serde(default)]
    browser: bool,
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
    // The build's output link keeps it from the host's store's collection
    // until its image keeps it, or it fails.
    static BUILDS: AtomicU64 = AtomicU64::new(0);
    let link = build.home.join(format!(
        "iglu-build-{}",
        BUILDS.fetch_add(1, Ordering::Relaxed)
    ));
    let store_path = nix_build(build, source, tokens, &link).await?;
    let imported = import_output(incus, settings, build, &store_path, timeout).await;
    let _ = tokio::fs::remove_file(&link).await;
    imported
}

async fn import_output(
    incus: &Incus,
    settings: &config::Incus,
    build: &config::Build,
    store_path: &Path,
    timeout: Duration,
) -> Result<BuiltImage, BuildError> {
    let manifest_bytes = tokio::fs::read(store_path.join("iglu.json"))
        .await
        .map_err(|e| BuildError::BadOutput(format!("iglu.json: {e}")))?;
    let speaks: Speaks = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| BuildError::BadOutput(format!("iglu.json: {e}")))?;
    guest::compatible(speaks.guest_interface)?;
    let manifest: ImageManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| BuildError::BadOutput(format!("iglu.json: {e}")))?;
    let metadata = store_path.join("metadata.tar.xz");
    let rootfs = store_path.join("rootfs.squashfs");
    let fingerprint = fingerprint(&metadata, &rootfs).await?;
    let system = tokio::fs::read_link(store_path.join("system"))
        .await
        .map_err(|e| BuildError::BadOutput(format!("system: {e}")))?;
    import(incus, settings, &fingerprint, &metadata, &rootfs).await?;
    keep(&fingerprint, store_path).await?;
    record(incus, &fingerprint, &system, timeout).await?;
    warm(
        incus,
        settings,
        &fingerprint,
        Duration::from_secs(build.timeout_secs),
    )
    .await?;
    Ok(BuiltImage {
        fingerprint,
        arch: manifest.arch,
        user: manifest.user,
        browser: manifest.browser,
        store_path: store_path.display().to_string(),
    })
}

async fn nix_build(
    build: &config::Build,
    source: &EnvSource,
    tokens: &FetchTokens,
    link: &Path,
) -> Result<PathBuf, BuildError> {
    let mut command = Command::new(&build.nix);
    command
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "build",
            "--print-out-paths",
            "--out-link",
        ])
        .arg(link)
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

/// Unpacks the image onto the workspaces' storage pool now, so the first
/// workspace made from it doesn't wait for it: on a slow disk that takes
/// minutes. Incus unpacks an image when the first instance is made from it
/// and keeps the result for later ones, and that's the only way to ask for
/// it, so a stopped instance is made and deleted. Its name isn't a
/// workspace's, so nothing claims it.
async fn warm(
    incus: &Incus,
    settings: &config::Incus,
    fingerprint: &ImageFingerprint,
    timeout: Duration,
) -> Result<(), BuildError> {
    let id = fingerprint.to_string();
    let name = format!("iglu-warm-{}", &id[..16]);
    let path = format!("/1.0/instances/{name}");
    let failed = |e: IncusError| BuildError::Import(format!("unpacking the image: {e}"));
    // One a build cut short left behind.
    match incus.run(Method::DELETE, &path, None, timeout).await {
        Ok(_) => {}
        Err(error) if error.is_not_found() => {}
        Err(error) => return Err(failed(error)),
    }
    let body = serde_json::json!({
        "name": name,
        "type": "container",
        "source": { "type": "image", "fingerprint": id },
        // The workspaces' profile, for their root disk's pool.
        "profiles": [settings.profile],
        "start": false,
    });
    incus
        .run(Method::POST, "/1.0/instances", Some(&body), timeout)
        .await
        .map_err(failed)?;
    incus
        .run(Method::DELETE, &path, None, timeout)
        .await
        .map_err(failed)?;
    Ok(())
}

/// The image property recording which guest-tools interface an image's
/// tools speak, so creating from an image can check it. Every image hostd
/// built has it.
pub const INTERFACE_PROPERTY: &str = "iglu.guest-interface";

/// The image property recording the NixOS system the image is of, which a
/// workspace from an older build upgrades to in place.
pub const SYSTEM_PROPERTY: &str = "iglu.system";

/// Where hostd keeps each image's build alive in the host's store, so its
/// system is there to copy into workspaces while the image is: Nix's own
/// record, not hostd's.
const ROOTS: &str = "/nix/var/nix/gcroots/iglu";

fn root_of(fingerprint: &ImageFingerprint) -> PathBuf {
    Path::new(ROOTS).join(fingerprint.as_str())
}

/// Lets the host's store collect an image's build, once the image is gone.
pub async fn release(fingerprint: &ImageFingerprint) {
    let _ = tokio::fs::remove_file(root_of(fingerprint)).await;
}

/// The system an imported image records, if it records one.
pub fn recorded_system(image: &serde_json::Value) -> Option<String> {
    image["properties"][SYSTEM_PROPERTY]
        .as_str()
        .filter(|path| path.starts_with("/nix/store/"))
        .map(str::to_owned)
}

/// The image property recording when hostd built the image, in milliseconds
/// since the Unix epoch.
pub const BUILT_PROPERTY: &str = "iglu.built-at";

/// When an imported image was built, if it records it.
pub fn recorded_build(image: &serde_json::Value) -> Option<Timestamp> {
    image["properties"][BUILT_PROPERTY]
        .as_str()?
        .parse()
        .ok()
        .map(Timestamp::from_unix_millis)
}

/// The interface an imported image records, if it records one.
pub fn recorded_interface(image: &serde_json::Value) -> Option<Interface> {
    image["properties"][INTERFACE_PROPERTY]
        .as_str()?
        .parse()
        .ok()
        .map(Interface::new)
}

/// Keeps the build in the host's store while its image is on the host.
async fn keep(fingerprint: &ImageFingerprint, store_path: &Path) -> Result<(), BuildError> {
    let root = root_of(fingerprint);
    tokio::fs::create_dir_all(ROOTS)
        .await
        .map_err(|e| BuildError::Import(format!("keeping the build: {e}")))?;
    let _ = tokio::fs::remove_file(&root).await;
    tokio::fs::symlink(store_path, &root)
        .await
        .map_err(|e| BuildError::Import(format!("keeping the build: {e}")))
}

/// Records on the image that its tools speak this host's interface, which
/// the build just checked, and the system it is of.
async fn record(
    incus: &Incus,
    fingerprint: &ImageFingerprint,
    system: &Path,
    timeout: Duration,
) -> Result<(), BuildError> {
    let path = format!("/1.0/images/{fingerprint}");
    let mut image: serde_json::Value = incus
        .get(&path)
        .await
        .map_err(|e| BuildError::Import(e.to_string()))?;
    image["properties"][INTERFACE_PROPERTY] = serde_json::json!(INTERFACE.to_string());
    let built = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    image["properties"][BUILT_PROPERTY] = serde_json::json!(built.to_string());
    image["properties"][SYSTEM_PROPERTY] = serde_json::json!(system.display().to_string());
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
