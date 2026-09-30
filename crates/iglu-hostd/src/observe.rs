//! Turning Incus state and guest files into domain observations.

use std::collections::HashMap;
use std::sync::Mutex;

use iglu_domain::attention::{AttentionState, SessionStatus, Summary};
use iglu_domain::capacity::Bytes;
use iglu_domain::env::GuestUser;
use iglu_domain::id::{InstanceName, PrincipalId, WorkspaceId};
use iglu_domain::lifecycle::{
    Instance, Present, Provisioning, Readiness, Running, Runtime, SecretsGeneration,
};
use iglu_domain::terminal::SessionName;
use iglu_domain::time::Timestamp;
use iglu_proto::InstanceReport;
use serde::Deserialize;

use crate::guestfs::{self, paths};
use crate::incus::{InstanceFull, InstanceState};

/// Instance config keys hostd owns. Guests can read `user.*` keys but not
/// change them, so they are trustworthy records of iglu's own decisions.
pub mod keys {
    pub const WORKSPACE: &str = "user.iglu.workspace";
    pub const OWNER: &str = "user.iglu.owner";
    pub const GUEST_USER: &str = "user.iglu.guest-user";
    pub const PROVISIONED: &str = "user.iglu.provisioned";
}

/// An Incus instance that belongs to iglu, with the facts hostd recorded on it.
#[derive(Clone, Debug)]
pub struct Owned {
    pub name: InstanceName,
    pub owner: PrincipalId,
    pub user: GuestUser,
    pub provisioning: Provisioning,
    pub state: Option<InstanceState>,
}

#[derive(Debug, thiserror::Error)]
pub enum Ownership {
    #[error("not an iglu instance")]
    Foreign,
    #[error("iglu instance {0} has damaged metadata: {1}")]
    Damaged(String, &'static str),
}

impl Owned {
    /// Accepts an instance only if both its name and its recorded workspace
    /// say it's iglu's. A matching name alone isn't proof.
    pub fn parse(instance: InstanceFull) -> Result<Self, Ownership> {
        let name: InstanceName = instance.name.parse().map_err(|_| Ownership::Foreign)?;
        let recorded: Option<WorkspaceId> = instance
            .config
            .get(keys::WORKSPACE)
            .and_then(|v| v.parse().ok());
        if recorded != Some(name.workspace()) {
            return Err(Ownership::Foreign);
        }
        let damaged = |why| Ownership::Damaged(instance.name.clone(), why);
        let owner = instance
            .config
            .get(keys::OWNER)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| damaged("owner"))?;
        let user = instance
            .config
            .get(keys::GUEST_USER)
            .and_then(|v| serde_json::from_str(v).ok())
            .ok_or_else(|| damaged("guest user"))?;
        let provisioning = match instance.config.get(keys::PROVISIONED).map(String::as_str) {
            Some("true") => Provisioning::Complete,
            Some(_) | None => Provisioning::Pending,
        };
        Ok(Self {
            name,
            owner,
            user,
            provisioning,
            state: instance.state,
        })
    }

    pub const fn workspace(&self) -> WorkspaceId {
        self.name.workspace()
    }

    /// The init PID of a running or frozen instance.
    pub fn pid(&self) -> Option<i64> {
        self.state.as_ref().map(|s| s.pid).filter(|pid| *pid > 0)
    }
}

/// What Incus's status code says, before looking inside the guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Stopped,
    Running,
    Frozen,
    Transitioning,
    Failed,
}

/// Incus status codes: 102 Stopped, 103 Running, 110 Frozen, 112 Error;
/// everything else is a transition in progress.
pub const fn status(code: u16) -> Status {
    match code {
        102 => Status::Stopped,
        103 => Status::Running,
        110 => Status::Frozen,
        112 => Status::Failed,
        _ => Status::Transitioning,
    }
}

/// What hostd learned by looking inside one boot of a guest. Keyed by init
/// PID, so a restart starts over. Rebuilt from scratch if hostd restarts.
#[derive(Clone, Copy, Debug, Default)]
struct BootFacts {
    system_ready: bool,
    secrets: Option<SecretsGeneration>,
}

#[derive(Default)]
pub struct BootCache(Mutex<HashMap<InstanceName, (i64, BootFacts)>>);

impl BootCache {
    fn get(&self, name: InstanceName, pid: i64) -> BootFacts {
        let cache = self
            .0
            .lock()
            .expect("the boot cache lock is never held across a panic");
        match cache.get(&name) {
            Some((cached_pid, facts)) if *cached_pid == pid => *facts,
            Some(_) | None => BootFacts::default(),
        }
    }

    fn put(&self, name: InstanceName, pid: i64, facts: BootFacts) {
        self.0
            .lock()
            .expect("the boot cache lock is never held across a panic")
            .insert(name, (pid, facts));
    }

    /// Records a secrets delivery that just succeeded.
    pub fn delivered(&self, name: InstanceName, pid: i64, generation: SecretsGeneration) {
        let mut facts = self.get(name, pid);
        facts.secrets = Some(generation);
        self.put(name, pid, facts);
    }

    pub fn forget(&self, name: InstanceName) {
        self.0
            .lock()
            .expect("the boot cache lock is never held across a panic")
            .remove(&name);
    }
}

/// Observes one owned instance.
pub async fn instance(owned: &Owned, cache: &BootCache) -> Instance {
    let Some(state) = &owned.state else {
        return Instance::Present(Present {
            runtime: Runtime::Transitioning,
            provisioning: owned.provisioning,
        });
    };
    let runtime = match status(state.status_code) {
        Status::Stopped => Runtime::Stopped,
        Status::Frozen => Runtime::Frozen,
        Status::Transitioning => Runtime::Transitioning,
        Status::Failed => Runtime::Failed,
        Status::Running => Runtime::Running(running(owned.name, state, cache).await),
    };
    Instance::Present(Present {
        runtime,
        provisioning: owned.provisioning,
    })
}

async fn running(name: InstanceName, state: &InstanceState, cache: &BootCache) -> Running {
    let pid = state.pid;
    let mut facts = cache.get(name, pid);
    if !facts.system_ready {
        facts.system_ready = matches!(guestfs::read(pid, paths::READY).await, Ok(Some(_)));
    }
    if facts.system_ready && facts.secrets.is_none() {
        facts.secrets = match guestfs::read(pid, paths::SECRETS_GENERATION).await {
            Ok(Some(bytes)) => parse_generation(&bytes),
            Ok(None) | Err(_) => None,
        };
    }
    cache.put(name, pid, facts);
    let readiness = match (facts.system_ready, has_address(state)) {
        (false, _) => Readiness::Booting,
        (true, false) => Readiness::System,
        (true, true) => Readiness::Network,
    };
    Running {
        readiness,
        secrets: facts.secrets,
    }
}

fn parse_generation(bytes: &[u8]) -> Option<SecretsGeneration> {
    std::str::from_utf8(bytes)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(SecretsGeneration::from_u64)
}

fn has_address(state: &InstanceState) -> bool {
    state.network.as_ref().is_some_and(|nics| {
        nics.iter()
            .filter(|(name, _)| name.as_str() != "lo")
            .flat_map(|(_, nic)| &nic.addresses)
            .any(|addr| addr.family == "inet" && addr.scope == "global")
    })
}

/// One entry of the guest's status file, as `iglu-status` writes it.
#[derive(Deserialize)]
struct RawStatus {
    state: AttentionState,
    #[serde(default)]
    summary: Option<Summary>,
    at: i64,
}

/// Parses the guest's status file. Untrusted: malformed entries are dropped
/// individually, and the whole file is capped.
pub fn parse_statuses(bytes: &[u8]) -> Vec<SessionStatus> {
    const MAX_SESSIONS: usize = 64;
    let Ok(raw) = serde_json::from_slice::<HashMap<String, serde_json::Value>>(bytes) else {
        return Vec::new();
    };
    let mut statuses: Vec<SessionStatus> = raw
        .into_iter()
        .filter_map(|(session, value)| {
            let session: SessionName = session.parse().ok()?;
            let status: RawStatus = serde_json::from_value(value).ok()?;
            Some(SessionStatus {
                session,
                state: status.state,
                summary: status
                    .summary
                    .unwrap_or_else(|| "".parse().expect("empty summaries parse")),
                updated_at: Timestamp::from_unix_millis(status.at),
            })
        })
        .collect();
    statuses.sort_by(|a, b| a.session.cmp(&b.session));
    statuses.truncate(MAX_SESSIONS);
    statuses
}

pub async fn report(owned: &Owned, cache: &BootCache) -> InstanceReport {
    let instance = instance(owned, cache).await;
    let running = matches!(
        instance,
        Instance::Present(Present {
            runtime: Runtime::Running(_),
            ..
        })
    );
    let sessions = match (running, owned.pid()) {
        (true, Some(pid)) => match guestfs::read(pid, paths::STATUS).await {
            Ok(Some(bytes)) => parse_statuses(&bytes),
            Ok(None) | Err(_) => Vec::new(),
        },
        (true, None) | (false, _) => Vec::new(),
    };
    let memory = owned
        .state
        .as_ref()
        .map(|s| Bytes::new(s.memory.usage))
        .filter(|b| b.get() > 0);
    InstanceReport {
        workspace: owned.workspace(),
        owner: owned.owner,
        instance,
        memory,
        sessions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incus_status_codes_classify() {
        assert_eq!(status(102), Status::Stopped);
        assert_eq!(status(103), Status::Running);
        assert_eq!(status(110), Status::Frozen);
        assert_eq!(status(112), Status::Failed);
        assert_eq!(status(106), Status::Transitioning);
    }

    #[test]
    fn statuses_drop_malformed_entries() {
        let file = br#"{
            "t1": {"state": "waiting", "summary": "approve Bash?", "at": 5},
            "t2": {"state": "exploding", "at": 5},
            "../x": {"state": "done", "at": 1},
            "t3": {"state": "done", "at": 7}
        }"#;
        let parsed = parse_statuses(file);
        let names: Vec<_> = parsed.iter().map(|s| s.session.as_str()).collect();
        assert_eq!(names, ["t1", "t3"]);
        assert_eq!(parsed[0].summary.as_str(), "approve Bash?");
        assert!(parse_statuses(b"not json").is_empty());
    }
}
