//! Upgrading a workspace's NixOS in place, as `nixos-rebuild` would: the new
//! system's store paths the guest lacks are copied from the host's store
//! into the guest's, the system profile is pointed at it, and its
//! `switch-to-configuration boot` makes it the system the container boots.
//! Everything else on the guest's disk stays.
//!
//! These are the only programs hostd runs in a guest as root: the guest
//! system's own Nix tools and the new system's activation, with store paths
//! hostd chose. Nothing the workspace's user can change is run.

use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use iglu_domain::id::InstanceName;
use tokio::process::Command;

use super::client::Incus;
use super::exec::{self, Program};
use crate::config;
use crate::runtime::RuntimeError;

/// The guest's system profile, which the container boots the newest of.
const PROFILE: &str = "/nix/var/nix/profiles/system";

/// The host's `nix-store`, beside the `nix` builds use.
fn host_nix_store(build: &config::Build) -> std::path::PathBuf {
    build.nix.with_file_name("nix-store")
}

async fn run_root(
    incus: &Incus,
    name: InstanceName,
    argv: &[&str],
    timeout: Duration,
) -> Result<String, RuntimeError> {
    let output = exec::capture(
        incus,
        name,
        &Program::system(argv.iter().copied()),
        None,
        timeout,
    )
    .await
    .map_err(RuntimeError::failed)?;
    if !output.success {
        return Err(RuntimeError::failed(format!(
            "{} failed: {}",
            argv.join(" "),
            output.stderr.trim()
        )));
    }
    Ok(output.stdout)
}

/// Waits until the guest runs programs, as it does as soon as the container
/// has started.
pub async fn ready(
    incus: &Incus,
    name: InstanceName,
    within: Duration,
) -> Result<(), RuntimeError> {
    let deadline = Instant::now() + within;
    loop {
        match run_root(incus, name, &["true"], Duration::from_secs(10)).await {
            Ok(_) => return Ok(()),
            Err(error) if Instant::now() > deadline => return Err(error),
            Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}

/// Puts `system` in the guest's store and makes it the system it boots.
pub async fn install(
    incus: &Incus,
    name: InstanceName,
    build: &config::Build,
    system: &str,
    timeout: Duration,
) -> Result<(), RuntimeError> {
    let closure = Command::new(host_nix_store(build))
        .args(["--query", "--requisites", system])
        .env("NIX_REMOTE", "daemon")
        .output()
        .await
        .map_err(RuntimeError::failed)?;
    if !closure.status.success() {
        return Err(RuntimeError::failed(format!(
            "listing the system's store paths: {}",
            String::from_utf8_lossy(&closure.stderr).trim()
        )));
    }
    let closure: Vec<String> = String::from_utf8_lossy(&closure.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    let mut check = vec!["nix-store", "--check-validity", "--print-invalid"];
    check.extend(closure.iter().map(String::as_str));
    // Its answer is what it prints, whatever it exits with.
    let missing: Vec<String> = exec::capture(incus, name, &Program::system(check), None, timeout)
        .await
        .map_err(RuntimeError::failed)?
        .stdout
        .lines()
        .filter(|line| closure.iter().any(|path| path == line))
        .map(str::to_owned)
        .collect();
    if !missing.is_empty() {
        let mut export = Command::new(host_nix_store(build))
            .arg("--export")
            .args(&missing)
            .env("NIX_REMOTE", "daemon")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(RuntimeError::failed)?;
        let stdout = export
            .stdout
            .take()
            .ok_or_else(|| RuntimeError::failed("nix-store --export has no output"))?;
        let imported = exec::capture_from(
            incus,
            name,
            &Program::system(["nix-store", "--import"]),
            stdout,
            timeout,
        )
        .await
        .map_err(RuntimeError::failed)?;
        let exported = export.wait().await.map_err(RuntimeError::failed)?;
        if !exported.success() || !imported.success {
            return Err(RuntimeError::failed(format!(
                "copying the system into the workspace failed: {}",
                imported.stderr.trim()
            )));
        }
    }
    run_root(
        incus,
        name,
        &["nix-env", "--profile", PROFILE, "--set", system],
        timeout,
    )
    .await?;
    let switch = format!("{system}/bin/switch-to-configuration");
    run_root(incus, name, &[&switch, "boot"], timeout).await?;
    Ok(())
}

/// Once the guest runs `system`, removes older systems from its profile and
/// deletes what only `old` kept: what the person keeps, their own builds
/// included, stays. Best done; a failure leaves only disk used.
pub async fn tidy(
    incus: &Incus,
    name: InstanceName,
    system: &str,
    old: &str,
    within: Duration,
) -> Result<(), RuntimeError> {
    let deadline = Instant::now() + within;
    loop {
        let current = run_root(
            incus,
            name,
            &["readlink", "/run/current-system"],
            Duration::from_secs(10),
        )
        .await;
        if current.as_deref().map(str::trim) == Ok(system) {
            break;
        }
        if Instant::now() > deadline {
            return Err(RuntimeError::failed(
                "the workspace never booted its new system",
            ));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let timeout = within;
    run_root(
        incus,
        name,
        &[
            "nix-env",
            "--profile",
            PROFILE,
            "--delete-generations",
            "old",
        ],
        timeout,
    )
    .await?;
    if !Path::new(old).is_absolute() {
        return Ok(());
    }
    let old_closure: HashSet<String> = run_root(
        incus,
        name,
        &["nix-store", "--query", "--requisites", old],
        timeout,
    )
    .await?
    .lines()
    .map(str::to_owned)
    .collect();
    // What's printed may be cut short; whatever of the old system isn't
    // listed is left for the next upgrade.
    let doomed: Vec<String> =
        run_root(incus, name, &["nix-store", "--gc", "--print-dead"], timeout)
            .await?
            .lines()
            .filter(|path| old_closure.contains(*path))
            .map(str::to_owned)
            .collect();
    if doomed.is_empty() {
        return Ok(());
    }
    let mut delete = vec!["nix-store", "--delete"];
    delete.extend(doomed.iter().map(String::as_str));
    run_root(incus, name, &delete, timeout).await.map(|_| ())
}
