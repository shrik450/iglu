//! What the local runtime keeps on disk, and where.
//!
//! ```text
//! <state>/images/<fingerprint>.json      an environment recorded here
//! <state>/instances/<name>/record.json   who it's for, as on an Incus instance
//! <state>/instances/<name>/power.json    stopped, running or frozen, and which boot
//! <state>/instances/<name>/home/         the workspace's HOME
//! <state>/slots                          the last runtime slot handed out
//! <state>/gitconfig                      Git's system configuration for workspaces
//! <runtime>/<slot>/                      its XDG_RUNTIME_DIR for the current boot
//! ```
//!
//! A workspace's runtime directory is named by a short slot rather than its
//! name, because terminal sessions are Unix sockets inside it and socket
//! paths are short. Files are replaced atomically, so a crash leaves the old
//! or the new one.

use std::fmt;
use std::path::{Path, PathBuf};

use iglu_domain::env::{EnvSource, GuestUser, ImageFingerprint};
use iglu_domain::guest::Interface;
use iglu_domain::id::{InstanceName, PrincipalId, WorkspaceId};
use iglu_domain::terminal::SessionName;
use iglu_hostd::runtime::{BootId, State};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

pub struct Layout {
    state: PathBuf,
    runtime: PathBuf,
}

impl Layout {
    pub const fn new(state: PathBuf, runtime: PathBuf) -> Self {
        Self { state, runtime }
    }

    pub fn images(&self) -> PathBuf {
        self.state.join("images")
    }

    pub fn image(&self, fingerprint: &ImageFingerprint) -> PathBuf {
        self.images().join(format!("{fingerprint}.json"))
    }

    pub fn instances(&self) -> PathBuf {
        self.state.join("instances")
    }

    pub fn instance(&self, name: InstanceName) -> PathBuf {
        self.instances().join(name.to_string())
    }

    pub fn record(&self, name: InstanceName) -> PathBuf {
        self.instance(name).join("record.json")
    }

    pub fn power(&self, name: InstanceName) -> PathBuf {
        self.instance(name).join("power.json")
    }

    pub fn home(&self, name: InstanceName) -> PathBuf {
        self.instance(name).join("home")
    }

    pub fn slots(&self) -> PathBuf {
        self.state.join("slots")
    }

    pub fn gitconfig(&self) -> PathBuf {
        self.state.join("gitconfig")
    }

    pub fn runtime(&self) -> &Path {
        &self.runtime
    }

    pub fn run(&self, slot: Slot) -> PathBuf {
        self.runtime.join(slot.to_string())
    }
}

/// Names a workspace's runtime directory. Handed out once each, in order,
/// so no two workspaces ever share one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Slot(u32);

impl Slot {
    pub const NONE_YET: Self = Self(0);

    #[must_use]
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The longest path a Unix socket can have on every platform the devhost
/// runs on: macOS allows 104 bytes, including the terminating NUL.
pub const MAX_SOCKET_PATH: usize = 103;

/// The longest terminal socket path any workspace could need under
/// `runtime`: the largest slot, zmx's directory, and the longest session
/// name.
#[must_use]
pub fn longest_socket_path(runtime: &Path) -> usize {
    let slot = u32::MAX.to_string().len();
    runtime.as_os_str().len() + 1 + slot + "/zmx/".len() + SessionName::MAX_LEN
}

/// An environment recorded, not built: workspaces use this machine's tools.
#[derive(Debug, Serialize, Deserialize)]
pub struct Image {
    pub source: EnvSource,
    /// What the guest tools spoke when the environment was recorded.
    #[serde(default)]
    pub guest_interface: Option<Interface>,
}

/// What hostd records on an instance, and where its runtime directory is.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub workspace: WorkspaceId,
    pub owner: PrincipalId,
    pub user: GuestUser,
    pub image: ImageFingerprint,
    pub provisioned: bool,
    pub slot: Slot,
    /// What the guest tools spoke when the workspace was created. Records
    /// from before it was kept don't say.
    #[serde(default)]
    pub guest_interface: Option<Interface>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Power {
    Stopped,
    Running,
    Frozen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boots {
    pub power: Power,
    /// How many times the workspace has started, which is also the current
    /// boot's ID while it's up.
    pub count: u64,
    /// The machine boot the workspace last started in.
    pub machine: MachineBoot,
}

impl Boots {
    pub const NEVER: Self = Self {
        power: Power::Stopped,
        count: 0,
        machine: MachineBoot(0),
    };
}

/// One boot of the machine, as the time it booted. A workspace's processes
/// can't outlive it, and nor should its runtime directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MachineBoot(u64);

impl MachineBoot {
    pub fn current() -> Self {
        Self(sysinfo::System::boot_time())
    }
}

/// What's left of a workspace's current boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remains {
    /// Its runtime directory exists, so its boot does.
    RuntimeDir,
    /// The runtime directory is gone, but some of its processes aren't.
    Processes,
    Nothing,
}

/// The state a workspace is in, in machine boot `now`. A workspace last
/// started in an earlier machine boot is stopped: restarting the machine
/// ended its processes. So is one recorded as up with nothing left. One
/// whose runtime directory went while its processes didn't has lost its
/// boot, which is a failure; starting it again ends them first.
#[must_use]
pub fn state(boots: Boots, now: MachineBoot, remains: Remains) -> State {
    if boots.machine != now {
        return State::Stopped;
    }
    match (boots.power, remains) {
        (Power::Stopped, _) | (Power::Running | Power::Frozen, Remains::Nothing) => State::Stopped,
        (Power::Running | Power::Frozen, Remains::Processes) => State::Failed,
        (Power::Running, Remains::RuntimeDir) => State::Running {
            boot: BootId::new(boots.count),
            has_address: true,
        },
        (Power::Frozen, Remains::RuntimeDir) => State::Frozen,
    }
}

/// Replaces `path` with `value` as JSON, atomically.
pub async fn write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let temp = path.with_extension("tmp");
    let mut file = tokio::fs::File::create(&temp).await?;
    file.write_all(&serde_json::to_vec(value).map_err(std::io::Error::other)?)
        .await?;
    file.sync_all().await?;
    tokio::fs::rename(temp, path).await
}

pub async fn read_json<T: DeserializeOwned>(path: &Path) -> std::io::Result<T> {
    serde_json::from_slice(&tokio::fs::read(path).await?).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_is_up_only_while_its_boot_is() {
        let now = MachineBoot(100);
        let up = |power| Boots {
            power,
            count: 3,
            machine: now,
        };
        assert_eq!(
            state(up(Power::Running), now, Remains::RuntimeDir),
            State::Running {
                boot: BootId::new(3),
                has_address: true
            }
        );
        assert_eq!(
            state(up(Power::Frozen), now, Remains::RuntimeDir),
            State::Frozen
        );
        for power in [Power::Running, Power::Frozen] {
            assert_eq!(
                state(up(power), now, Remains::Nothing),
                State::Stopped,
                "{power:?}"
            );
            assert_eq!(
                state(up(power), now, Remains::Processes),
                State::Failed,
                "{power:?}"
            );
        }
        for remains in [Remains::RuntimeDir, Remains::Processes, Remains::Nothing] {
            assert_eq!(state(up(Power::Stopped), now, remains), State::Stopped);
        }
    }

    #[test]
    fn restarting_the_machine_stops_every_workspace() {
        let before = Boots {
            power: Power::Running,
            count: 3,
            machine: MachineBoot(100),
        };
        assert_eq!(
            state(before, MachineBoot(200), Remains::RuntimeDir),
            State::Stopped
        );
    }

    #[test]
    fn a_cache_path_under_a_home_leaves_room_for_every_session() {
        let fits = Path::new("/Users/someone/.cache/iglu/0123abcd");
        assert!(longest_socket_path(fits) <= MAX_SOCKET_PATH);
        let worktree =
            Path::new("/Users/someone/worktrees/iglu/local-runtime/.dev/state/devhost/run");
        assert!(longest_socket_path(worktree) > MAX_SOCKET_PATH);
    }
}
