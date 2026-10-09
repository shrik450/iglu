//! Attention status: `iglu-status` records what each thread in each
//! terminal session is doing, and hostd reads the file.

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, File};
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;

use iglu_domain::attention::{self, AttentionState, Summary, ThreadKey};
pub use iglu_domain::guest::StatusEntry;
use iglu_domain::terminal::SessionName;
use iglu_domain::time::Timestamp;

use crate::paths::Dirs;

pub type Threads = BTreeMap<ThreadKey, StatusEntry>;
pub type Statuses = BTreeMap<SessionName, Threads>;

/// What a session says about one of its threads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    Set {
        state: AttentionState,
        summary: Summary,
        /// Becomes the thread's title if it has none yet.
        title: Option<Summary>,
        at: i64,
    },
    Clear,
}

/// Now, for a report's `at`. Zero if the clock is before 1970.
#[must_use]
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Applies a report to one thread, then keeps only the session's threads
/// the core says to.
#[must_use]
pub fn update(
    mut statuses: Statuses,
    session: SessionName,
    thread: ThreadKey,
    report: Report,
) -> Statuses {
    let mut threads = statuses.remove(&session).unwrap_or_default();
    match report {
        Report::Set {
            state,
            summary,
            title,
            at,
        } => {
            let title = threads
                .get(&thread)
                .map(|entry| entry.title.clone())
                .filter(|kept| !kept.as_str().is_empty())
                .or(title)
                .unwrap_or_else(|| Summary::sanitize(""));
            threads.insert(
                thread,
                StatusEntry {
                    state,
                    summary,
                    title,
                    at,
                },
            );
        }
        Report::Clear => {
            threads.remove(&thread);
        }
    }
    let listed: Vec<(ThreadKey, AttentionState, Timestamp)> = threads
        .iter()
        .map(|(key, entry)| {
            (
                key.clone(),
                entry.state,
                Timestamp::from_unix_millis(entry.at),
            )
        })
        .collect();
    let kept = attention::keep(&listed);
    threads.retain(|key, _| kept.contains(key));
    if !threads.is_empty() {
        statuses.insert(session, threads);
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
/// the file atomically so hosts never read a partial write.
///
/// # Errors
///
/// When the lock or the file can't be read or written.
pub fn modify(dirs: &Dirs, change: impl FnOnce(Statuses) -> Statuses) -> Result<(), StatusError> {
    let file = dirs.status_file();
    if let Some(dir) = file.parent() {
        DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dirs.status_lock())?;
    lock.lock()?;
    let current: Statuses = match fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Statuses::new(),
        Err(error) => return Err(error.into()),
    };
    let next = change(current);
    write_atomic(&file, &serde_json::to_vec(&next)?)?;
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

    fn set(state: AttentionState, title: &str, at: i64) -> Report {
        Report::Set {
            state,
            summary: Summary::sanitize(""),
            title: Some(Summary::sanitize(title)),
            at,
        }
    }

    #[test]
    fn threads_are_set_cleared_and_keep_their_first_title() {
        let shell: SessionName = "claude".parse().expect("valid");
        let one: ThreadKey = "one".parse().expect("valid");
        let two: ThreadKey = "two".parse().expect("valid");
        let statuses = update(
            Statuses::new(),
            shell.clone(),
            one.clone(),
            set(AttentionState::Working, "fix login", 1),
        );
        let statuses = update(
            statuses,
            shell.clone(),
            one.clone(),
            set(AttentionState::Done, "something else", 2),
        );
        let statuses = update(
            statuses,
            shell.clone(),
            two.clone(),
            set(AttentionState::Waiting, "add tests", 3),
        );
        let threads = statuses.get(&shell).expect("the session");
        assert_eq!(
            threads.get(&one).map(|e| (e.state, e.title.as_str())),
            Some((AttentionState::Done, "fix login"))
        );
        assert_eq!(threads.len(), 2);
        let statuses = update(statuses, shell.clone(), one, Report::Clear);
        let statuses = update(statuses, shell.clone(), two, Report::Clear);
        assert!(statuses.is_empty(), "a session without threads goes");
    }

    #[test]
    fn old_exited_threads_are_pruned() {
        let shell: SessionName = "claude".parse().expect("valid");
        let statuses = (0..9).fold(Statuses::new(), |statuses, n| {
            let thread: ThreadKey = format!("t{n}").parse().expect("valid");
            update(
                statuses,
                shell.clone(),
                thread,
                set(AttentionState::Exited, "", n),
            )
        });
        assert_eq!(
            statuses.get(&shell).map(BTreeMap::len),
            Some(attention::KEPT_EXITED)
        );
    }
}
