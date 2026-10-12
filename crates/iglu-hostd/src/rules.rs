//! hostd's decisions, as pure functions over what the runtime reports:
//! which steps a command takes from the state an instance is in, when hostd
//! may act inside a guest, and what an observation tells the control plane.
//!
//! Every command is idempotent: one that already took effect takes no steps.

use iglu_domain::guest as guest_tools;
use iglu_domain::lifecycle::{
    Columns, Instance, Present, Readiness, Running, Runtime, SecretsGeneration,
};
use iglu_proto::{CommandError, CreateSpec, ErrorCode, UpgradeSpec};

use crate::runtime::{Claim, Guest, Observed, Ownership, State, StoredImage};
use iglu_domain::env::ImageFingerprint;
use iglu_domain::time::{Millis, Timestamp};

/// A lifecycle step: what a protocol command asks for, and what a runtime
/// does. A command takes one or more steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step<'a> {
    Create(&'a CreateSpec),
    Start,
    Freeze,
    Thaw,
    /// Ends every process, whether the instance is running or frozen.
    Stop,
    Delete,
    Upgrade(&'a UpgradeSpec),
}

fn not_ours(ownership: &Ownership) -> CommandError {
    match ownership {
        Ownership::Foreign => CommandError::new(ErrorCode::Conflict, "the instance isn't iglu's"),
        Ownership::Damaged(..) => CommandError::new(ErrorCode::Conflict, ownership.to_string()),
    }
}

fn missing() -> CommandError {
    CommandError::new(ErrorCode::NotFound, "no such workspace instance")
}

fn invalid(message: &str) -> CommandError {
    CommandError::new(ErrorCode::InvalidState, message)
}

/// The steps that carry out `command` on the instance found under the
/// workspace's name, in order. None when it has already taken effect.
///
/// # Errors
///
/// When the command can't apply: the instance is missing, isn't iglu's, or
/// is in a state the command doesn't start from, or, to start, has guest
/// tools that speak another interface than this host.
pub fn decide<'a>(command: Step<'a>, found: Option<&Claim>) -> Result<Vec<Step<'a>>, CommandError> {
    let observed = match found {
        None => {
            return match command {
                Step::Create(_) => Ok(vec![command]),
                Step::Delete => Ok(vec![]),
                Step::Start | Step::Freeze | Step::Thaw | Step::Stop | Step::Upgrade(_) => {
                    Err(missing())
                }
            };
        }
        Some(Err(ownership)) => return Err(not_ours(ownership)),
        Some(Ok(observed)) => observed,
    };
    let state = observed.state;
    match command {
        Step::Create(_) => Ok(vec![]),
        Step::Start => match state {
            State::Stopped | State::Failed => {
                speaks_this_hosts_interface(observed)?;
                Ok(vec![Step::Start])
            }
            State::Running { .. } | State::Frozen | State::Transitioning => Ok(vec![]),
        },
        Step::Freeze => match state {
            State::Running { .. } => Ok(vec![Step::Freeze]),
            State::Frozen => Ok(vec![]),
            State::Stopped | State::Transitioning | State::Failed => {
                Err(invalid("only a running workspace can be frozen"))
            }
        },
        Step::Thaw => match state {
            State::Frozen => Ok(vec![Step::Thaw]),
            State::Running { .. } => Ok(vec![]),
            State::Stopped | State::Transitioning | State::Failed => {
                Err(invalid("the workspace isn't frozen"))
            }
        },
        Step::Stop => match state {
            State::Stopped => Ok(vec![]),
            State::Running { .. } | State::Frozen | State::Transitioning | State::Failed => {
                Ok(vec![Step::Stop])
            }
        },
        Step::Delete => match state {
            State::Stopped => Ok(vec![Step::Delete]),
            State::Running { .. } | State::Frozen | State::Transitioning | State::Failed => {
                Ok(vec![Step::Stop, Step::Delete])
            }
        },
        // The runtime brings it up to upgrade it: only system tools run, so
        // an instance whose own tools speak another interface still can be.
        Step::Upgrade(spec) => match state {
            State::Stopped | State::Running { .. } | State::Frozen => Ok(vec![Step::Upgrade(spec)]),
            State::Failed => Ok(vec![Step::Stop, Step::Upgrade(spec)]),
            State::Transitioning => Err(invalid("the workspace is changing state")),
        },
    }
}

/// The guest hostd may run commands in, read from, or connect to: only a
/// running one.
///
/// # Errors
///
/// When the instance is missing, isn't iglu's, or isn't running.
pub fn guest(found: Option<&Claim>) -> Result<Guest, CommandError> {
    match found {
        None => Err(missing()),
        Some(Err(ownership)) => Err(not_ours(ownership)),
        Some(Ok(observed)) => {
            running(observed).ok_or_else(|| invalid("the workspace isn't running"))
        }
    }
}

/// The guest hostd may run the guest tools in: a running one whose tools
/// speak this host's interface.
///
/// # Errors
///
/// As for [`guest`], and when the instance's tools speak another
/// interface, or it doesn't record one.
pub fn tools_guest(found: Option<&Claim>) -> Result<Guest, CommandError> {
    if let Some(Ok(observed)) = found {
        speaks_this_hosts_interface(observed)?;
    }
    guest(found)
}

/// Whether an instance's guest tools speak this host's interface. Only
/// starting it and running its tools need them to; stopping and deleting
/// an old workspace still work, and nothing ever runs an old tool.
fn speaks_this_hosts_interface(observed: &Observed) -> Result<(), CommandError> {
    guest_tools::compatible(observed.guest_interface).map_err(|incompatible| {
        CommandError::new(
            ErrorCode::ImageIncompatible,
            format!(
                "this workspace's image was built with a different iglu ({}) than this host's \
                 (interface {}): rebuild its environment with this iglu, then update the workspace",
                incompatible.found,
                guest_tools::INTERFACE
            ),
        )
    })
}

/// The guest of an instance, if it's running.
#[must_use]
pub fn running(observed: &Observed) -> Option<Guest> {
    match observed.state {
        State::Running { boot, .. } => Some(Guest {
            name: observed.name,
            user: observed.user.clone(),
            boot,
        }),
        State::Stopped | State::Frozen | State::Transitioning | State::Failed => None,
    }
}

/// What hostd learned by looking inside one boot of a guest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BootFacts {
    /// The guest finished booting.
    pub ready: bool,
    pub secrets: Option<SecretsGeneration>,
    /// The boot has opened the workspace's columns.
    pub columns_opened: bool,
}

impl BootFacts {
    /// Facts only accumulate within a boot: once ready it stays ready, and
    /// generations only grow, so a newer delivery wins over an older read.
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        Self {
            ready: self.ready || other.ready,
            secrets: self.secrets.max(other.secrets),
            columns_opened: self.columns_opened || other.columns_opened,
        }
    }
}

/// What an observation tells the control plane.
#[must_use]
pub fn instance(observed: &Observed, facts: BootFacts) -> Instance {
    let runtime = match observed.state {
        State::Stopped => Runtime::Stopped,
        State::Frozen => Runtime::Frozen,
        State::Transitioning => Runtime::Transitioning,
        State::Failed => Runtime::Failed,
        State::Running { has_address, .. } => Runtime::Running(Running {
            readiness: readiness(facts.ready, has_address),
            secrets: facts.secrets,
            columns: if facts.columns_opened {
                Columns::Opened
            } else {
                Columns::Pending
            },
        }),
    };
    Instance::Present(Present {
        runtime,
        provisioning: observed.provisioning,
    })
}

const fn readiness(ready: bool, has_address: bool) -> Readiness {
    match (ready, has_address) {
        (false, _) => Readiness::Booting,
        (true, false) => Readiness::System,
        (true, true) => Readiness::Network,
    }
}

/// The images to remove: those not kept, but for any built within
/// `older_than` of `now`. One built at an unknown time is old.
#[must_use]
pub fn unkept(
    images: &[StoredImage],
    keep: &[ImageFingerprint],
    older_than: Millis,
    now: Timestamp,
) -> Vec<ImageFingerprint> {
    images
        .iter()
        .filter(|image| !keep.contains(&image.fingerprint))
        .filter(|image| {
            image
                .built
                .is_none_or(|built| now.since(built) >= older_than)
        })
        .map(|image| image.fingerprint.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::BootId;

    fn image(n: u8, built: Option<i64>) -> StoredImage {
        StoredImage {
            fingerprint: format!("{n:064x}").parse().expect("valid fingerprint"),
            built: built.map(Timestamp::from_unix_millis),
        }
    }

    #[test]
    fn images_kept_or_new_stay() {
        let hour = Millis::from_secs(3600);
        let now = Timestamp::from_unix_millis(10_000_000);
        let kept = image(1, Some(0));
        let new = image(2, Some(now.unix_millis() - 60_000));
        let old = image(3, Some(now.unix_millis() - 3_600_000));
        let unknown = image(4, None);
        let images = [kept.clone(), new, old.clone(), unknown.clone()];
        assert_eq!(
            unkept(&images, &[kept.fingerprint], hour, now),
            [old.fingerprint, unknown.fingerprint]
        );
    }
    use iglu_domain::env::GuestUser;
    use iglu_domain::lifecycle::Provisioning;

    const RUNNING: State = State::Running {
        boot: BootId::new(7),
        has_address: true,
    };
    const ALL: [State; 5] = [
        State::Stopped,
        RUNNING,
        State::Frozen,
        State::Transitioning,
        State::Failed,
    ];

    fn observed(state: State) -> Observed {
        Observed {
            name: "iglu-0123456789abcdef0123456789abcdef"
                .parse()
                .expect("valid name"),
            owner: "0123456789abcdef0123456789abcdef"
                .parse()
                .expect("valid id"),
            user: serde_json::from_str::<GuestUser>(
                r#"{"name":"dev","uid":1000,"gid":100,"home":"/home/dev"}"#,
            )
            .expect("valid user"),
            provisioning: Provisioning::Pending,
            guest_interface: Some(guest_tools::INTERFACE),
            image: None,
            state,
            memory: None,
        }
    }

    fn spec() -> CreateSpec {
        serde_json::from_value(serde_json::json!({
            "owner": "0123456789abcdef0123456789abcdef",
            "image": "0".repeat(64),
            "user": {"name": "dev", "uid": 1000, "gid": 100, "home": "/home/dev"},
            "limits": {"cpus": 1, "memory": 1, "processes": 1, "swap": 1},
        }))
        .expect("a valid spec")
    }

    fn steps(command: Step<'_>, state: State) -> Result<Vec<Step<'_>>, ErrorCode> {
        decide(command, Some(&Ok(observed(state)))).map_err(|e| e.code)
    }

    #[test]
    fn create_happens_once() {
        let spec = spec();
        let create = Step::Create(&spec);
        assert_eq!(decide(create, None), Ok(vec![create]));
        for state in ALL {
            assert_eq!(steps(create, state), Ok(vec![]), "{state:?}");
        }
    }

    #[test]
    fn start_boots_only_what_is_down() {
        assert_eq!(steps(Step::Start, State::Stopped), Ok(vec![Step::Start]));
        assert_eq!(steps(Step::Start, State::Failed), Ok(vec![Step::Start]));
        for state in [RUNNING, State::Frozen, State::Transitioning] {
            assert_eq!(steps(Step::Start, state), Ok(vec![]), "{state:?}");
        }
    }

    #[test]
    fn freeze_only_from_running() {
        assert_eq!(steps(Step::Freeze, RUNNING), Ok(vec![Step::Freeze]));
        assert_eq!(steps(Step::Freeze, State::Frozen), Ok(vec![]));
        for state in [State::Stopped, State::Transitioning, State::Failed] {
            assert_eq!(
                steps(Step::Freeze, state),
                Err(ErrorCode::InvalidState),
                "{state:?}"
            );
        }
    }

    #[test]
    fn thaw_only_from_frozen() {
        assert_eq!(steps(Step::Thaw, State::Frozen), Ok(vec![Step::Thaw]));
        assert_eq!(steps(Step::Thaw, RUNNING), Ok(vec![]));
        for state in [State::Stopped, State::Transitioning, State::Failed] {
            assert_eq!(
                steps(Step::Thaw, state),
                Err(ErrorCode::InvalidState),
                "{state:?}"
            );
        }
    }

    #[test]
    fn stop_ends_anything_not_stopped_frozen_included() {
        assert_eq!(steps(Step::Stop, State::Stopped), Ok(vec![]));
        for state in [RUNNING, State::Frozen, State::Transitioning, State::Failed] {
            assert_eq!(steps(Step::Stop, state), Ok(vec![Step::Stop]), "{state:?}");
        }
    }

    #[test]
    fn delete_stops_first_and_is_done_when_absent() {
        assert_eq!(decide(Step::Delete, None), Ok(vec![]));
        assert_eq!(steps(Step::Delete, State::Stopped), Ok(vec![Step::Delete]));
        for state in [RUNNING, State::Frozen, State::Transitioning, State::Failed] {
            assert_eq!(
                steps(Step::Delete, state),
                Ok(vec![Step::Stop, Step::Delete]),
                "{state:?}"
            );
        }
    }

    #[test]
    fn an_upgrade_starts_from_any_settled_state_and_needs_an_instance() {
        let spec = UpgradeSpec {
            image: "a".repeat(64).parse().expect("a fingerprint"),
            user: observed(State::Stopped).user,
        };
        for state in [State::Stopped, RUNNING, State::Frozen] {
            assert_eq!(
                steps(Step::Upgrade(&spec), state),
                Ok(vec![Step::Upgrade(&spec)]),
                "{state:?}"
            );
        }
        assert_eq!(
            steps(Step::Upgrade(&spec), State::Failed),
            Ok(vec![Step::Stop, Step::Upgrade(&spec)])
        );
        assert_eq!(
            decide(Step::Upgrade(&spec), None).map_err(|e| e.code),
            Err(ErrorCode::NotFound)
        );
    }

    #[test]
    fn nothing_acts_on_missing_or_foreign_instances() {
        for command in [Step::Start, Step::Freeze, Step::Thaw, Step::Stop] {
            assert_eq!(
                decide(command, None).map_err(|e| e.code),
                Err(ErrorCode::NotFound)
            );
        }
        let spec = spec();
        let foreign: Claim = Err(Ownership::Foreign);
        for command in [
            Step::Create(&spec),
            Step::Start,
            Step::Freeze,
            Step::Thaw,
            Step::Stop,
            Step::Delete,
        ] {
            assert_eq!(
                decide(command, Some(&foreign)).map_err(|e| e.code),
                Err(ErrorCode::Conflict),
                "{command:?}"
            );
        }
        assert_eq!(
            guest(Some(&foreign)).map_err(|e| e.code),
            Err(ErrorCode::Conflict)
        );
        assert_eq!(guest(None).map_err(|e| e.code), Err(ErrorCode::NotFound));
    }

    #[test]
    fn an_instance_from_another_iglu_can_be_stopped_but_not_started_or_used() {
        let older = guest_tools::Interface::new(guest_tools::INTERFACE.get() - 1);
        let refused = |result: Result<(), CommandError>| {
            result.map_err(|e| (e.code, e.message.contains("update the workspace")))
        };
        for recorded in [Some(older), None] {
            let instance = |state| {
                Ok(Observed {
                    guest_interface: recorded,
                    ..observed(state)
                })
            };
            assert_eq!(
                refused(decide(Step::Start, Some(&instance(State::Stopped))).map(|_| ())),
                Err((ErrorCode::ImageIncompatible, true)),
                "{recorded:?}"
            );
            // Updating is how it gets tools this host speaks.
            let spec = UpgradeSpec {
                image: "a".repeat(64).parse().expect("a fingerprint"),
                user: observed(State::Stopped).user,
            };
            assert_eq!(
                decide(Step::Upgrade(&spec), Some(&instance(State::Stopped))),
                Ok(vec![Step::Upgrade(&spec)]),
                "{recorded:?}"
            );
            assert_eq!(
                refused(tools_guest(Some(&instance(RUNNING))).map(|_| ())),
                Err((ErrorCode::ImageIncompatible, true)),
                "{recorded:?}"
            );
            assert_eq!(
                decide(Step::Stop, Some(&instance(RUNNING))),
                Ok(vec![Step::Stop])
            );
            assert_eq!(
                decide(Step::Delete, Some(&instance(State::Stopped))),
                Ok(vec![Step::Delete])
            );
            assert!(
                guest(Some(&instance(RUNNING))).is_ok(),
                "terminals still work"
            );
        }
        assert!(tools_guest(Some(&Ok(observed(RUNNING)))).is_ok());
    }

    #[test]
    fn only_a_running_guest_is_acted_in() {
        let running = guest(Some(&Ok(observed(RUNNING)))).expect("running");
        assert_eq!(running.boot, BootId::new(7));
        for state in [
            State::Stopped,
            State::Frozen,
            State::Transitioning,
            State::Failed,
        ] {
            assert_eq!(
                guest(Some(&Ok(observed(state)))).map_err(|e| e.code),
                Err(ErrorCode::InvalidState),
                "{state:?}"
            );
        }
    }

    #[test]
    fn a_stale_read_never_undoes_a_newer_delivery() {
        let generation = |n| Some(SecretsGeneration::from_u64(n));
        let delivered = BootFacts {
            ready: true,
            secrets: generation(4),
            ..BootFacts::default()
        };
        let stale = BootFacts {
            ready: true,
            secrets: generation(3),
            ..BootFacts::default()
        };
        assert_eq!(delivered.merge(stale), delivered);
        assert_eq!(stale.merge(delivered), delivered);
        assert_eq!(BootFacts::default().merge(stale), stale);
    }

    #[test]
    fn readiness_follows_the_boot_and_the_address() {
        let running = observed(RUNNING);
        let offline = Observed {
            state: State::Running {
                boot: BootId::new(7),
                has_address: false,
            },
            ..running.clone()
        };
        let readiness_of = |observed: &Observed, ready| match instance(
            observed,
            BootFacts {
                ready,
                ..BootFacts::default()
            },
        ) {
            Instance::Present(Present {
                runtime: Runtime::Running(running),
                ..
            }) => running.readiness,
            other @ (Instance::Absent | Instance::Present(_)) => panic!("not running: {other:?}"),
        };
        assert_eq!(readiness_of(&running, false), Readiness::Booting);
        assert_eq!(readiness_of(&offline, true), Readiness::System);
        assert_eq!(readiness_of(&running, true), Readiness::Network);
    }
}
