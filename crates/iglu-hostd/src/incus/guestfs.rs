//! Reading small files from inside a running guest without entering it.
//!
//! Guest files are read through `/proc/<init pid>/root`, resolved with
//! `RESOLVE_IN_ROOT` so symlinks planted by the guest can't point hostd at
//! host files. This avoids forking into the container on every poll.

use crate::runtime::MAX_GUEST_FILE;

/// Files the platform module and the guest tools agree on.
pub mod paths {
    use iglu_domain::env::GuestUser;

    /// Written by `iglu-ready.service` once the system finished booting.
    pub const READY: &str = "run/iglu/ready";
    /// The guest tool, installed by the platform module.
    pub const GUEST_TOOL: &str = "/run/current-system/sw/bin/iglu-guest";

    /// The user's runtime directory, which systemd-logind creates at boot
    /// for the lingering workspace user.
    pub fn runtime_dir(user: &GuestUser) -> String {
        format!("/run/user/{}", user.uid.get())
    }

    /// One of the guest tools' files under the runtime directory, relative
    /// to the guest's root.
    pub fn in_runtime_dir(user: &GuestUser, file: &str) -> String {
        format!("{}/{file}", runtime_dir(user).trim_start_matches('/'))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GuestFileError {
    #[error("reading guest file: {0}")]
    Io(#[from] std::io::Error),
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    #[error("guest file is larger than {MAX_GUEST_FILE} bytes")]
    TooLarge,
}

/// Reads `relative` (no leading slash) from the guest whose init has `pid`.
/// Returns `None` when the file doesn't exist.
pub async fn read(pid: u64, relative: String) -> Result<Option<Vec<u8>>, GuestFileError> {
    tokio::task::spawn_blocking(move || read_blocking(pid, &relative))
        .await
        .map_err(|e| GuestFileError::Io(std::io::Error::other(e)))?
}

/// Opens `relative` (no leading slash) in the guest whose init has `pid`,
/// of any size, through no link at all, so the guest can't point hostd at
/// another of its files either. Returns `None` when there's no such
/// regular file.
pub async fn open(pid: u64, relative: String) -> Result<Option<std::fs::File>, GuestFileError> {
    tokio::task::spawn_blocking(move || open_blocking(pid, &relative, true))
        .await
        .map_err(|e| GuestFileError::Io(std::io::Error::other(e)))?
}

#[cfg(target_os = "linux")]
fn read_blocking(pid: u64, relative: &str) -> Result<Option<Vec<u8>>, GuestFileError> {
    use std::io::Read;

    let Some(file) = open_blocking(pid, relative, false)? else {
        return Ok(None);
    };
    if file.metadata()?.len() > MAX_GUEST_FILE {
        return Err(GuestFileError::TooLarge);
    }
    let mut content = Vec::new();
    file.take(MAX_GUEST_FILE).read_to_end(&mut content)?;
    Ok(Some(content))
}

#[cfg(target_os = "linux")]
fn open_blocking(
    pid: u64,
    relative: &str,
    no_links: bool,
) -> Result<Option<std::fs::File>, GuestFileError> {
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
    let mut resolve = ResolveFlags::IN_ROOT | ResolveFlags::NO_MAGICLINKS;
    if no_links {
        resolve |= ResolveFlags::NO_SYMLINKS;
    }
    let fd = match rustix::fs::openat2(
        &root,
        relative,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        Mode::empty(),
        resolve,
    ) {
        Ok(fd) => fd,
        // A link where none may be is no such file.
        Err(rustix::io::Errno::NOENT | rustix::io::Errno::LOOP) => return Ok(None),
        Err(errno) => return Err(std::io::Error::from(errno).into()),
    };
    let file = std::fs::File::from(fd);
    if !file.metadata()?.is_file() {
        return Ok(None);
    }
    Ok(Some(file))
}

#[cfg(not(target_os = "linux"))]
fn read_blocking(_pid: u64, _relative: &str) -> Result<Option<Vec<u8>>, GuestFileError> {
    Err(std::io::Error::other("guest files can only be read on Linux").into())
}

#[cfg(not(target_os = "linux"))]
fn open_blocking(
    _pid: u64,
    _relative: &str,
    _no_links: bool,
) -> Result<Option<std::fs::File>, GuestFileError> {
    Err(std::io::Error::other("guest files can only be read on Linux").into())
}
