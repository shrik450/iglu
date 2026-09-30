//! Copies of the database, taken at startup before any migration and then
//! periodically, keeping the newest few. Ship the directory elsewhere to
//! survive losing the disk.
//!
//! Stored secrets in a copy are sealed with the key in `secret_key_file`,
//! which the copy doesn't include; back the key up separately.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iglu_domain::time::Timestamp;
use rusqlite::{Connection, OpenFlags};

use crate::app::now;
use crate::config::Backups;

const PREFIX: &str = "iglu-";
const SUFFIX: &str = ".db";
const PARTIAL: &str = ".partial";

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("backup: {0}")]
    Io(#[from] std::io::Error),
    #[error("backup: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("backup: the directory {0} isn't a UTF-8 path")]
    Path(PathBuf),
}

/// Takes a copy every interval. The first comes one interval after startup,
/// which takes its own.
pub async fn run(database: PathBuf, policy: Backups) {
    let period = policy.interval();
    let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        let (database, policy) = (database.clone(), policy.clone());
        match tokio::task::spawn_blocking(move || back_up(&database, &policy, now())).await {
            Ok(Ok(path)) => tracing::info!(path = %path.display(), "backed up the database"),
            Ok(Err(error)) => tracing::warn!(%error, "couldn't back up the database"),
            Err(error) => tracing::warn!(%error, "the backup task failed"),
        }
    }
}

/// Writes a consistent copy of `database` and prunes old copies.
///
/// # Errors
///
/// When the directory can't be written or SQLite can't read the database.
pub fn back_up(database: &Path, policy: &Backups, at: Timestamp) -> Result<PathBuf, BackupError> {
    let directory = &policy.directory;
    std::fs::create_dir_all(directory)?;
    // A crash mid-copy leaves a partial file behind.
    for name in names(directory)? {
        if name.starts_with(PREFIX) && name.ends_with(PARTIAL) {
            std::fs::remove_file(directory.join(name))?;
        }
    }

    let name = file_name(at);
    let partial = directory.join(format!("{name}{PARTIAL}"));
    let target = partial
        .to_str()
        .ok_or_else(|| BackupError::Path(directory.clone()))?;
    // Without the create flag, a missing database is an error, not a new one.
    let source = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    source.execute("VACUUM INTO ?1", [target])?;
    drop(source);
    std::fs::File::open(&partial)?.sync_all()?;
    let path = directory.join(&name);
    std::fs::rename(&partial, &path)?;

    for old in expired(&names(directory)?, policy.keep) {
        std::fs::remove_file(directory.join(old))?;
    }
    Ok(path)
}

fn names(directory: &Path) -> std::io::Result<Vec<String>> {
    std::fs::read_dir(directory)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect()
}

fn file_name(at: Timestamp) -> String {
    format!("{PREFIX}{}{SUFFIX}", at.unix_millis())
}

fn taken_at(name: &str) -> Option<i64> {
    name.strip_prefix(PREFIX)?
        .strip_suffix(SUFFIX)?
        .parse()
        .ok()
}

/// The copies to delete so that only the newest `keep` remain. Anything
/// that isn't a copy is left alone.
fn expired(names: &[String], keep: NonZeroUsize) -> Vec<&str> {
    let mut copies: Vec<(i64, &str)> = names
        .iter()
        .filter_map(|name| taken_at(name).map(|at| (at, name.as_str())))
        .collect();
    copies.sort_unstable_by(|a, b| b.cmp(a));
    copies
        .into_iter()
        .skip(keep.get())
        .map(|(_, name)| name)
        .collect()
}

impl Backups {
    pub fn interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.interval_minutes.get()) * 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keep(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("nonzero")
    }

    #[test]
    fn only_the_newest_copies_survive() {
        let names: Vec<String> = [900, 1_000, 10_000, 5]
            .into_iter()
            .map(|at| file_name(Timestamp::from_unix_millis(at)))
            .collect();
        // By time, not by name: "iglu-900.db" sorts after "iglu-10000.db".
        assert_eq!(expired(&names, keep(2)), vec!["iglu-900.db", "iglu-5.db"]);
        assert!(expired(&names, keep(4)).is_empty());
    }

    #[test]
    fn other_files_are_left_alone() {
        let names = [
            "iglu-1.db",
            "iglu-2.db",
            "iglu-3.db.partial",
            "notes.txt",
            "iglu-x.db",
        ]
        .map(str::to_owned);
        assert_eq!(expired(&names, keep(1)), vec!["iglu-1.db"]);
    }
}
