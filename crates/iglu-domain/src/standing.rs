//! Where a workspace sits among the others: what needs the person first.
//!
//! The console and CLI show workspaces in this order and mark the ones that
//! need someone, so the rule lives here once instead of in each client.

use serde::{Deserialize, Serialize};

use crate::attention::{AttentionState, Seen, Urgency, urgency};
use crate::lifecycle::Phase;
use crate::time::Timestamp;

/// Whether something is wrong with the workspace itself, beyond its phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Fine,
    /// The last step failed or the runtime reports the instance as broken.
    Trouble,
}

/// Whether the workspace is waiting on the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NeedsYou {
    No,
    Yes,
}

/// Why a workspace needs its person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "snake_case"))]
pub enum Need {
    /// A thread finished and nobody has looked yet.
    Done,
    /// A thread is waiting on an answer.
    Waiting,
    /// Something is wrong with the workspace itself.
    Trouble,
}

/// How awake a workspace is, least to most.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Activity {
    Gone,
    Asleep,
    Frozen,
    Awake,
}

#[must_use]
pub const fn activity(phase: Phase) -> Activity {
    match phase {
        Phase::Creating | Phase::Starting | Phase::Running | Phase::Freezing => Activity::Awake,
        Phase::Frozen => Activity::Frozen,
        Phase::Stopping | Phase::Stopped => Activity::Asleep,
        Phase::Deleting | Phase::Deleted => Activity::Gone,
    }
}

/// A workspace's place in the list. Ordered least to most pressing, so the
/// list sorts by it descending.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Standing {
    pub needs_you: NeedsYou,
    activity: Activity,
    urgency: Option<Urgency>,
    /// The newest change: its most urgent status, or its creation.
    latest: Timestamp,
    /// Why it needs its person, when it does.
    pub need: Option<Need>,
}

/// The facts a standing is decided from.
#[derive(Clone, Copy, Debug)]
pub struct Facts {
    pub phase: Phase,
    pub health: Health,
    /// The workspace's most urgent status, if any session reported one.
    pub top: Option<(AttentionState, Seen, Timestamp)>,
    pub created_at: Timestamp,
}

#[must_use]
pub fn standing(facts: Facts) -> Standing {
    let activity = activity(facts.phase);
    let urgency = facts.top.map(|(state, seen, _)| urgency(state, seen));
    // A frozen or stopped agent can't be waiting on anyone until it's awake.
    let pressing = activity == Activity::Awake
        && matches!(urgency, Some(Urgency::Waiting | Urgency::UnseenDone));
    let need = match (facts.health, pressing, urgency) {
        (Health::Trouble, _, _) => Some(Need::Trouble),
        (Health::Fine, true, Some(Urgency::Waiting)) => Some(Need::Waiting),
        (Health::Fine, true, Some(_)) => Some(Need::Done),
        (Health::Fine, true, None) | (Health::Fine, false, _) => None,
    };
    let needs_you = if need.is_some() {
        NeedsYou::Yes
    } else {
        NeedsYou::No
    };
    Standing {
        needs_you,
        activity,
        urgency,
        latest: facts.top.map_or(facts.created_at, |(_, _, at)| at),
        need,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_unix_millis(ms)
    }

    fn facts(phase: Phase, top: Option<(AttentionState, Seen, i64)>) -> Facts {
        Facts {
            phase,
            health: Health::Fine,
            top: top.map(|(state, seen, ms)| (state, seen, at(ms))),
            created_at: at(0),
        }
    }

    #[test]
    fn a_waiting_agent_needs_you_and_comes_first() {
        let waiting = standing(facts(
            Phase::Running,
            Some((AttentionState::Waiting, Seen::Seen, 1)),
        ));
        let working = standing(facts(
            Phase::Running,
            Some((AttentionState::Working, Seen::Seen, 9)),
        ));
        assert_eq!(waiting.needs_you, NeedsYou::Yes);
        assert_eq!(waiting.need, Some(Need::Waiting));
        assert_eq!(working.needs_you, NeedsYou::No);
        assert_eq!(working.need, None);
        assert!(waiting > working);
    }

    #[test]
    fn unseen_done_needs_you_but_seen_done_does_not() {
        let unseen = standing(facts(
            Phase::Running,
            Some((AttentionState::Done, Seen::Unseen, 1)),
        ));
        let seen = standing(facts(
            Phase::Running,
            Some((AttentionState::Done, Seen::Seen, 1)),
        ));
        assert_eq!(unseen.needs_you, NeedsYou::Yes);
        assert_eq!(unseen.need, Some(Need::Done));
        assert_eq!(seen.needs_you, NeedsYou::No);
    }

    #[test]
    fn a_frozen_waiting_agent_does_not_need_you_until_it_thaws() {
        let frozen = standing(facts(
            Phase::Frozen,
            Some((AttentionState::Waiting, Seen::Unseen, 1)),
        ));
        assert_eq!(frozen.needs_you, NeedsYou::No);
        let running = standing(facts(Phase::Running, None));
        assert!(running > frozen);
    }

    #[test]
    fn trouble_needs_you_whatever_the_phase() {
        let broken = standing(Facts {
            health: Health::Trouble,
            ..facts(Phase::Stopped, None)
        });
        assert_eq!(broken.needs_you, NeedsYou::Yes);
        assert_eq!(broken.need, Some(Need::Trouble));
        assert!(
            broken
                > standing(facts(
                    Phase::Running,
                    Some((AttentionState::Working, Seen::Seen, 5))
                ))
        );
    }

    #[test]
    fn awake_comes_before_frozen_before_stopped() {
        let awake = standing(facts(Phase::Starting, None));
        let frozen = standing(facts(Phase::Frozen, None));
        let stopped = standing(facts(Phase::Stopped, None));
        assert!(awake > frozen && frozen > stopped);
    }

    #[test]
    fn ties_go_to_the_newest_change() {
        let older = standing(facts(
            Phase::Running,
            Some((AttentionState::Working, Seen::Seen, 1)),
        ));
        let newer = standing(facts(
            Phase::Running,
            Some((AttentionState::Working, Seen::Seen, 2)),
        ));
        assert!(newer > older);
        let fresh = standing(Facts {
            created_at: at(5),
            ..facts(Phase::Running, None)
        });
        assert!(fresh > standing(facts(Phase::Running, None)));
    }
}
