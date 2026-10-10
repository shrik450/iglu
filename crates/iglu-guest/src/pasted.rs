//! Keeping the files a person pastes or drops on a terminal, so the path
//! pasted in their place leads somewhere.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use iglu_domain::pasted::FileName;

use crate::paths::Dirs;

/// How long a pasted file is kept. By then whatever it was pasted into has
/// long read it.
const KEPT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Where pasted files go: under the home directory, so they last across a
/// restart and don't take memory.
#[must_use]
pub fn dir(dirs: &Dirs) -> PathBuf {
    dirs.home().join(".cache/iglu/pasted")
}

/// Keeps `bytes` as a new file named `name`, numbered if one of that name
/// is there already, readable only by the user. Files older than a week go.
///
/// # Errors
///
/// When the directory or file can't be written.
pub fn keep(dirs: &Dirs, name: &FileName, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = dir(dirs);
    fs::create_dir_all(&dir)?;
    forget_old(&dir, SystemTime::now());
    for n in 1..=1000 {
        let path = dir.join(name.numbered(n));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(bytes)?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other(format!(
        "too many files called {name}"
    )))
}

fn forget_old(dir: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|modified| now.duration_since(modified).is_ok_and(|age| age > KEPT));
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}
