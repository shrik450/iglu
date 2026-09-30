//! Moving a frozen workspace's memory to swap, so frozen workspaces are cheap.
//!
//! Incus has no API for this, so hostd writes the cgroup's `memory.reclaim`
//! directly. The path follows Incus's cgroup layout for containers.

use iglu_domain::id::InstanceName;

fn cgroup(name: InstanceName) -> String {
    format!("/sys/fs/cgroup/lxc.payload.{name}")
}

/// Asks the kernel to reclaim everything the frozen cgroup holds. Partial
/// reclaim is normal (the kernel returns `EAGAIN`), and a failure only means
/// the workspace stays in RAM, so this never fails the freeze.
pub async fn reclaim(name: InstanceName) {
    let dir = cgroup(name);
    let current = match tokio::fs::read_to_string(format!("{dir}/memory.current")).await {
        Ok(text) => text.trim().to_owned(),
        Err(error) => {
            tracing::warn!(%name, %error, "couldn't read memory usage; skipping reclaim");
            return;
        }
    };
    let target = format!("{dir}/memory.reclaim");
    // `swappiness=` prefers swapping anonymous memory over dropping file
    // cache; kernels without it reject the argument, so retry plainly.
    let mut result = tokio::fs::write(&target, format!("{current} swappiness=200\n")).await;
    if result.as_ref().is_err_and(|e| e.raw_os_error() == Some(22)) {
        result = tokio::fs::write(&target, format!("{current}\n")).await;
    }
    match result {
        Ok(()) => tracing::info!(%name, bytes = %current, "reclaimed a frozen workspace's memory"),
        Err(error) if error.raw_os_error() == Some(11) => {
            tracing::info!(%name, "reclaimed part of a frozen workspace's memory");
        }
        Err(error) => tracing::warn!(%name, %error, "memory reclaim failed"),
    }
}
