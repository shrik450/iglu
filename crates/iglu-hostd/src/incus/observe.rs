//! Reading Incus's view of an instance into hostd's.

use iglu_domain::capacity::Bytes;
use iglu_domain::guest::Interface;
use iglu_domain::id::{InstanceName, WorkspaceId};
use iglu_domain::lifecycle::Provisioning;

use super::client::{InstanceFull, InstanceState};
use crate::runtime::{BootId, Claim, Observed, Ownership, State};

/// Instance config keys hostd owns. Guests can read `user.*` keys but not
/// change them, so they are trustworthy records of iglu's own decisions.
pub mod keys {
    pub const WORKSPACE: &str = "user.iglu.workspace";
    pub const OWNER: &str = "user.iglu.owner";
    pub const GUEST_USER: &str = "user.iglu.guest-user";
    pub const PROVISIONED: &str = "user.iglu.provisioned";
    pub const GUEST_INTERFACE: &str = "user.iglu.guest-interface";
}

/// Accepts an instance only if both its name and its recorded workspace say
/// it's iglu's. A matching name alone isn't proof.
pub fn claim(instance: &InstanceFull) -> Claim {
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
    Ok(Observed {
        name,
        owner,
        user,
        provisioning,
        guest_interface: instance
            .config
            .get(keys::GUEST_INTERFACE)
            .and_then(|v| v.parse().ok())
            .map(Interface::new),
        state: instance.state.as_ref().map_or(State::Transitioning, state),
        memory: instance
            .state
            .as_ref()
            .map(|s| Bytes::new(s.memory.usage))
            .filter(|b| b.get() > 0),
    })
}

/// Incus status codes: 102 Stopped, 103 Running, 110 Frozen, 112 Error;
/// everything else is a transition in progress. A boot is identified by its
/// init PID, which hostd also reads guest files through.
fn state(state: &InstanceState) -> State {
    match state.status_code {
        102 => State::Stopped,
        103 => match u64::try_from(state.pid) {
            Ok(pid) if pid > 0 => State::Running {
                boot: BootId::new(pid),
                has_address: has_address(state),
            },
            Ok(_) | Err(_) => State::Transitioning,
        },
        110 => State::Frozen,
        112 => State::Failed,
        _ => State::Transitioning,
    }
}

fn has_address(state: &InstanceState) -> bool {
    state.network.as_ref().is_some_and(|nics| {
        nics.iter()
            .filter(|(name, _)| name.as_str() != "lo")
            .flat_map(|(_, nic)| &nic.addresses)
            .any(|addr| addr.family == "inet" && addr.scope == "global")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(status_code: u16, pid: i64) -> State {
        state(&InstanceState {
            status_code,
            pid,
            memory: super::super::client::MemoryState::default(),
            network: None,
        })
    }

    #[test]
    fn incus_status_codes_classify() {
        assert_eq!(with(102, 0), State::Stopped);
        assert_eq!(
            with(103, 42),
            State::Running {
                boot: BootId::new(42),
                has_address: false
            }
        );
        assert_eq!(with(110, 42), State::Frozen);
        assert_eq!(with(112, 0), State::Failed);
        assert_eq!(with(106, 42), State::Transitioning);
    }

    #[test]
    fn running_without_an_init_is_still_starting() {
        assert_eq!(with(103, 0), State::Transitioning);
        assert_eq!(with(103, -1), State::Transitioning);
    }
}
