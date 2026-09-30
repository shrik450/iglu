//! Workspace lifecycle: what the user wants, what the host reports, and the
//! one next step that moves the second toward the first.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::capacity::Capacity;
use crate::parse::text_type;

/// What the user wants a workspace to be. Set through the API; owned by the
/// control plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredState {
    Running,
    Frozen,
    Stopped,
    Deleted,
}

impl DesiredState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Frozen => "frozen",
            Self::Stopped => "stopped",
            Self::Deleted => "deleted",
        }
    }
}

impl fmt::Display for DesiredState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DesiredState {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(Self::Running),
            "frozen" => Ok(Self::Frozen),
            "stopped" => Ok(Self::Stopped),
            "deleted" => Ok(Self::Deleted),
            _ => Err(ParseError::new(
                "desired state",
                "expected running, frozen, stopped or deleted",
            )),
        }
    }
}

text_type!(DesiredState);

/// The desired states in which the workspace still exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Live {
    Running,
    Frozen,
    Stopped,
}

impl DesiredState {
    const fn live(self) -> Option<Live> {
        match self {
            Self::Running => Some(Live::Running),
            Self::Frozen => Some(Live::Frozen),
            Self::Stopped => Some(Live::Stopped),
            Self::Deleted => None,
        }
    }
}

/// Increases every time the desired state or anything it depends on changes.
/// Used for optimistic concurrency at the API and to discard stale
/// observations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision(u64);

impl Revision {
    pub const INITIAL: Self = Self(1);

    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Which delivery of an owner's secrets a workspace should hold. Increases
/// whenever the owner's secrets change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretsGeneration(u64);

impl SecretsGeneration {
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The inputs to [`plan`] that come from the control plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desired {
    pub state: DesiredState,
    pub secrets: SecretsGeneration,
}

/// What the host reports about a workspace's instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Instance {
    Absent,
    Present(Present),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Present {
    pub runtime: Runtime,
    pub provisioning: Provisioning,
}

/// The instance's runtime status as Incus reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Runtime {
    Stopped,
    Running(Running),
    Frozen,
    /// Incus is in the middle of changing state.
    Transitioning,
    /// Incus reports the instance as broken.
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Running {
    pub readiness: Readiness,
    /// The secrets delivery the guest currently holds, if any. Secrets live on
    /// a tmpfs, so every boot starts with none.
    pub secrets: Option<SecretsGeneration>,
}

/// How far a running guest has come up. Ordered: each step implies the ones before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    Booting,
    /// systemd finished starting the system.
    System,
    /// The guest also has an address.
    Network,
}

/// Whether the repository has been cloned into the workspace. Recorded on the
/// instance itself, so it survives restarts of every iglu process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provisioning {
    Pending,
    Complete,
}

/// One step the host can take. The shell attaches the payload each step needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Create the instance from its image, stopped, with limits and policy applied.
    Create,
    Start,
    DeliverSecrets,
    /// Clone the repository and check out the branch.
    Provision,
    /// Pause the instance and reclaim its memory to swap.
    Freeze,
    Thaw,
    Stop,
    Delete,
}

/// Why nothing can happen yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Wait {
    Booting,
    Transition,
    Capacity(Capacity),
}

/// Why nothing will happen until something outside the loop changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocker {
    RuntimeFailed,
}

/// Why the control plane should change its own intent to match reality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdoptReason {
    /// A frozen workspace's memory lives in RAM and swap, which don't survive
    /// a host restart. The workspace is stopped now; restarting it just to
    /// freeze it again would waste the host.
    FrozenStateLost,
}

/// The single next step for a workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    Stable,
    Perform(Effect),
    Wait(Wait),
    Blocked(Blocker),
    Adopt(DesiredState, AdoptReason),
}

/// Decides the next step toward `desired`, given what the host reports and
/// whether the host has room for one more running workspace.
///
/// `capacity` is only consulted for steps that bring memory back: start and thaw.
pub fn plan(desired: Desired, instance: Instance, capacity: Capacity) -> Plan {
    match (desired.state.live(), instance) {
        (None, Instance::Absent) => Plan::Stable,
        (None, Instance::Present(_)) => Plan::Perform(Effect::Delete),
        (Some(_), Instance::Absent) => Plan::Perform(Effect::Create),
        (Some(live), Instance::Present(present)) => {
            plan_live(live, desired.secrets, present, capacity)
        }
    }
}

fn plan_live(live: Live, secrets: SecretsGeneration, present: Present, capacity: Capacity) -> Plan {
    let admitted = |effect| match capacity {
        Capacity::Fits => Plan::Perform(effect),
        Capacity::Short { .. } => Plan::Wait(Wait::Capacity(capacity)),
    };

    match (live, present.runtime) {
        (_, Runtime::Transitioning) => Plan::Wait(Wait::Transition),

        (Live::Stopped, Runtime::Stopped) => Plan::Stable,
        (Live::Stopped, Runtime::Running(_) | Runtime::Frozen | Runtime::Failed) => {
            Plan::Perform(Effect::Stop)
        }

        (Live::Running | Live::Frozen, Runtime::Failed) => Plan::Blocked(Blocker::RuntimeFailed),

        (Live::Running, Runtime::Stopped) => admitted(Effect::Start),
        (Live::Frozen, Runtime::Stopped) => {
            Plan::Adopt(DesiredState::Stopped, AdoptReason::FrozenStateLost)
        }

        (Live::Running, Runtime::Frozen) => admitted(Effect::Thaw),
        (Live::Frozen, Runtime::Frozen) => Plan::Stable,

        (Live::Running, Runtime::Running(running)) => {
            bring_up(secrets, running, present.provisioning).unwrap_or(Plan::Stable)
        }
        (Live::Frozen, Runtime::Running(running)) => {
            bring_up(secrets, running, present.provisioning)
                .unwrap_or(Plan::Perform(Effect::Freeze))
        }
    }
}

/// The steps every running workspace needs before it's usable: finish
/// booting, hold the current secrets, and have its repository.
fn bring_up(
    secrets: SecretsGeneration,
    running: Running,
    provisioning: Provisioning,
) -> Option<Plan> {
    if running.readiness < Readiness::System {
        return Some(Plan::Wait(Wait::Booting));
    }
    if running.secrets != Some(secrets) {
        return Some(Plan::Perform(Effect::DeliverSecrets));
    }
    match provisioning {
        Provisioning::Complete => None,
        Provisioning::Pending if running.readiness < Readiness::Network => {
            Some(Plan::Wait(Wait::Booting))
        }
        Provisioning::Pending => Some(Plan::Perform(Effect::Provision)),
    }
}

/// What the user sees a workspace doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Creating,
    Starting,
    Running,
    Freezing,
    Frozen,
    Stopping,
    Stopped,
    Deleting,
    Deleted,
}

impl Phase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Freezing => "freezing",
            Self::Frozen => "frozen",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Deleting => "deleting",
            Self::Deleted => "deleted",
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Derives the user-facing phase from intent and observation.
pub fn phase(desired: DesiredState, instance: Instance) -> Phase {
    match (desired.live(), instance) {
        (None, Instance::Absent) => Phase::Deleted,
        (None, Instance::Present(_)) => Phase::Deleting,
        (Some(_), Instance::Absent) => Phase::Creating,
        (Some(live), Instance::Present(present)) => live_phase(live, present),
    }
}

fn live_phase(live: Live, present: Present) -> Phase {
    match (live, present.runtime, present.provisioning) {
        (Live::Running, _, Provisioning::Pending) => Phase::Creating,

        (Live::Running, Runtime::Running(running), Provisioning::Complete) => {
            if running.readiness < Readiness::System || running.secrets.is_none() {
                Phase::Starting
            } else {
                Phase::Running
            }
        }
        (
            Live::Running,
            Runtime::Stopped | Runtime::Frozen | Runtime::Transitioning,
            Provisioning::Complete,
        ) => Phase::Starting,
        (Live::Running | Live::Frozen, Runtime::Failed, _) => Phase::Stopped,

        (Live::Frozen, Runtime::Frozen, _) => Phase::Frozen,
        (Live::Frozen, Runtime::Running(_) | Runtime::Transitioning, _) => Phase::Freezing,
        (Live::Frozen, Runtime::Stopped, _) => Phase::Stopped,

        (Live::Stopped, Runtime::Stopped | Runtime::Failed, _) => Phase::Stopped,
        (Live::Stopped, Runtime::Running(_) | Runtime::Frozen | Runtime::Transitioning, _) => {
            Phase::Stopping
        }
    }
}

/// A requested change of desired state that the current phase doesn't allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransitionError {
    #[error("the workspace is being deleted")]
    Deleting,
    #[error("only a running workspace can be frozen")]
    FreezeNeedsRunning,
}

/// Whether a user may ask for `requested` while the workspace is in `current`.
pub fn allow_transition(current: Phase, requested: DesiredState) -> Result<(), TransitionError> {
    match (current, requested) {
        (Phase::Deleting | Phase::Deleted, _) => Err(TransitionError::Deleting),
        (_, DesiredState::Deleted | DesiredState::Running | DesiredState::Stopped) => Ok(()),
        (Phase::Running | Phase::Freezing | Phase::Frozen, DesiredState::Frozen) => Ok(()),
        (
            Phase::Creating | Phase::Starting | Phase::Stopping | Phase::Stopped,
            DesiredState::Frozen,
        ) => Err(TransitionError::FreezeNeedsRunning),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capacity::Bytes;

    const GEN: SecretsGeneration = SecretsGeneration::from_u64(3);

    fn desired(state: DesiredState) -> Desired {
        Desired {
            state,
            secrets: GEN,
        }
    }

    fn present(runtime: Runtime, provisioning: Provisioning) -> Instance {
        Instance::Present(Present {
            runtime,
            provisioning,
        })
    }

    fn running(readiness: Readiness, secrets: Option<SecretsGeneration>) -> Runtime {
        Runtime::Running(Running { readiness, secrets })
    }

    fn ready() -> Runtime {
        running(Readiness::Network, Some(GEN))
    }

    const SHORT: Capacity = Capacity::Short {
        available: Bytes::gib(1),
        needed: Bytes::gib(3),
    };

    #[test]
    fn a_new_workspace_is_created_then_started_then_brought_up() {
        let want = desired(DesiredState::Running);
        assert_eq!(
            plan(want, Instance::Absent, Capacity::Fits),
            Plan::Perform(Effect::Create)
        );
        assert_eq!(
            plan(
                want,
                present(Runtime::Stopped, Provisioning::Pending),
                Capacity::Fits
            ),
            Plan::Perform(Effect::Start)
        );
        assert_eq!(
            plan(
                want,
                present(running(Readiness::Booting, None), Provisioning::Pending),
                Capacity::Fits
            ),
            Plan::Wait(Wait::Booting)
        );
        assert_eq!(
            plan(
                want,
                present(running(Readiness::System, None), Provisioning::Pending),
                Capacity::Fits
            ),
            Plan::Perform(Effect::DeliverSecrets)
        );
        assert_eq!(
            plan(
                want,
                present(running(Readiness::System, Some(GEN)), Provisioning::Pending),
                Capacity::Fits
            ),
            Plan::Wait(Wait::Booting)
        );
        assert_eq!(
            plan(
                want,
                present(ready(), Provisioning::Pending),
                Capacity::Fits
            ),
            Plan::Perform(Effect::Provision)
        );
        assert_eq!(
            plan(
                want,
                present(ready(), Provisioning::Complete),
                Capacity::Fits
            ),
            Plan::Stable
        );
    }

    #[test]
    fn changed_secrets_are_redelivered_to_running_workspaces() {
        let stale = running(Readiness::Network, Some(SecretsGeneration::from_u64(2)));
        assert_eq!(
            plan(
                desired(DesiredState::Running),
                present(stale, Provisioning::Complete),
                Capacity::Fits
            ),
            Plan::Perform(Effect::DeliverSecrets)
        );
    }

    #[test]
    fn start_and_thaw_wait_for_capacity() {
        let want = desired(DesiredState::Running);
        assert_eq!(
            plan(
                want,
                present(Runtime::Stopped, Provisioning::Complete),
                SHORT
            ),
            Plan::Wait(Wait::Capacity(SHORT))
        );
        assert_eq!(
            plan(
                want,
                present(Runtime::Frozen, Provisioning::Complete),
                SHORT
            ),
            Plan::Wait(Wait::Capacity(SHORT))
        );
        assert_eq!(
            plan(
                want,
                present(Runtime::Frozen, Provisioning::Complete),
                Capacity::Fits
            ),
            Plan::Perform(Effect::Thaw)
        );
    }

    #[test]
    fn capacity_never_blocks_stopping_freezing_or_deleting() {
        for (state, runtime, effect) in [
            (DesiredState::Stopped, ready(), Effect::Stop),
            (DesiredState::Frozen, ready(), Effect::Freeze),
            (DesiredState::Deleted, ready(), Effect::Delete),
            (DesiredState::Stopped, Runtime::Frozen, Effect::Stop),
        ] {
            assert_eq!(
                plan(
                    desired(state),
                    present(runtime, Provisioning::Complete),
                    SHORT
                ),
                Plan::Perform(effect)
            );
        }
    }

    #[test]
    fn a_workspace_finishes_provisioning_before_it_freezes() {
        assert_eq!(
            plan(
                desired(DesiredState::Frozen),
                present(ready(), Provisioning::Pending),
                Capacity::Fits
            ),
            Plan::Perform(Effect::Provision)
        );
        assert_eq!(
            plan(
                desired(DesiredState::Frozen),
                present(ready(), Provisioning::Complete),
                Capacity::Fits
            ),
            Plan::Perform(Effect::Freeze)
        );
    }

    #[test]
    fn a_frozen_workspace_found_stopped_is_adopted_as_stopped() {
        assert_eq!(
            plan(
                desired(DesiredState::Frozen),
                present(Runtime::Stopped, Provisioning::Complete),
                Capacity::Fits
            ),
            Plan::Adopt(DesiredState::Stopped, AdoptReason::FrozenStateLost)
        );
    }

    #[test]
    fn deletion_is_complete_once_the_instance_is_absent() {
        let want = desired(DesiredState::Deleted);
        assert_eq!(plan(want, Instance::Absent, Capacity::Fits), Plan::Stable);
        for runtime in [
            Runtime::Stopped,
            ready(),
            Runtime::Frozen,
            Runtime::Failed,
            Runtime::Transitioning,
        ] {
            assert_eq!(
                plan(
                    want,
                    present(runtime, Provisioning::Pending),
                    Capacity::Fits
                ),
                Plan::Perform(Effect::Delete)
            );
        }
    }

    #[test]
    fn transitions_in_progress_are_waited_out() {
        for state in [
            DesiredState::Running,
            DesiredState::Frozen,
            DesiredState::Stopped,
        ] {
            assert_eq!(
                plan(
                    desired(state),
                    present(Runtime::Transitioning, Provisioning::Complete),
                    Capacity::Fits
                ),
                Plan::Wait(Wait::Transition)
            );
        }
    }

    #[test]
    fn failed_runtimes_block_until_stopped_or_deleted() {
        let failed = present(Runtime::Failed, Provisioning::Complete);
        assert_eq!(
            plan(desired(DesiredState::Running), failed, Capacity::Fits),
            Plan::Blocked(Blocker::RuntimeFailed)
        );
        assert_eq!(
            plan(desired(DesiredState::Stopped), failed, Capacity::Fits),
            Plan::Perform(Effect::Stop)
        );
    }

    #[test]
    fn phases_describe_progress() {
        assert_eq!(
            phase(DesiredState::Running, Instance::Absent),
            Phase::Creating
        );
        assert_eq!(
            phase(
                DesiredState::Running,
                present(ready(), Provisioning::Pending)
            ),
            Phase::Creating
        );
        assert_eq!(
            phase(
                DesiredState::Running,
                present(ready(), Provisioning::Complete)
            ),
            Phase::Running
        );
        assert_eq!(
            phase(
                DesiredState::Running,
                present(Runtime::Stopped, Provisioning::Complete)
            ),
            Phase::Starting
        );
        assert_eq!(
            phase(
                DesiredState::Frozen,
                present(ready(), Provisioning::Complete)
            ),
            Phase::Freezing
        );
        assert_eq!(
            phase(
                DesiredState::Frozen,
                present(Runtime::Frozen, Provisioning::Complete)
            ),
            Phase::Frozen
        );
        assert_eq!(
            phase(
                DesiredState::Stopped,
                present(ready(), Provisioning::Complete)
            ),
            Phase::Stopping
        );
        assert_eq!(
            phase(
                DesiredState::Stopped,
                present(Runtime::Stopped, Provisioning::Pending)
            ),
            Phase::Stopped
        );
        assert_eq!(
            phase(
                DesiredState::Deleted,
                present(ready(), Provisioning::Complete)
            ),
            Phase::Deleting
        );
        assert_eq!(
            phase(DesiredState::Deleted, Instance::Absent),
            Phase::Deleted
        );
    }

    #[test]
    fn only_running_workspaces_can_be_frozen() {
        assert_eq!(
            allow_transition(Phase::Running, DesiredState::Frozen),
            Ok(())
        );
        assert_eq!(
            allow_transition(Phase::Stopped, DesiredState::Frozen),
            Err(TransitionError::FreezeNeedsRunning)
        );
        assert_eq!(
            allow_transition(Phase::Stopped, DesiredState::Running),
            Ok(())
        );
        assert_eq!(
            allow_transition(Phase::Deleting, DesiredState::Running),
            Err(TransitionError::Deleting)
        );
    }

    #[test]
    fn desired_states_round_trip_as_text() {
        for state in [
            DesiredState::Running,
            DesiredState::Frozen,
            DesiredState::Stopped,
            DesiredState::Deleted,
        ] {
            assert_eq!(state.as_str().parse(), Ok(state));
        }
        assert!("paused".parse::<DesiredState>().is_err());
    }
}
