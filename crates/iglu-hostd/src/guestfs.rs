//! Reading small files from inside a running guest without entering it.
//!
//! Guest files are read through `/proc/<init pid>/root`, resolved with
//! `RESOLVE_IN_ROOT` so symlinks planted by the guest can't point hostd at
//! host files. This avoids forking into the container on every poll.

/// Files the platform module and the guest tools agree on.
pub mod paths {
    /// Written by `iglu-ready.service` once the system finished booting.
    pub const READY: &str = "run/iglu/ready";
    /// The secrets delivery the guest holds, written last by `iglu-guest install-secrets`.
    pub const SECRETS_GENERATION: &str = "run/iglu/secrets/generation";
    /// Per-session attention status, maintained by `iglu-status`.
    pub const STATUS: &str = "run/iglu/status/sessions.json";
    /// Where hostd drops a secrets bundle for the guest tool to install.
    pub const SECRETS_INCOMING: &str = "/run/iglu/secrets/.incoming.json";
    /// The guest tool, installed by the platform module.
    pub const GUEST_TOOL: &str = "/run/current-system/sw/bin/iglu-guest";
}

/// Largest guest file hostd will read.
const MAX_FILE: u64 = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum GuestFileError {
    #[error("reading guest file: {0}")]
    Io(#[from] std::io::Error),
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    #[error("guest file is larger than {MAX_FILE} bytes")]
    TooLarge,
}

/// Reads `relative` (no leading slash) from the guest whose init has `pid`.
/// Returns `None` when the file doesn't exist.
pub async fn read(pid: i64, relative: &'static str) -> Result<Option<Vec<u8>>, GuestFileError> {
    tokio::task::spawn_blocking(move || read_blocking(pid, relative))
        .await
        .map_err(|e| GuestFileError::Io(std::io::Error::other(e)))?
}

#[cfg(target_os = "linux")]
fn read_blocking(pid: i64, relative: &str) -> Result<Option<Vec<u8>>, GuestFileError> {
    use std::io::Read;
    use std::path::Path;

    use rustix::fs::{Mode, OFlags, ResolveFlags};

    let root_path = format!("/proc/{pid}/root");
    let root = match rustix::fs::open(
        Path::new(&root_path),
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(errno) => return Err(std::io::Error::from(errno).into()),
    };
    let fd = match rustix::fs::openat2(
        &root,
        relative,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        Mode::empty(),
        ResolveFlags::IN_ROOT | ResolveFlags::NO_MAGICLINKS,
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(errno) => return Err(std::io::Error::from(errno).into()),
    };
    let file = std::fs::File::from(fd);
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Ok(None);
    }
    if metadata.len() > MAX_FILE {
        return Err(GuestFileError::TooLarge);
    }
    let mut content = Vec::new();
    file.take(MAX_FILE).read_to_end(&mut content)?;
    Ok(Some(content))
}

#[cfg(not(target_os = "linux"))]
fn read_blocking(_pid: i64, _relative: &str) -> Result<Option<Vec<u8>>, GuestFileError> {
    Err(std::io::Error::other("guest files can only be read on Linux").into())
}
