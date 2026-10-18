//! A simulated runtime: instances are records in memory, and every step
//! that would change one, or run something in its guest, waits until
//! whoever drives the simulation resolves it: lets it finish, fail before
//! it takes effect, or take effect and fail anyway, as an Incus operation
//! whose answer is lost does. The driver decides the order everything
//! happens in, so a seeded run happens the same way every time.
//!
//! It runs no programs. Its guests answer the guest tools' commands as
//! `iglu-guest` does, as far as hostd can tell, and the conformance suite
//! holds it to every check that needs no program in a guest (see
//! [`crate::conformance::Guests`]).
//!
//! It records what a real runtime would refuse: a step its instance's
//! state doesn't allow when hostd asked for it, which only a mistake in
//! hostd's rules can cause.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use iglu_domain::capacity;
use iglu_domain::download::DownloadName;
use iglu_domain::env::{Arch, BuiltImage, EnvSource, GuestUser, ImageFingerprint};
use iglu_domain::id::{InstanceName, PrincipalId};
use iglu_domain::lifecycle::{Provisioning, SecretsGeneration};
use iglu_domain::port::GuestPort;
use iglu_domain::secret::{FetchTokens, SecretBundle};
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_domain::time::Timestamp;
use iglu_proto::{CreateSpec, TerminalInfo, UpgradeSpec};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

use crate::guest::{GuestCommand, Opening};
use crate::runtime::{
    BootId, BuildFailure, Claim, Guest, GuestFile, Observed, Output, Runtime, RuntimeError, State,
    StoredImage, Terminal,
};

/// How a pending step ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// It takes effect and says so.
    None,
    /// It fails without taking effect.
    Before,
    /// It takes effect, but its caller hears that it failed.
    After,
}

/// Who a pending step is for, judged by what it is: the reconciler's
/// effects, a person's requests through the API, or a build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    Effect,
    Request,
    Build,
}

/// A step waiting to be resolved, as the driver sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// Registration order. Concurrent callers register in no fixed order,
    /// so a driver that wants a reproducible run orders by the rest.
    pub seq: u64,
    pub origin: Origin,
    pub instance: Option<InstanceName>,
    pub source: Option<EnvSource>,
    /// What the step is, such as `start` or `open-column`.
    pub what: &'static str,
    /// Its caller is gone, as when the hostd that asked was restarted: it
    /// still takes effect when resolved, as an Incus operation would.
    pub orphaned: bool,
}

/// One instance as the simulation holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    pub owner: PrincipalId,
    pub user: GuestUser,
    pub image: ImageFingerprint,
    pub provisioning: Provisioning,
    pub power: Power,
    pub guest: GuestState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power {
    Stopped,
    Running(BootId),
    Frozen(BootId),
    Failed,
}

/// What one boot of a guest holds. A new boot starts empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuestState {
    pub ready: bool,
    pub secrets: Option<SecretsGeneration>,
    pub columns_opened: bool,
    pub sessions: BTreeSet<SessionName>,
}

/// Secrets a guest was given, as a driver checks them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub owner: PrincipalId,
    pub generation: SecretsGeneration,
    /// What the guest held before, this boot.
    pub replaced: Option<SecretsGeneration>,
    pub values: Vec<String>,
}

pub struct SimConfig {
    /// Whether steps wait for the driver. Without, each takes effect at
    /// once, as the conformance suite needs.
    pub gated: bool,
    /// The repositories guests can clone; there's no other network.
    pub repos: BTreeSet<String>,
    /// Where instances' channels go.
    pub channels: PathBuf,
    /// How long hostd waits for a lifecycle step before giving up on it,
    /// as it waits for an Incus operation. The step carries on.
    pub operation: Duration,
}

/// What a step is, for whom, and how long its caller waits for it.
struct About {
    origin: Origin,
    instance: Option<InstanceName>,
    source: Option<EnvSource>,
    what: &'static str,
    timeout: (Duration, OnTimeout),
}

/// What happens to a step whose caller stopped waiting.
#[derive(Clone, Copy)]
enum OnTimeout {
    /// It ends, as a guest command a runtime kills does.
    Cancel,
    /// It carries on, as an Incus operation does.
    Continue,
}

type Resolve = Box<dyn FnOnce(&mut World, Fault, bool) + Send>;

tokio::task_local! {
    /// Whether the hostd request this task serves has lost its caller.
    static ABANDONED: std::cell::Cell<bool>;
}

/// Serves one hostd request, so that once a step loses its caller, the
/// steps the request goes on to take are known to have no caller either:
/// hostd carries on with a command whoever asked for it has given up on.
pub async fn serving<F: Future>(requests: F) -> F::Output {
    ABANDONED.scope(std::cell::Cell::new(false), requests).await
}

fn abandoned() -> bool {
    ABANDONED.try_with(std::cell::Cell::get).unwrap_or(false)
}

fn abandon() {
    let _ = ABANDONED.try_with(|abandoned| abandoned.set(true));
}

struct Waiting {
    view: Pending,
    resolve: Resolve,
}

struct World {
    config: SimConfig,
    instances: Ordered<InstanceName, Instance>,
    images: Ordered<ImageFingerprint, (BuiltImage, Timestamp)>,
    builds: BTreeMap<String, u64>,
    boots: u64,
    seq: u64,
    pending: Vec<Waiting>,
    violations: Vec<String>,
    refused_while_changing: usize,
    deliveries: Vec<Delivery>,
    channels: Ordered<InstanceName, usize>,
}

#[derive(Clone)]
pub struct SimRuntime {
    world: Arc<Mutex<World>>,
}

impl SimRuntime {
    #[must_use]
    pub fn new(config: SimConfig) -> Self {
        Self {
            world: Arc::new(Mutex::new(World {
                config,
                instances: Ordered::default(),
                images: Ordered::default(),
                builds: BTreeMap::new(),
                boots: 0,
                seq: 0,
                pending: Vec::new(),
                violations: Vec::new(),
                refused_while_changing: 0,
                deliveries: Vec::new(),
                channels: Ordered::default(),
            })),
        }
    }

    fn world(&self) -> MutexGuard<'_, World> {
        self.world
            .lock()
            .expect("the world's lock is never held across a panic")
    }

    /// The steps waiting to be resolved.
    #[must_use]
    pub fn pending(&self) -> Vec<Pending> {
        self.world()
            .pending
            .iter()
            .map(|w| w.view.clone())
            .collect()
    }

    /// Resolves the pending step registered as `seq`. False when there's
    /// no such step.
    #[expect(
        clippy::must_use_candidate,
        reason = "resolving is the point; whether it was there is incidental"
    )]
    pub fn resolve(&self, seq: u64, fault: Fault) -> bool {
        let mut world = self.world();
        let Some(index) = world.pending.iter().position(|w| w.view.seq == seq) else {
            return false;
        };
        let waiting = world.pending.remove(index);
        (waiting.resolve)(&mut world, fault, waiting.view.orphaned);
        true
    }

    /// Marks every pending step from `origin` as having lost its caller.
    pub fn orphan(&self, origin: Origin) {
        for waiting in &mut self.world().pending {
            if waiting.view.origin == origin {
                waiting.view.orphaned = true;
            }
        }
    }

    /// Marks one pending step as having lost its caller.
    pub fn orphan_step(&self, seq: u64) {
        for waiting in &mut self.world().pending {
            if waiting.view.seq == seq {
                waiting.view.orphaned = true;
            }
        }
    }

    /// The host restarts: every instance that was up is stopped, with its
    /// guest's boot gone, and steps under way fail.
    pub fn reboot(&self) {
        let mut world = self.world();
        let pending = std::mem::take(&mut world.pending);
        for waiting in pending {
            (waiting.resolve)(&mut world, Fault::Before, waiting.view.orphaned);
        }
        for (_, instance) in &mut world.instances.0 {
            if matches!(instance.power, Power::Running(_) | Power::Frozen(_)) {
                instance.power = Power::Stopped;
                instance.guest = GuestState::default();
            }
        }
    }

    /// An instance's processes end on their own: it shows as stopped, or as
    /// broken when `failed`.
    pub fn crash(&self, name: InstanceName, failed: bool) {
        if let Some(instance) = self.world().instances.get_mut(&name)
            && matches!(instance.power, Power::Running(_) | Power::Frozen(_))
        {
            instance.power = if failed {
                Power::Failed
            } else {
                Power::Stopped
            };
            instance.guest = GuestState::default();
        }
    }

    /// Every instance, as the simulation holds it.
    #[must_use]
    pub fn instances_held(&self) -> Vec<(InstanceName, Instance)> {
        self.world().instances.0.clone()
    }

    /// Steps hostd asked for that the instance's state didn't allow.
    #[must_use]
    pub fn violations(&self) -> Vec<String> {
        self.world().violations.clone()
    }

    /// Steps refused because hostd acted on an instance it saw changing
    /// state, whose state underneath didn't allow them.
    #[must_use]
    pub fn refused_while_changing(&self) -> usize {
        self.world().refused_while_changing
    }

    /// Secrets delivered since the last call.
    #[must_use]
    pub fn take_deliveries(&self) -> Vec<Delivery> {
        std::mem::take(&mut self.world().deliveries)
    }

    /// Registers a step and waits for it to be resolved, or, ungated, does
    /// it at once. `apply` makes its effect and says how it went.
    fn step<T, A>(
        &self,
        about: About,
        failed: T,
        apply: A,
    ) -> impl Future<Output = Result<T, RuntimeError>> + Send + use<T, A>
    where
        T: Send + 'static,
        A: FnOnce(&mut World) -> T + Send + 'static,
    {
        let (tell, told) = oneshot::channel();
        let resolve: Resolve = Box::new(move |world, fault, orphaned| {
            let result = match fault {
                Fault::None => apply(world),
                Fault::Before => failed,
                Fault::After => {
                    let _ = apply(world);
                    failed
                }
            };
            let _ = tell.send((result, orphaned));
        });
        let About {
            origin,
            instance,
            source,
            what,
            timeout: (limit, on_timeout),
        } = about;
        let mut world = self.world();
        let seq = world.seq + 1;
        if world.config.gated {
            world.seq = seq;
            let view = Pending {
                seq,
                origin,
                instance,
                source,
                what,
                orphaned: abandoned(),
            };
            world.pending.push(Waiting { view, resolve });
        } else {
            resolve(&mut world, Fault::None, false);
        }
        drop(world);
        let world = self.world.clone();
        async move {
            if let Ok(told) = tokio::time::timeout(limit, told).await {
                let (result, orphaned) =
                    told.map_err(|_| RuntimeError::Failed("the simulation ended".into()))?;
                if orphaned {
                    abandon();
                }
                return Ok(result);
            }
            abandon();
            let mut world = world
                .lock()
                .expect("the world's lock is never held across a panic");
            match on_timeout {
                OnTimeout::Cancel => world.pending.retain(|w| w.view.seq != seq),
                OnTimeout::Continue => {
                    for waiting in &mut world.pending {
                        if waiting.view.seq == seq {
                            waiting.view.orphaned = true;
                        }
                    }
                }
            }
            Err(RuntimeError::Failed(format!(
                "{what} timed out after {}s",
                limit.as_secs()
            )))
        }
    }

    fn operation(&self) -> (Duration, OnTimeout) {
        (self.world().config.operation, OnTimeout::Continue)
    }

    /// A lifecycle step on an existing instance, whose state must be one
    /// `allowed` says. A step hostd shouldn't have asked for is refused,
    /// as Incus would, and recorded.
    async fn change(
        &self,
        name: InstanceName,
        what: &'static str,
        allowed: fn(Power) -> bool,
        apply: impl FnOnce(&mut World, InstanceName) -> Result<(), RuntimeError> + Send + 'static,
    ) -> Result<(), RuntimeError> {
        {
            let mut world = self.world();
            let power = world.instances.get(&name).map(|i| i.power);
            match power {
                None => return Err(RuntimeError::NotFound),
                Some(power) if !allowed(power) => {
                    // hostd sees an instance with a step under way as
                    // changing state, and its rules may act on it then. Incus
                    // refuses what the state underneath doesn't allow.
                    if world.changing(name) {
                        world.refused_while_changing += 1;
                    } else {
                        world
                            .violations
                            .push(format!("{what} asked of {name} while {power:?}"));
                    }
                    return Err(RuntimeError::Refused(format!(
                        "can't {what} an instance that's {power:?}"
                    )));
                }
                Some(_) => {}
            }
        }
        let failed = Err(RuntimeError::Failed(format!("simulated: {what} failed")));
        let timeout = self.operation();
        let about = About {
            origin: Origin::Effect,
            instance: Some(name),
            source: None,
            what,
            timeout,
        };
        self.step(about, failed, move |world| {
            // The state may have moved on while the step waited, as the host
            // restarting would move it; Incus refuses then too.
            match world.instances.get(&name).map(|i| i.power) {
                None => Err(RuntimeError::NotFound),
                Some(power) if !allowed(power) => Err(RuntimeError::Refused(format!(
                    "can't {what} an instance that's {power:?}"
                ))),
                Some(_) => apply(world, name),
            }
        })
        .await?
    }
}

impl World {
    fn boot(&mut self, name: InstanceName) {
        self.boots += 1;
        let boot = BootId::new(self.boots);
        if let Some(instance) = self.instances.get_mut(&name) {
            instance.power = Power::Running(boot);
            // Guests come up at once: nothing in a simulation boots.
            instance.guest = GuestState {
                ready: true,
                ..GuestState::default()
            };
        }
    }

    /// Whether a lifecycle step is waiting on the instance, which Incus
    /// shows as the instance changing state.
    fn changing(&self, name: InstanceName) -> bool {
        self.pending.iter().any(|w| {
            w.view.instance == Some(name)
                && w.view.origin == Origin::Effect
                && LIFECYCLE.contains(&w.view.what)
        })
    }

    fn observed(&self, name: InstanceName, instance: &Instance) -> Observed {
        let state = if self.changing(name) {
            State::Transitioning
        } else {
            match instance.power {
                Power::Stopped => State::Stopped,
                Power::Running(boot) => State::Running {
                    boot,
                    has_address: true,
                },
                Power::Frozen(_) => State::Frozen,
                Power::Failed => State::Failed,
            }
        };
        Observed {
            name,
            owner: instance.owner,
            user: instance.user.clone(),
            provisioning: instance.provisioning,
            state,
            image: Some(instance.image.clone()),
            memory: None,
        }
    }

    /// The running guest a command was meant for, if it's still that boot.
    fn guest_mut(&mut self, guest: &Guest) -> Option<&mut Instance> {
        self.instances
            .get_mut(&guest.name)
            .filter(|i| i.power == Power::Running(guest.boot))
    }
}

/// A map that keeps insertion order, which the driver's choices make the
/// same every run, for keys with no order of their own.
struct Ordered<K, V>(Vec<(K, V)>);

impl<K, V> Default for Ordered<K, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<K: PartialEq, V> Ordered<K, V> {
    fn get(&self, key: &K) -> Option<&V> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    fn insert(&mut self, key: K, value: V) {
        match self.get_mut(&key) {
            Some(existing) => *existing = value,
            None => self.0.push((key, value)),
        }
    }

    fn remove(&mut self, key: &K) {
        self.0.retain(|(k, _)| k != key);
    }
}

const LIFECYCLE: [&str; 6] = ["start", "freeze", "thaw", "stop", "delete", "upgrade"];

fn ok(stdout: impl Into<String>) -> Output {
    Output {
        success: true,
        stdout: stdout.into(),
        stderr: String::new(),
    }
}

fn refused(stderr: &str) -> Output {
    Output {
        success: false,
        stdout: String::new(),
        stderr: format!("iglu-guest: {stderr}"),
    }
}

/// What `iglu-guest` does for a command, as far as hostd can tell.
fn answer(world: &mut World, guest: &Guest, command: Command) -> Output {
    let repos = world.config.repos.clone();
    let Some(instance) = world.guest_mut(guest) else {
        return refused("the workspace isn't running");
    };
    let owner = instance.owner;
    match command {
        Command::Install(bundle) => {
            let replaced = instance.guest.secrets;
            instance.guest.secrets = Some(bundle.generation);
            let values = bundle
                .env
                .values()
                .chain(bundle.files.values())
                .map(|v| v.expose().to_owned())
                .chain(bundle.git.iter().map(|g| g.password.expose().to_owned()))
                .collect();
            world.deliveries.push(Delivery {
                owner,
                generation: bundle.generation,
                replaced,
                values,
            });
            ok("")
        }
        Command::Provision(repo) => match repo {
            Some(repo) if !repos.contains(&repo) => refused(&format!(
                "git clone failed: could not resolve the host of {repo}"
            )),
            Some(_) | None => ok(""),
        },
        Command::Open(sessions, opening) => {
            instance.guest.sessions.extend(sessions);
            if opening == Opening::Boot {
                instance.guest.columns_opened = true;
            }
            ok("")
        }
        Command::Sessions => {
            let listed: Vec<TerminalInfo> = instance
                .guest
                .sessions
                .iter()
                .map(|name| TerminalInfo {
                    name: name.clone(),
                    clients: 0,
                })
                .collect();
            ok(serde_json::to_string(&listed).expect("a session list always serializes"))
        }
        Command::Close(session) => {
            if instance.guest.sessions.remove(&session) {
                ok("")
            } else {
                refused(&format!("no session called {session}"))
            }
        }
        Command::Output(session) => {
            if instance.guest.sessions.contains(&session) {
                ok(r#"{"text":"","truncated":false}"#)
            } else {
                refused(&format!("no session called {session}"))
            }
        }
        Command::Input(session) => {
            if instance.guest.sessions.contains(&session) {
                ok("")
            } else {
                refused(&format!("no session called {session}"))
            }
        }
        Command::Listeners => ok("[]"),
        Command::GitState => ok("null"),
        Command::Unsupported(what) => refused(&format!("{what} isn't simulated")),
    }
}

/// A guest command with what it carries, owned, so it can wait.
enum Command {
    Install(SecretBundle),
    Provision(Option<String>),
    Open(Vec<SessionName>, Opening),
    Sessions,
    Close(SessionName),
    Output(SessionName),
    Input(SessionName),
    Listeners,
    GitState,
    Unsupported(&'static str),
}

impl Command {
    fn of(command: &GuestCommand<'_>) -> (Self, &'static str, Origin) {
        match command {
            GuestCommand::InstallSecrets(bundle) => (
                Self::Install((*bundle).clone()),
                "install-secrets",
                Origin::Effect,
            ),
            GuestCommand::Provision(spec) => (
                Self::Provision(spec.checkout.as_ref().map(|c| c.repo.to_string())),
                "provision",
                Origin::Effect,
            ),
            GuestCommand::Open(sessions, Opening::Boot) => (
                Self::Open(
                    sessions.iter().map(|s| s.name.clone()).collect(),
                    Opening::Boot,
                ),
                "open-columns",
                Origin::Effect,
            ),
            GuestCommand::Open(sessions, Opening::Column) => (
                Self::Open(
                    sessions.iter().map(|s| s.name.clone()).collect(),
                    Opening::Column,
                ),
                "open-column",
                Origin::Request,
            ),
            GuestCommand::Sessions => (Self::Sessions, "sessions", Origin::Request),
            GuestCommand::Listeners => (Self::Listeners, "listeners", Origin::Request),
            GuestCommand::GitState => (Self::GitState, "git-state", Origin::Request),
            GuestCommand::Close(session) => {
                (Self::Close((*session).clone()), "close", Origin::Request)
            }
            GuestCommand::Output(session, _) => {
                (Self::Output((*session).clone()), "output", Origin::Request)
            }
            GuestCommand::Input(session, _) => {
                (Self::Input((*session).clone()), "input", Origin::Request)
            }
            GuestCommand::Keep(..) => (Self::Unsupported("keeping files"), "keep", Origin::Request),
            GuestCommand::Display(_) => {
                (Self::Unsupported("the browser"), "display", Origin::Request)
            }
        }
    }
}

/// The account every simulated image runs its workspaces as.
fn guest_user() -> GuestUser {
    serde_json::from_value(serde_json::json!({
        "name": "dev", "uid": 1000, "gid": 100, "home": "/home/dev"
    }))
    .expect("a valid guest user")
}

impl Runtime for SimRuntime {
    type Stream = tokio::io::DuplexStream;

    fn memory_available(&self) -> impl Future<Output = capacity::Bytes> + Send {
        std::future::ready(capacity::Bytes::gib(1024))
    }

    fn instances(&self) -> impl Future<Output = Result<Vec<Claim>, RuntimeError>> + Send {
        let world = self.world();
        std::future::ready(Ok(world
            .instances
            .0
            .iter()
            .map(|(name, instance)| Ok(world.observed(*name, instance)))
            .collect()))
    }

    fn instance(
        &self,
        name: InstanceName,
    ) -> impl Future<Output = Result<Option<Claim>, RuntimeError>> + Send {
        let world = self.world();
        std::future::ready(Ok(world
            .instances
            .get(&name)
            .map(|instance| Ok(world.observed(name, instance)))))
    }

    /// Each build of a source makes a new image, as one whose inputs moved
    /// would.
    async fn build(
        &self,
        source: &EnvSource,
        _tokens: &FetchTokens,
    ) -> Result<BuiltImage, BuildFailure> {
        let key = source.to_string();
        let failed = Err(BuildFailure("simulated: the build failed".into()));
        let timeout = (Duration::from_secs(3600), OnTimeout::Continue);
        let about = About {
            origin: Origin::Build,
            instance: None,
            source: Some(source.clone()),
            what: "build",
            timeout,
        };
        self.step(about, failed, move |world| {
            let count = world.builds.entry(key.clone()).or_insert(0);
            *count += 1;
            let digest = Sha256::digest(format!("{key}#{count}").as_bytes());
            let fingerprint: ImageFingerprint = hex::encode(digest)
                .parse()
                .expect("a SHA-256 in hex is a fingerprint");
            let image = BuiltImage {
                fingerprint: fingerprint.clone(),
                arch: crate::host_arch().unwrap_or(Arch::X86_64),
                user: guest_user(),
                browser: false,
                store_path: format!("/nix/store/{}-sim", &fingerprint.as_str()[..32]),
            };
            world.images.insert(fingerprint, (image.clone(), now()));
            Ok(image)
        })
        .await
        .map_err(|e| BuildFailure(e.to_string()))?
    }

    async fn create(&self, name: InstanceName, spec: &CreateSpec) -> Result<(), RuntimeError> {
        {
            let mut world = self.world();
            if world.instances.contains_key(&name) {
                world
                    .violations
                    .push(format!("create asked of {name}, which exists"));
                return Err(RuntimeError::Refused(format!("{name} exists")));
            }
        }
        let spec = spec.clone();
        let failed = Err(RuntimeError::Failed("simulated: create failed".into()));
        let timeout = self.operation();
        let about = About {
            origin: Origin::Effect,
            instance: Some(name),
            source: None,
            what: "create",
            timeout,
        };
        self.step(about, failed, move |world| {
            if !world.images.contains_key(&spec.image) {
                return Err(RuntimeError::ImageMissing);
            }
            if world.instances.contains_key(&name) {
                return Err(RuntimeError::Refused(format!("{name} exists")));
            }
            let index = world.channels.0.len();
            world.channels.insert(name, index);
            world.instances.insert(
                name,
                Instance {
                    owner: spec.owner,
                    user: spec.user,
                    image: spec.image,
                    provisioning: Provisioning::Pending,
                    power: Power::Stopped,
                    guest: GuestState::default(),
                },
            );
            Ok(())
        })
        .await?
    }

    async fn start(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.change(
            name,
            "start",
            |p| matches!(p, Power::Stopped | Power::Failed),
            |world, name| {
                world.boot(name);
                Ok(())
            },
        )
        .await
    }

    async fn freeze(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.change(
            name,
            "freeze",
            |p| matches!(p, Power::Running(_)),
            |world, name| {
                if let Some(instance) = world.instances.get_mut(&name)
                    && let Power::Running(boot) = instance.power
                {
                    instance.power = Power::Frozen(boot);
                }
                Ok(())
            },
        )
        .await
    }

    async fn thaw(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.change(
            name,
            "thaw",
            |p| matches!(p, Power::Frozen(_)),
            |world, name| {
                if let Some(instance) = world.instances.get_mut(&name)
                    && let Power::Frozen(boot) = instance.power
                {
                    instance.power = Power::Running(boot);
                }
                Ok(())
            },
        )
        .await
    }

    async fn stop(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.change(
            name,
            "stop",
            |p| matches!(p, Power::Running(_) | Power::Frozen(_) | Power::Failed),
            |world, name| {
                if let Some(instance) = world.instances.get_mut(&name) {
                    instance.power = Power::Stopped;
                    instance.guest = GuestState::default();
                }
                Ok(())
            },
        )
        .await
    }

    async fn delete(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.change(
            name,
            "delete",
            |p| matches!(p, Power::Stopped),
            |world, name| {
                world.instances.remove(&name);
                Ok(())
            },
        )
        .await
    }

    async fn upgrade(&self, name: InstanceName, spec: &UpgradeSpec) -> Result<(), RuntimeError> {
        let spec = spec.clone();
        self.change(
            name,
            "upgrade",
            |p| matches!(p, Power::Stopped | Power::Running(_) | Power::Frozen(_)),
            move |world, name| {
                let Some(instance) = world.instances.get(&name) else {
                    return Err(RuntimeError::NotFound);
                };
                if instance.image == spec.image {
                    return Ok(());
                }
                if instance.user != spec.user {
                    return Err(RuntimeError::Refused(
                        "the image's account isn't the instance's".into(),
                    ));
                }
                if !world.images.contains_key(&spec.image) {
                    return Err(RuntimeError::ImageMissing);
                }
                if let Some(instance) = world.instances.get_mut(&name) {
                    instance.image = spec.image;
                }
                world.boot(name);
                Ok(())
            },
        )
        .await
    }

    async fn mark_provisioned(&self, name: InstanceName) -> Result<(), RuntimeError> {
        if !self.world().instances.contains_key(&name) {
            return Err(RuntimeError::NotFound);
        }
        let failed = Err(RuntimeError::Failed("simulated: recording failed".into()));
        let timeout = self.operation();
        let about = About {
            origin: Origin::Effect,
            instance: Some(name),
            source: None,
            what: "mark-provisioned",
            timeout,
        };
        self.step(about, failed, move |world| {
            match world.instances.get_mut(&name) {
                Some(instance) => {
                    instance.provisioning = Provisioning::Complete;
                    Ok(())
                }
                None => Err(RuntimeError::NotFound),
            }
        })
        .await?
    }

    async fn run(
        &self,
        guest: &Guest,
        command: &GuestCommand<'_>,
        timeout: Duration,
    ) -> Result<Output, RuntimeError> {
        let (command, what, origin) = Command::of(command);
        let guest = guest.clone();
        let failed = Err(RuntimeError::Failed(format!("simulated: {what} failed")));
        // A runtime ends a guest command that runs too long.
        let timeout = (timeout, OnTimeout::Cancel);
        let about = About {
            origin,
            instance: Some(guest.name),
            source: None,
            what,
            timeout,
        };
        self.step(about, failed, move |world| {
            Ok(answer(world, &guest, command))
        })
        .await?
    }

    fn attach(
        &self,
        _guest: &Guest,
        _session: &SessionName,
        _size: TerminalSize,
    ) -> impl Future<Output = Result<Terminal, RuntimeError>> + Send {
        std::future::ready(Err(RuntimeError::Failed(
            "the simulation runs no terminals".into(),
        )))
    }

    fn read(
        &self,
        guest: &Guest,
        file: GuestFile,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, RuntimeError>> + Send {
        let mut world = self.world();
        let read = world.guest_mut(guest).and_then(|instance| {
            let state = &instance.guest;
            match file {
                GuestFile::Ready => state.ready.then(Vec::new),
                GuestFile::SecretsGeneration => {
                    state.secrets.map(|g| g.get().to_string().into_bytes())
                }
                GuestFile::Status => None,
                GuestFile::ColumnsOpened => state.columns_opened.then(Vec::new),
            }
        });
        std::future::ready(Ok(read))
    }

    fn download(
        &self,
        _guest: &Guest,
        _name: &DownloadName,
    ) -> impl Future<Output = Result<Option<tokio::fs::File>, RuntimeError>> + Send {
        std::future::ready(Ok(None))
    }

    fn channel(&self, name: InstanceName) -> PathBuf {
        let world = self.world();
        let index = world.channels.get(&name).copied().unwrap_or(usize::MAX);
        world.config.channels.join(format!("{index}.sock"))
    }

    fn connect(
        &self,
        _guest: &Guest,
        _port: GuestPort,
    ) -> impl Future<Output = Result<Self::Stream, RuntimeError>> + Send {
        std::future::ready(Err(RuntimeError::Failed(
            "nothing listens in a simulated guest".into(),
        )))
    }

    fn images(&self) -> impl Future<Output = Result<Vec<StoredImage>, RuntimeError>> + Send {
        std::future::ready(Ok(self
            .world()
            .images
            .0
            .iter()
            .map(|(fingerprint, (_, built))| StoredImage {
                fingerprint: fingerprint.clone(),
                built: Some(*built),
            })
            .collect()))
    }

    fn delete_image(
        &self,
        image: &ImageFingerprint,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        self.world().images.remove(image);
        std::future::ready(Ok(()))
    }
}

/// The clock, as the domain's time: builds are dated as a real runtime
/// dates them.
fn now() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        });
    Timestamp::from_unix_millis(millis)
}
