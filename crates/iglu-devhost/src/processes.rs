//! Finding and signalling a workspace's processes. Every process the local
//! runtime starts carries a marker in its environment naming its workspace,
//! which children inherit; processes that clear their environment are still
//! found as descendants of marked ones. A marker always wins over ancestry,
//! and the devhost itself is never a member, so stopping one workspace can't
//! reach another, or the devhost and what it started. Without isolation this
//! is best effort: a process that drops the marker and leaves the tree
//! escapes.

use std::collections::{HashMap, HashSet};

use std::time::Duration;

use iglu_domain::id::InstanceName;
use rustix::process::{Pid, Signal, kill_process, test_kill_process};
use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

/// The environment variable that marks a workspace's processes.
pub const MARKER: &str = "IGLU_DEVHOST_WORKSPACE";

/// Which workspace a process's environment says it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    This,
    Another,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub pid: u32,
    pub parent: Option<u32>,
    pub mark: Mark,
}

/// The processes marked for this workspace and their descendants, except
/// `devhost` and anything marked for another workspace, and what lies below
/// either.
#[must_use]
pub fn members(table: &[Entry], devhost: u32) -> Vec<u32> {
    let marks: HashMap<u32, Mark> = table.iter().map(|e| (e.pid, e.mark)).collect();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for entry in table {
        if let Some(parent) = entry.parent {
            children.entry(parent).or_default().push(entry.pid);
        }
    }
    let ours = |pid: &u32| pid != &devhost && marks.get(pid) != Some(&Mark::Another);
    let mut found: HashSet<u32> = HashSet::new();
    let mut pending: Vec<u32> = table
        .iter()
        .filter(|e| e.mark == Mark::This)
        .map(|e| e.pid)
        .filter(ours)
        .collect();
    while let Some(pid) = pending.pop() {
        if found.insert(pid) {
            pending.extend(children.get(&pid).into_iter().flatten().filter(|c| ours(c)));
        }
    }
    let mut members: Vec<u32> = found.into_iter().collect();
    members.sort_unstable();
    members
}

fn mark(environ: &[std::ffi::OsString], name: &str) -> Mark {
    let prefix = format!("{MARKER}=");
    environ
        .iter()
        .find_map(|var| var.to_str()?.strip_prefix(&prefix))
        .map_or(Mark::None, |value| {
            if value == name {
                Mark::This
            } else {
                Mark::Another
            }
        })
}

fn table(name: InstanceName) -> Vec<Entry> {
    let name = name.to_string();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
    );
    system
        .processes()
        .iter()
        // A zombie has already ended; only its parent can clear it.
        .filter(|(_, process)| process.status() != ProcessStatus::Zombie)
        .map(|(pid, process)| Entry {
            pid: pid.as_u32(),
            parent: process.parent().map(sysinfo::Pid::as_u32),
            mark: mark(process.environ(), &name),
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("listing processes failed: {0}")]
    Scan(String),
    #[error("{0} processes outlived SIGKILL")]
    Survivors(usize),
}

/// The workspace's processes right now.
///
/// # Errors
///
/// When the process table can't be read.
pub async fn of(name: InstanceName) -> Result<Vec<u32>, ProcessError> {
    tokio::task::spawn_blocking(move || members(&table(name), std::process::id()))
        .await
        .map_err(|e| ProcessError::Scan(e.to_string()))
}

fn pid(raw: u32) -> Option<Pid> {
    i32::try_from(raw).ok().and_then(Pid::from_raw)
}

/// Sends `signal` to every process. Ones that are already gone are fine.
pub fn signal(pids: &[u32], signal: Signal) {
    for pid in pids.iter().copied().filter_map(pid) {
        let _ = kill_process(pid, signal);
    }
}

fn alive(pids: &[u32]) -> Vec<u32> {
    pids.iter()
        .copied()
        .filter(|raw| pid(*raw).is_some_and(|pid| test_kill_process(pid).is_ok()))
        .collect()
}

/// Ends every process of the workspace, frozen ones included: asks first,
/// and kills whatever is left after `grace`, then checks nothing survived.
///
/// # Errors
///
/// When the process table can't be read, or processes outlive `SIGKILL`.
pub async fn end(name: InstanceName, grace: Duration) -> Result<(), ProcessError> {
    let pids = of(name).await?;
    signal(&pids, Signal::CONT);
    signal(&pids, Signal::TERM);
    let deadline = tokio::time::Instant::now() + grace;
    while !alive(&pids).is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Anything started since the first look goes too.
    let left = of(name).await?;
    if left.is_empty() {
        return Ok(());
    }
    signal(&left, Signal::KILL);
    tokio::time::sleep(Duration::from_millis(200)).await;
    match of(name).await?.len() {
        0 => Ok(()),
        survivors => Err(ProcessError::Survivors(survivors)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVHOST: u32 = 50;

    fn entry(pid: u32, parent: u32, mark: Mark) -> Entry {
        Entry {
            pid,
            parent: Some(parent),
            mark,
        }
    }

    #[test]
    fn members_are_marked_processes_and_their_descendants() {
        let table = [
            entry(1, 0, Mark::None),
            // A session daemon, reparented to init, and its shell.
            entry(10, 1, Mark::This),
            entry(11, 10, Mark::This),
            // A dev server that cleared its environment.
            entry(12, 11, Mark::None),
            entry(13, 12, Mark::None),
            // The user's own.
            entry(20, 1, Mark::None),
            entry(21, 20, Mark::None),
        ];
        assert_eq!(members(&table, DEVHOST), [10, 11, 12, 13]);
        assert!(members(&[entry(5, 1, Mark::None)], DEVHOST).is_empty());
    }

    #[test]
    fn a_child_marked_for_another_workspace_is_not_ours() {
        let table = [
            entry(10, 1, Mark::This),
            entry(11, 10, Mark::Another),
            entry(12, 11, Mark::None),
        ];
        assert_eq!(members(&table, DEVHOST), [10]);
    }

    #[test]
    fn the_devhost_and_what_it_started_are_never_members() {
        // A devhost started from a workspace's terminal carries its marker,
        // as does everything it starts for other workspaces.
        let table = [
            entry(10, 1, Mark::This),
            entry(DEVHOST, 10, Mark::This),
            entry(51, DEVHOST, Mark::None),
            entry(52, DEVHOST, Mark::Another),
        ];
        assert_eq!(members(&table, DEVHOST), [10]);
    }

    #[test]
    fn marks_match_the_whole_name() {
        let environ = |value: &str| vec![format!("{MARKER}={value}").into()];
        assert_eq!(mark(&environ("iglu-a"), "iglu-a"), Mark::This);
        assert_eq!(mark(&environ("iglu-ab"), "iglu-a"), Mark::Another);
        assert_eq!(mark(&[], "iglu-a"), Mark::None);
    }
}
