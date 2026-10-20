//! The lifecycle planner against a model of a host that does what it's
//! told: from anywhere, following the plan reaches what the person wants,
//! and the planner never asks the host for a step that can't apply.

use iglu_domain::attention::{AttentionState, Seen};
use iglu_domain::capacity::{Bytes, Capacity, Room};
use iglu_domain::idle::{Busy, IdlePolicy, IdleStep, busy, decide, thaw_on_open};
use iglu_domain::lifecycle::{
    Build, Columns, Desired, DesiredState, Effect, Instance, Phase, Plan, Present, Provisioning,
    Readiness, Running, Runtime, SecretsGeneration, TransitionError, Wait, allow_transition, phase,
    plan, plan_within,
};
use iglu_domain::standing::{Facts, Health, NeedsYou, standing};
use iglu_domain::time::{Millis, Timestamp};
use proptest::prelude::*;

const STATES: [DesiredState; 4] = [
    DesiredState::Running,
    DesiredState::Frozen,
    DesiredState::Stopped,
    DesiredState::Deleted,
];

const PHASES: [Phase; 9] = [
    Phase::Creating,
    Phase::Starting,
    Phase::Running,
    Phase::Freezing,
    Phase::Frozen,
    Phase::Stopping,
    Phase::Stopped,
    Phase::Deleting,
    Phase::Deleted,
];

const SHORT: Capacity = Capacity::Short {
    available: Bytes::gib(1),
    needed: Bytes::gib(3),
};

fn generation() -> impl Strategy<Value = SecretsGeneration> {
    (0..3u64).prop_map(SecretsGeneration::from_u64)
}

fn desired() -> impl Strategy<Value = Desired> {
    (
        prop::sample::select(&STATES[..]),
        generation(),
        prop_oneof![Just(Build::Current), Just(Build::Outdated)],
    )
        .prop_map(|(state, secrets, build)| Desired {
            state,
            secrets,
            build,
        })
}

fn running() -> impl Strategy<Value = Running> {
    (
        prop::sample::select(&[Readiness::Booting, Readiness::System, Readiness::Network][..]),
        prop::option::of(generation()),
        prop_oneof![Just(Columns::Pending), Just(Columns::Opened)],
    )
        .prop_map(|(readiness, secrets, columns)| Running {
            readiness,
            secrets,
            columns,
        })
}

fn instance() -> impl Strategy<Value = Instance> {
    let runtime = prop_oneof![
        Just(Runtime::Stopped),
        running().prop_map(Runtime::Running),
        Just(Runtime::Frozen),
        Just(Runtime::Transitioning),
        Just(Runtime::Failed),
    ];
    let provisioning = prop_oneof![Just(Provisioning::Pending), Just(Provisioning::Complete)];
    prop_oneof![
        1 => Just(Instance::Absent),
        6 => (runtime, provisioning).prop_map(|(runtime, provisioning)| Instance::Present(Present { runtime, provisioning })),
    ]
}

/// A host that does each step as asked, and a guest that boots when given
/// time. A frozen guest keeps what it had; a stopped one loses its secrets
/// and its columns; Incus finishes a transition on its own.
struct Host {
    instance: Instance,
    /// What the guest had when it was frozen, for when it thaws.
    frozen: Running,
}

impl Host {
    fn fresh_boot() -> Running {
        Running {
            readiness: Readiness::Booting,
            secrets: None,
            columns: Columns::Pending,
        }
    }

    fn present(&self) -> Option<Present> {
        match self.instance {
            Instance::Present(present) => Some(present),
            Instance::Absent => None,
        }
    }

    fn set(&mut self, runtime: Runtime) {
        if let Some(mut present) = self.present() {
            present.runtime = runtime;
            self.instance = Instance::Present(present);
        }
    }

    /// Whether the host can take `effect` now: the step's precondition.
    fn applies(&self, effect: Effect, desired: Desired) -> bool {
        let present = self.present();
        let runtime = present.map(|p| p.runtime);
        match effect {
            Effect::Create => present.is_none(),
            Effect::Delete => present.is_some(),
            Effect::Start => runtime == Some(Runtime::Stopped),
            Effect::Thaw => runtime == Some(Runtime::Frozen),
            Effect::Freeze => matches!(runtime, Some(Runtime::Running(_))),
            Effect::Stop => matches!(
                runtime,
                Some(Runtime::Running(_) | Runtime::Frozen | Runtime::Failed)
            ),
            Effect::DeliverSecrets => {
                matches!(runtime, Some(Runtime::Running(r)) if r.readiness >= Readiness::System)
            }
            Effect::Provision => {
                matches!(runtime, Some(Runtime::Running(r)) if r.readiness == Readiness::Network && r.secrets == Some(desired.secrets))
                    && present.map(|p| p.provisioning) == Some(Provisioning::Pending)
            }
            Effect::OpenColumns => {
                matches!(runtime, Some(Runtime::Running(r)) if r.readiness >= Readiness::System && r.columns == Columns::Pending)
                    && present.map(|p| p.provisioning) == Some(Provisioning::Complete)
            }
            Effect::Upgrade => {
                desired.build == Build::Outdated
                    && matches!(
                        runtime,
                        Some(Runtime::Stopped | Runtime::Running(_) | Runtime::Frozen)
                    )
            }
        }
    }

    fn perform(&mut self, effect: Effect, desired: &mut Desired) {
        match effect {
            Effect::Create => {
                self.instance = Instance::Present(Present {
                    runtime: Runtime::Stopped,
                    provisioning: Provisioning::Pending,
                });
            }
            Effect::Delete => self.instance = Instance::Absent,
            Effect::Start => self.set(Runtime::Running(Self::fresh_boot())),
            Effect::Thaw => self.set(Runtime::Running(self.frozen)),
            Effect::Freeze => {
                if let Some(Present {
                    runtime: Runtime::Running(running),
                    ..
                }) = self.present()
                {
                    self.frozen = running;
                }
                self.set(Runtime::Frozen);
            }
            Effect::Stop => self.set(Runtime::Stopped),
            Effect::DeliverSecrets => self.update_running(|r| r.secrets = Some(desired.secrets)),
            Effect::Provision => {
                if let Some(mut present) = self.present() {
                    present.provisioning = Provisioning::Complete;
                    self.instance = Instance::Present(present);
                }
            }
            Effect::OpenColumns => self.update_running(|r| r.columns = Columns::Opened),
            Effect::Upgrade => {
                // Restarted into the new system, whatever it was doing.
                self.set(Runtime::Running(Self::fresh_boot()));
                desired.build = Build::Current;
            }
        }
    }

    fn update_running(&mut self, change: impl FnOnce(&mut Running)) {
        if let Some(Present {
            runtime: Runtime::Running(mut running),
            ..
        }) = self.present()
        {
            change(&mut running);
            self.set(Runtime::Running(running));
        }
    }

    /// Time passes: a booting guest comes up a step, and a transition ends.
    fn wait(&mut self, settles_to: Runtime) {
        match self.present().map(|p| p.runtime) {
            Some(Runtime::Running(running)) if running.readiness < Readiness::Network => {
                self.update_running(|r| {
                    r.readiness = match r.readiness {
                        Readiness::Booting => Readiness::System,
                        Readiness::System | Readiness::Network => Readiness::Network,
                    };
                });
            }
            Some(Runtime::Transitioning) => self.set(settles_to),
            _ => {}
        }
    }
}

proptest! {
    #[test]
    fn following_the_plan_reaches_what_the_person_wants(
        mut desired in desired(),
        instance in instance(),
        frozen in running(),
        settles_to in prop_oneof![Just(Runtime::Stopped), Just(Runtime::Frozen), running().prop_map(Runtime::Running)],
    ) {
        let mut host = Host { instance, frozen };
        let mut steps = Vec::new();
        for _ in 0..20 {
            let next = plan(desired, host.instance, Capacity::Fits);
            steps.push(next);
            match next {
                Plan::Stable => {
                    prop_assert_eq!(phase(desired.state, host.instance), desired.state.settled(), "stable at {:?} after {:?}", host.instance, steps);
                    return Ok(());
                }
                Plan::Perform(effect) => {
                    prop_assert!(host.applies(effect, desired), "{:?} can't apply to {:?} wanting {:?}, after {:?}", effect, host.instance, desired, steps);
                    host.perform(effect, &mut desired);
                }
                Plan::Wait(Wait::Booting | Wait::Transition) => host.wait(settles_to),
                Plan::Wait(Wait::Capacity(_)) => prop_assert!(false, "waited for capacity that's there"),
                Plan::Adopt(state, _) => desired.state = state,
                Plan::Blocked(_) => {
                    // Only a broken instance blocks, and only what a person must look at.
                    prop_assert_eq!(host.present().map(|p| p.runtime), Some(Runtime::Failed));
                    prop_assert!(matches!(desired.state, DesiredState::Running | DesiredState::Frozen));
                    return Ok(());
                }
            }
        }
        prop_assert!(false, "never settled: {:?}", steps);
    }

    #[test]
    fn without_room_nothing_brings_memory_back(desired in desired(), instance in instance()) {
        let next = plan(desired, instance, SHORT);
        prop_assert!(!matches!(next, Plan::Perform(Effect::Start | Effect::Thaw | Effect::Upgrade)), "{next:?}");
        // And whatever doesn't need room is planned as if there were room.
        let roomy = plan(desired, instance, Capacity::Fits);
        if matches!(roomy, Plan::Perform(Effect::Start | Effect::Thaw | Effect::Upgrade)) {
            prop_assert_eq!(next, Plan::Wait(Wait::Capacity(SHORT)));
        } else {
            prop_assert_eq!(next, roomy);
        }
    }

    #[test]
    fn a_pass_charges_only_what_brings_memory_back(
        workspaces in prop::collection::vec((desired(), instance()), 0..12),
        available in 0..16u64,
        reservation in 1..4u64,
    ) {
        let mut room = Room::new(Bytes::gib(available), Bytes::gib(reservation), Bytes::gib(1));
        let mut started = 0u64;
        for (desired, instance) in workspaces {
            let before = room;
            let next = plan_within(desired, instance, &mut room);
            if matches!(next, Plan::Perform(Effect::Start | Effect::Thaw | Effect::Upgrade)) {
                started += 1;
                prop_assert_eq!(room, before.charge());
            } else {
                prop_assert_eq!(room, before);
            }
        }
        // Each start keeps the 1 GiB of headroom free.
        prop_assert!(started == 0 || started * reservation < available);
    }

    /// The person sees a workspace running only when it is: the planner
    /// has nothing left to do for it but, perhaps, hand it newer secrets.
    #[test]
    fn a_workspace_shows_as_running_only_once_it_is_up(desired in desired(), instance in instance()) {
        if phase(desired.state, instance) == Phase::Running {
            let next = plan(Desired { build: Build::Current, ..desired }, instance, Capacity::Fits);
            prop_assert!(matches!(next, Plan::Stable | Plan::Perform(Effect::DeliverSecrets)), "{:?} shows running while {:?}", instance, next);
        }
    }

    #[test]
    fn a_stable_plan_is_the_settled_phase(desired in desired(), instance in instance()) {
        if plan(desired, instance, Capacity::Fits) == Plan::Stable {
            prop_assert_eq!(phase(desired.state, instance), desired.state.settled());
        }
    }

    #[test]
    fn deletion_is_never_refused_and_never_undone(current in prop::sample::select(&PHASES[..]), requested in prop::sample::select(&STATES[..])) {
        let allowed = allow_transition(current, requested);
        match current {
            Phase::Deleting | Phase::Deleted => prop_assert_eq!(allowed, Err(TransitionError::Deleting)),
            Phase::Creating
            | Phase::Starting
            | Phase::Running
            | Phase::Freezing
            | Phase::Frozen
            | Phase::Stopping
            | Phase::Stopped => {
                if requested == DesiredState::Frozen {
                    prop_assert_eq!(allowed.is_ok(), matches!(current, Phase::Running | Phase::Freezing | Phase::Frozen));
                } else {
                    prop_assert_eq!(allowed, Ok(()));
                }
            }
        }
    }

    #[test]
    fn idle_steps_only_put_unused_workspaces_to_sleep(
        freeze_after in prop::option::of(0..10_000u32),
        stop_after in prop::option::of(0..10_000u32),
        now in any::<i64>(),
        last_used in any::<i64>(),
        current in prop::sample::select(&PHASES[..]),
        threads in prop::collection::vec(prop::sample::select(&[AttentionState::Working, AttentionState::Waiting, AttentionState::Done, AttentionState::Idle][..]), 0..4),
    ) {
        let policy = IdlePolicy { freeze_after: freeze_after.map(Millis::from_secs), stop_after: stop_after.map(Millis::from_secs) };
        let (now, last_used) = (Timestamp::from_unix_millis(now), Timestamp::from_unix_millis(last_used));
        let is_busy = busy(threads.iter().copied());
        prop_assert_eq!(is_busy == Busy::Yes, threads.iter().any(|s| matches!(s, AttentionState::Working | AttentionState::Waiting)));
        let unused = now.since(last_used);
        match decide(policy, now, current, last_used, is_busy) {
            Some(IdleStep::Freeze) => {
                prop_assert_eq!(current, Phase::Running);
                prop_assert_eq!(is_busy, Busy::No);
                prop_assert!(policy.freeze_after.is_some_and(|limit| unused >= limit));
            }
            Some(IdleStep::Stop) => {
                prop_assert_eq!(current, Phase::Frozen);
                prop_assert!(policy.stop_after.is_some_and(|limit| unused >= limit));
            }
            None => {}
        }
        // Waiting longer never takes a step back.
        if decide(policy, now, current, last_used, is_busy).is_some() {
            let later = now.plus(Millis::from_secs(60));
            prop_assert_eq!(decide(policy, later, current, last_used, is_busy), decide(policy, now, current, last_used, is_busy));
        }
    }

    #[test]
    fn opening_thaws_only_a_frozen_workspace(state in prop::sample::select(&STATES[..])) {
        prop_assert_eq!(thaw_on_open(state), (state == DesiredState::Frozen).then_some(DesiredState::Running));
    }

    #[test]
    fn whatever_needs_its_person_comes_first(
        a in facts(), b in facts(),
    ) {
        let (sa, sb) = (standing(a), standing(b));
        prop_assert_eq!(sa.needs_you == NeedsYou::Yes, sa.need.is_some());
        if sa.needs_you == NeedsYou::Yes && sb.needs_you == NeedsYou::No {
            prop_assert!(sa > sb);
        }
        if a.health == Health::Trouble {
            prop_assert_eq!(sa.needs_you, NeedsYou::Yes);
        }
        // An agent in a workspace that isn't awake can't be waiting on anyone.
        if a.health == Health::Fine && !matches!(a.phase, Phase::Creating | Phase::Starting | Phase::Running | Phase::Freezing) {
            prop_assert_eq!(sa.needs_you, NeedsYou::No);
        }
    }
}

/// Every workspace there can be, by its desired state and what's observed.
fn every_case() -> Vec<(DesiredState, Instance)> {
    let mut runtimes = vec![
        Runtime::Stopped,
        Runtime::Frozen,
        Runtime::Transitioning,
        Runtime::Failed,
    ];
    for readiness in [Readiness::Booting, Readiness::System, Readiness::Network] {
        for secrets in [None, Some(SecretsGeneration::from_u64(1))] {
            for columns in [Columns::Pending, Columns::Opened] {
                runtimes.push(Runtime::Running(Running {
                    readiness,
                    secrets,
                    columns,
                }));
            }
        }
    }
    let mut instances = vec![Instance::Absent];
    for runtime in runtimes {
        for provisioning in [Provisioning::Pending, Provisioning::Complete] {
            instances.push(Instance::Present(Present {
                runtime,
                provisioning,
            }));
        }
    }
    STATES
        .iter()
        .flat_map(|&state| instances.iter().map(move |&instance| (state, instance)))
        .collect()
}

/// Running means the person can use it: provisioned, booted at least to
/// its system, holding its secrets, with its columns open.
#[test]
fn a_workspace_is_running_exactly_when_it_is_up_with_its_secrets_and_columns() {
    for (state, instance) in every_case() {
        let up = state == DesiredState::Running
            && matches!(
                instance,
                Instance::Present(Present {
                    runtime: Runtime::Running(Running {
                        readiness: Readiness::System | Readiness::Network,
                        secrets: Some(_),
                        columns: Columns::Opened
                    }),
                    provisioning: Provisioning::Complete,
                })
            );
        assert_eq!(
            phase(state, instance) == Phase::Running,
            up,
            "{state:?} {instance:?}"
        );
    }
}

fn facts() -> impl Strategy<Value = Facts> {
    (
        prop::sample::select(&PHASES[..]),
        prop_oneof![Just(Health::Fine), Just(Health::Trouble)],
        prop::option::of((
            prop::sample::select(
                &[
                    AttentionState::Working,
                    AttentionState::Waiting,
                    AttentionState::Done,
                    AttentionState::Idle,
                ][..],
            ),
            prop_oneof![Just(Seen::Seen), Just(Seen::Unseen)],
            any::<i64>(),
        )),
        any::<i64>(),
    )
        .prop_map(|(phase, health, top, created_at)| Facts {
            phase,
            health,
            top: top.map(|(state, seen, at)| (state, seen, Timestamp::from_unix_millis(at))),
            created_at: Timestamp::from_unix_millis(created_at),
        })
}
