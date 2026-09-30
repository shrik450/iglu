//! Attention status: `iglu-status` records what each terminal session is
//! doing, and hostd reads the file.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use iglu_domain::attention::{AttentionState, Summary};
use iglu_domain::terminal::SessionName;
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub state: AttentionState,
    pub summary: Summary,
    pub at: i64,
}

pub type Statuses = BTreeMap<SessionName, Entry>;

/// Sets or clears one session's entry.
#[must_use]
pub fn update(mut statuses: Statuses, session: SessionName, entry: Option<Entry>) -> Statuses {
    match entry {
        Some(entry) => {
            statuses.insert(session, entry);
        }
        None => {
            statuses.remove(&session);
        }
    }
    statuses
}

#[derive(Debug, thiserror::Error)]
pub enum StatusError {
    #[error("status file: {0}")]
    Io(#[from] std::io::Error),
    #[error("status file: {0}")]
    Json(#[from] serde_json::Error),
}

/// Applies `change` to the status file under an exclusive lock, replacing
/// the file atomically so hostd never reads a partial write.
///
/// # Errors
///
/// When the lock or the file can't be read or written.
pub fn modify(change: impl FnOnce(Statuses) -> Statuses) -> Result<(), StatusError> {
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths::STATUS_LOCK)?;
    lock.lock()?;
    let current: Statuses = match fs::read(paths::STATUS_FILE) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Statuses::new(),
        Err(error) => return Err(error.into()),
    };
    let next = change(current);
    write_atomic(Path::new(paths::STATUS_FILE), &serde_json::to_vec(&next)?)?;
    lock.unlock()?;
    Ok(())
}

/// Writes `content` beside `path` and renames it into place.
///
/// # Errors
///
/// When writing, syncing or renaming fails.
pub fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension("tmp");
    let mut file = File::create(&temp)?;
    file.write_all(content)?;
    file.sync_all()?;
    fs::rename(temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(state: AttentionState) -> Entry {
        Entry {
            state,
            summary: Summary::sanitize(""),
            at: 1,
        }
    }

    #[test]
    fn update_sets_and_clears() {
        let t1: SessionName = "t1".parse().expect("valid");
        let set = update(
            Statuses::new(),
            t1.clone(),
            Some(entry(AttentionState::Working)),
        );
        assert_eq!(set.get(&t1).map(|e| e.state), Some(AttentionState::Working));
        assert!(update(set, t1, None).is_empty());
    }
}
