//! Putting idle workspaces to sleep, and waking frozen ones when used.

use serde::{Deserialize, Serialize};

use crate::attention::AttentionState;
use crate::lifecycle::{DesiredState, Phase};
use crate::time::{Millis, Timestamp};

/// How long a workspace may sit unused. `None` never does that step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdlePolicy {
    /// From running to frozen.
    pub freeze_after: Option<Millis>,
    /// From frozen to stopped, counted from the same last use.
    pub stop_after: Option<Millis>,
}

/// A project's say over how its workspaces idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(tag = "kind", rename_all = "snake_case")
)]
pub enum IdleRule {
    /// Whatever the host's operator set.
    #[default]
    Default,
    /// Never freeze or stop for being unused.
    Never,
    /// Freeze after this many unused minutes; stopping stays the default.
    After {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        minutes: std::num::NonZeroU32,
    },
}

impl IdleRule {
    /// The policy for a workspace, given the operator's.
    #[must_use]
    pub fn policy(self, operator: IdlePolicy) -> IdlePolicy {
        match self {
            Self::Default => operator,
            Self::Never => IdlePolicy {
                freeze_after: None,
                stop_after: None,
            },
            Self::After { minutes } => IdlePolicy {
                freeze_after: Some(Millis::from_secs(minutes.get().saturating_mul(60))),
                stop_after: operator.stop_after,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleStep {
    Freeze,
    Stop,
}

/// Whether anything in a workspace is busy for its person: a thread working,
/// or one waiting on them. A busy workspace never goes to sleep, so what it
/// needs stays in front of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    Yes,
    No,
}

/// A workspace is busy while any of its threads works or waits on its person.
#[must_use]
pub fn busy(threads: impl IntoIterator<Item = AttentionState>) -> Busy {
    let pressing = threads.into_iter().any(|state| match state {
        AttentionState::Working | AttentionState::Waiting => true,
        AttentionState::Done | AttentionState::Idle | AttentionState::Exited => false,
    });
    if pressing { Busy::Yes } else { Busy::No }
}

/// The step a workspace takes for being unused, if any.
#[must_use]
pub fn decide(
    policy: IdlePolicy,
    now: Timestamp,
    phase: Phase,
    last_used: Timestamp,
    busy: Busy,
) -> Option<IdleStep> {
    let unused = now.since(last_used);
    let past = |limit: Option<Millis>| limit.is_some_and(|limit| unused >= limit);
    match (phase, busy) {
        (Phase::Running, Busy::No) if past(policy.freeze_after) => Some(IdleStep::Freeze),
        (Phase::Frozen, _) if past(policy.stop_after) => Some(IdleStep::Stop),
        (
            Phase::Creating
            | Phase::Starting
            | Phase::Running
            | Phase::Freezing
            | Phase::Frozen
            | Phase::Stopping
            | Phase::Stopped
            | Phase::Deleting
            | Phase::Deleted,
            Busy::Yes | Busy::No,
        ) => None,
    }
}

/// What opening a workspace, by attaching or through a preview, asks for:
/// a frozen one thaws. A stopped one stays stopped: starting takes long
/// enough that it should be asked for.
#[must_use]
pub const fn thaw_on_open(desired: DesiredState) -> Option<DesiredState> {
    match desired {
        DesiredState::Frozen => Some(DesiredState::Running),
        DesiredState::Running | DesiredState::Stopped | DesiredState::Deleted => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_unix_millis(ms)
    }

    fn policy() -> IdlePolicy {
        IdlePolicy {
            freeze_after: Some(Millis::from_secs(2 * 3600)),
            stop_after: Some(Millis::from_secs(24 * 3600)),
        }
    }

    #[test]
    fn an_unused_workspace_freezes_then_stops() {
        assert_eq!(
            decide(policy(), at(HOUR), Phase::Running, at(0), Busy::No),
            None
        );
        assert_eq!(
            decide(policy(), at(2 * HOUR), Phase::Running, at(0), Busy::No),
            Some(IdleStep::Freeze)
        );
        assert_eq!(
            decide(policy(), at(3 * HOUR), Phase::Frozen, at(0), Busy::No),
            None
        );
        assert_eq!(
            decide(policy(), at(24 * HOUR), Phase::Frozen, at(0), Busy::No),
            Some(IdleStep::Stop)
        );
    }

    #[test]
    fn busy_workspaces_and_other_phases_are_left_alone() {
        assert_eq!(
            decide(policy(), at(9 * HOUR), Phase::Running, at(0), Busy::Yes),
            None
        );
        assert_eq!(
            decide(policy(), at(99 * HOUR), Phase::Stopped, at(0), Busy::No),
            None
        );
        assert_eq!(
            decide(policy(), at(99 * HOUR), Phase::Starting, at(0), Busy::No),
            None
        );
        let never = IdlePolicy {
            freeze_after: None,
            stop_after: None,
        };
        assert_eq!(
            decide(never, at(99 * HOUR), Phase::Running, at(0), Busy::No),
            None
        );
    }

    #[test]
    fn working_or_waiting_threads_keep_it_busy() {
        assert_eq!(
            busy([AttentionState::Done, AttentionState::Waiting]),
            Busy::Yes
        );
        assert_eq!(busy([AttentionState::Working]), Busy::Yes);
        assert_eq!(
            busy([
                AttentionState::Done,
                AttentionState::Idle,
                AttentionState::Exited
            ]),
            Busy::No
        );
        assert_eq!(busy([]), Busy::No);
    }

    #[test]
    fn projects_can_override_the_policy() {
        assert_eq!(IdleRule::Default.policy(policy()), policy());
        let never = IdleRule::Never.policy(policy());
        assert_eq!((never.freeze_after, never.stop_after), (None, None));
        let thirty = std::num::NonZeroU32::new(30).expect("non-zero");
        let soon = IdleRule::After { minutes: thirty }.policy(policy());
        assert_eq!(soon.freeze_after, Some(Millis::from_secs(1800)));
        assert_eq!(soon.stop_after, policy().stop_after);
    }

    #[test]
    fn opening_thaws_only_frozen_workspaces() {
        assert_eq!(
            thaw_on_open(DesiredState::Frozen),
            Some(DesiredState::Running)
        );
        assert_eq!(thaw_on_open(DesiredState::Stopped), None);
        assert_eq!(thaw_on_open(DesiredState::Running), None);
    }
}
