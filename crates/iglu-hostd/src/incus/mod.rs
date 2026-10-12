//! The production runtime: workspaces are Incus system containers running
//! NixOS images built by the host's Nix daemon.
//!
//! Each instance runs as an isolated idmap with its own network on a
//! bridge whose egress policy hostd keeps in place, and hostd reads guest
//! files without following the guest's symlinks out of its root. That's
//! what makes it [`Isolating`]; the VM test checks it.

mod build;
#[cfg(feature = "conformance")]
pub mod checks;
mod client;
mod exec;
mod guestfs;
mod observe;
mod policy;
mod reclaim;
mod tunnel;
mod upgrade;

use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use hyper::Method;
use iglu_domain::capacity;
use iglu_domain::download::DownloadName;
use iglu_domain::env::{BuiltImage, EnvSource, GuestUser, ImageFingerprint};
use iglu_domain::guest as guest_tools;
use iglu_domain::id::{InstanceName, PrincipalId};
use iglu_domain::port::GuestPort;
use iglu_domain::secret::FetchTokens;
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_proto::{CreateSpec, UpgradeSpec};
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use self::client::{Incus, IncusError, StateAction};
use self::exec::Program;
use self::guestfs::paths;
use self::observe::keys;

/// The proxy device that is an instance's channel to iglu.
const CHANNEL_DEVICE: &str = "iglu-channel";

/// The disk device that is the owner's kept directory.
const KEPT_DEVICE: &str = "iglu-kept";

/// The volume that holds a person's kept directory on this host. Its files
/// belong to the workspace account's uid, so an environment whose account
/// has another gets a volume of its own.
fn kept_volume(owner: PrincipalId, user: &GuestUser) -> String {
    format!("iglu-kept-{}-{}", owner.as_uuid().simple(), user.uid.get())
}
use crate::config::{self, Timeouts};
use crate::guest::{self, GuestCommand};
use crate::runtime::{
    BuildFailure, Claim, Guest, GuestFile, Isolating, Observed, Output, Runtime, RuntimeError,
    State, StoredImage, Terminal, TerminalInput,
};

pub use self::client::IncusError as Error;

pub struct IncusRuntime {
    incus: Incus,
    settings: config::Incus,
    build: config::Build,
    /// Where proxy devices put their host-side sockets.
    sockets: PathBuf,
    /// Where hostd listens for each instance's channel.
    channels: PathBuf,
    timeouts: Timeouts,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "an adapter for map_err, which passes errors by value"
)]
fn failed(error: IncusError) -> RuntimeError {
    if error.is_not_found() {
        RuntimeError::NotFound
    } else {
        RuntimeError::Failed(error.to_string())
    }
}

fn mib(bytes: capacity::Bytes) -> String {
    format!("{}MiB", bytes.get() / (1024 * 1024))
}

impl IncusRuntime {
    /// Connects to Incus and puts the workspace bridge's egress policy in
    /// place.
    ///
    /// # Errors
    ///
    /// When Incus is unreachable or refuses the policy, or the socket
    /// directory can't be made.
    pub async fn new(
        settings: config::Incus,
        build: config::Build,
        runtime_dir: &Path,
        timeouts: Timeouts,
    ) -> Result<Self, IncusError> {
        let incus = Incus::new(settings.socket.clone(), settings.project.clone());
        policy::ensure(
            &incus,
            &settings.network,
            &settings.acl,
            timeouts.operation(),
        )
        .await?;
        let sockets = runtime_dir.join("proxy");
        tokio::fs::create_dir_all(&sockets)
            .await
            .map_err(IncusError::Connect)?;
        let channels = runtime_dir.join("channels");
        Ok(Self {
            incus,
            settings,
            build,
            sockets,
            channels,
            timeouts,
        })
    }

    fn volumes(&self) -> String {
        format!("/1.0/storage-pools/{}/volumes/custom", self.settings.pool)
    }

    /// Makes the person's kept volume unless it's there. It's shifted, so
    /// every one of their instances, each with an idmap of its own, sees its
    /// files as the workspace account's. hostd never removes one.
    async fn keep(&self, volume: &str, user: &GuestUser) -> Result<(), RuntimeError> {
        let volumes = self.volumes();
        let path = format!("{volumes}/{volume}");
        match self.incus.get::<Value>(&path).await {
            Ok(_) => return Ok(()),
            Err(error) if error.is_not_found() => {}
            Err(error) => return Err(failed(error)),
        }
        let body = json!({
            "name": volume,
            "content_type": "filesystem",
            "config": {
                "security.shifted": "true",
                "initial.uid": user.uid.get().to_string(),
                "initial.gid": user.gid.get().to_string(),
                "initial.mode": "0700",
            },
        });
        match self
            .incus
            .run(
                Method::POST,
                &volumes,
                Some(&body),
                self.timeouts.operation(),
            )
            .await
        {
            Ok(_) => Ok(()),
            // Another of the person's workspaces made it first.
            Err(_) if self.incus.get::<Value>(&path).await.is_ok() => Ok(()),
            Err(error) => Err(failed(error)),
        }
    }

    async fn change(&self, name: InstanceName, action: StateAction) -> Result<(), RuntimeError> {
        self.incus
            .change_state(name, action, self.timeouts.operation())
            .await
            .map_err(failed)
    }
}

impl Isolating for IncusRuntime {}

impl Runtime for IncusRuntime {
    type Stream = UnixStream;

    async fn memory_available(&self) -> capacity::Bytes {
        let text = tokio::fs::read_to_string("/proc/meminfo")
            .await
            .unwrap_or_default();
        capacity::Bytes::new(mem_available_kib(&text).saturating_mul(1024))
    }

    async fn instances(&self) -> Result<Vec<Claim>, RuntimeError> {
        let instances = self.incus.instances().await.map_err(failed)?;
        Ok(instances.iter().map(observe::claim).collect())
    }

    async fn instance(&self, name: InstanceName) -> Result<Option<Claim>, RuntimeError> {
        let instance = self.incus.instance(name).await.map_err(failed)?;
        Ok(instance.as_ref().map(observe::claim))
    }

    async fn build(
        &self,
        source: &EnvSource,
        tokens: &FetchTokens,
    ) -> Result<BuiltImage, BuildFailure> {
        build::build(
            &self.incus,
            &self.settings,
            &self.build,
            source,
            tokens,
            self.timeouts.operation(),
        )
        .await
        .map_err(|e| BuildFailure(e.to_string()))
    }

    async fn create(&self, name: InstanceName, spec: &CreateSpec) -> Result<(), RuntimeError> {
        let image = match self
            .incus
            .get::<Value>(&format!("/1.0/images/{}", spec.image))
            .await
        {
            Ok(image) => image,
            Err(error) if error.is_not_found() => return Err(RuntimeError::ImageMissing),
            Err(error) => return Err(failed(error)),
        };
        guest_tools::compatible(build::recorded_interface(&image))?;
        let system = build::recorded_system(&image).unwrap_or_default();
        let guest_user = serde_json::to_string(&spec.user).map_err(RuntimeError::failed)?;
        let kept = kept_volume(spec.owner, &spec.user);
        self.keep(&kept, &spec.user).await?;
        // The instance's end of its channel, which only the workspace user
        // can open; the host's end is hostd's listener for this instance.
        let channel = json!({
            "type": "proxy",
            "bind": "instance",
            "listen": format!("unix:{}", guest_tools::INSTANCE_CHANNEL),
            "connect": format!("unix:{}", self.channel(name).display()),
            "uid": spec.user.uid.get().to_string(),
            "gid": spec.user.gid.get().to_string(),
            "mode": "0600",
        });
        let body = json!({
            "name": name.to_string(),
            "type": "container",
            "source": { "type": "image", "fingerprint": spec.image.to_string() },
            "profiles": [self.settings.profile],
            "start": false,
            "config": {
                "boot.autostart": "false",
                "security.nesting": "true",
                "security.idmap.isolated": "true",
                "limits.cpu": spec.limits.cpus.to_string(),
                "limits.memory": mib(spec.limits.memory),
                "limits.memory.swap": mib(spec.limits.swap),
                "limits.processes": spec.limits.processes.to_string(),
                keys::WORKSPACE: name.workspace().to_string(),
                keys::OWNER: spec.owner.to_string(),
                keys::GUEST_USER: guest_user,
                // The image was just checked to speak this host's interface.
                keys::GUEST_INTERFACE: guest_tools::INTERFACE.to_string(),
                keys::IMAGE: spec.image.to_string(),
                keys::SYSTEM: system,
            },
            "devices": {
                CHANNEL_DEVICE: channel,
                KEPT_DEVICE: {
                    "type": "disk",
                    "pool": self.settings.pool,
                    "source": kept,
                    "path": guest_tools::INSTANCE_KEPT,
                },
            },
        });
        self.incus
            .run(
                Method::POST,
                "/1.0/instances",
                Some(&body),
                self.timeouts.operation(),
            )
            .await
            .map(|_| ())
            .map_err(failed)
    }

    async fn start(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.change(name, StateAction::Start).await
    }

    async fn freeze(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.change(name, StateAction::Freeze).await?;
        reclaim::reclaim(name).await;
        Ok(())
    }

    async fn thaw(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.change(name, StateAction::Unfreeze).await
    }

    async fn stop(&self, name: InstanceName) -> Result<(), RuntimeError> {
        let frozen = matches!(
            self.instance(name).await?,
            Some(Ok(Observed {
                state: State::Frozen,
                ..
            }))
        );
        // A frozen cgroup can't run its shutdown. If it won't thaw, all
        // that's left is to force it.
        if frozen && let Err(error) = self.change(name, StateAction::Unfreeze).await {
            tracing::info!(%name, %error, "thawing to stop failed; forcing");
            return self.change(name, StateAction::ForceStop).await;
        }
        let grace = StateAction::Stop {
            timeout_secs: self.timeouts.stop_secs,
        };
        let wait = Duration::from_secs(u64::from(self.timeouts.stop_secs) + 10);
        if let Err(error) = self.incus.change_state(name, grace, wait).await {
            tracing::info!(%name, %error, "graceful stop failed; forcing");
            self.change(name, StateAction::ForceStop).await?;
        }
        Ok(())
    }

    async fn delete(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.incus
            .run(
                Method::DELETE,
                &format!("/1.0/instances/{name}"),
                None,
                self.timeouts.operation(),
            )
            .await
            .map_err(failed)?;
        tunnel::remove_sockets(&self.sockets, name).await;
        Ok(())
    }

    async fn upgrade(&self, name: InstanceName, spec: &UpgradeSpec) -> Result<(), RuntimeError> {
        let image = match self
            .incus
            .get::<Value>(&format!("/1.0/images/{}", spec.image))
            .await
        {
            Ok(image) => image,
            Err(error) if error.is_not_found() => return Err(RuntimeError::ImageMissing),
            Err(error) => return Err(failed(error)),
        };
        guest_tools::compatible(build::recorded_interface(&image))?;
        let refuse = |why: &str| {
            RuntimeError::Refused(format!(
                "{why}; build the environment again to update to it"
            ))
        };
        let system = build::recorded_system(&image)
            .ok_or_else(|| refuse("the build has no system to upgrade to"))?;
        if !Path::new(&system).exists() {
            return Err(refuse("this host no longer has the build's system"));
        }
        let instance: Value = self
            .incus
            .get(&format!("/1.0/instances/{name}"))
            .await
            .map_err(failed)?;
        let config = &instance["config"];
        let user: Option<GuestUser> = config[keys::GUEST_USER]
            .as_str()
            .and_then(|v| serde_json::from_str(v).ok());
        // Its files are the account's.
        if user.as_ref() != Some(&spec.user) {
            return Err(RuntimeError::Refused(
                "the environment's newest build has another account than this workspace; recreate it to use the build".into(),
            ));
        }
        let old = config[keys::SYSTEM].as_str().unwrap_or_default().to_owned();
        // Another image of the same system is only recorded.
        if old != system {
            let boot = Duration::from_secs(300);
            match self.instance(name).await? {
                Some(Ok(Observed {
                    state: State::Frozen,
                    ..
                })) => self.change(name, StateAction::Unfreeze).await?,
                Some(Ok(Observed {
                    state: State::Stopped,
                    ..
                })) => self.change(name, StateAction::Start).await?,
                _ => {}
            }
            upgrade::ready(&self.incus, name, boot).await?;
            let install = Duration::from_secs(1800);
            upgrade::install(&self.incus, name, &self.build, &system, install).await?;
            self.stop(name).await?;
            self.change(name, StateAction::Start).await?;
            if let Err(error) = upgrade::tidy(&self.incus, name, &system, &old, boot).await {
                tracing::info!(%name, %error, "removing the old system failed; it stays on disk");
            }
        }
        // Its tools were just checked to speak this host's interface.
        self.incus
            .update_instance(
                name,
                |instance| {
                    let config = &mut instance["config"];
                    config[keys::GUEST_INTERFACE] = json!(guest_tools::INTERFACE.to_string());
                    config[keys::IMAGE] = json!(spec.image.to_string());
                    config[keys::SYSTEM] = json!(system);
                },
                self.timeouts.operation(),
            )
            .await
            .map_err(failed)
    }

    async fn mark_provisioned(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.incus
            .update_instance(
                name,
                |instance| {
                    instance["config"][keys::PROVISIONED] = json!("true");
                },
                self.timeouts.operation(),
            )
            .await
            .map_err(failed)
    }

    async fn run(
        &self,
        guest: &Guest,
        command: &GuestCommand<'_>,
        timeout: Duration,
    ) -> Result<Output, RuntimeError> {
        let argv = std::iter::once(paths::GUEST_TOOL.to_owned()).chain(command.args());
        let program = Program::new(&guest.user, argv);
        exec::capture(
            &self.incus,
            guest.name,
            &program,
            command.input().as_deref(),
            timeout,
        )
        .await
        .map_err(failed)
    }

    async fn attach(
        &self,
        guest: &Guest,
        session: &SessionName,
        size: TerminalSize,
    ) -> Result<Terminal, RuntimeError> {
        let argv = std::iter::once(paths::GUEST_TOOL.to_owned()).chain(guest::attach_args(session));
        let program = Program::new(&guest.user, argv);
        let exec = exec::interactive(&self.incus, guest.name, &program, size)
            .await
            .map_err(failed)?;
        Ok(relay(exec))
    }

    async fn read(&self, guest: &Guest, file: GuestFile) -> Result<Option<Vec<u8>>, RuntimeError> {
        let path = match file {
            GuestFile::Ready => paths::READY.to_owned(),
            GuestFile::SecretsGeneration => {
                paths::in_runtime_dir(&guest.user, guest_tools::SECRETS_GENERATION)
            }
            GuestFile::Status => paths::in_runtime_dir(&guest.user, guest_tools::STATUS),
            GuestFile::ColumnsOpened => {
                paths::in_runtime_dir(&guest.user, guest_tools::COLUMNS_OPENED)
            }
        };
        guestfs::read(guest.boot.get(), path)
            .await
            .map_err(RuntimeError::failed)
    }

    async fn download(
        &self,
        guest: &Guest,
        name: &DownloadName,
    ) -> Result<Option<tokio::fs::File>, RuntimeError> {
        let file = guestfs::open(guest.boot.get(), name.in_home(&guest.user))
            .await
            .map_err(RuntimeError::failed)?;
        Ok(file.map(tokio::fs::File::from_std))
    }

    fn channel(&self, name: InstanceName) -> PathBuf {
        self.channels.join(format!("{name}.sock"))
    }

    async fn connect(&self, guest: &Guest, port: GuestPort) -> Result<UnixStream, RuntimeError> {
        tunnel::connect(
            &self.incus,
            &self.sockets,
            guest.name,
            port,
            self.timeouts.operation(),
        )
        .await
    }

    async fn images(&self) -> Result<Vec<StoredImage>, RuntimeError> {
        let images: Vec<Value> = self
            .incus
            .get("/1.0/images?recursion=1")
            .await
            .map_err(failed)?;
        Ok(images
            .iter()
            .filter(|image| build::recorded_interface(image).is_some())
            .filter_map(|image| {
                Some(StoredImage {
                    fingerprint: image["fingerprint"].as_str()?.parse().ok()?,
                    built: build::recorded_build(image),
                })
            })
            .collect())
    }

    async fn delete_image(&self, image: &ImageFingerprint) -> Result<(), RuntimeError> {
        match self
            .incus
            .run(
                Method::DELETE,
                &format!("/1.0/images/{image}"),
                None,
                self.timeouts.operation(),
            )
            .await
        {
            Ok(_) => {}
            Err(error) if error.is_not_found() => {}
            Err(error) => return Err(failed(error)),
        }
        build::release(image).await;
        Ok(())
    }
}

/// Pumps an interactive exec's sockets to and from a [`Terminal`]'s channels.
fn relay(exec: exec::Interactive) -> Terminal {
    let (input, mut from_client) = mpsc::channel(64);
    let (to_client, output) = mpsc::channel::<Bytes>(64);
    let (mut to_guest, mut from_guest) = exec.data.split();
    let mut control = exec.control;
    tokio::spawn(async move {
        while let Some(message) = from_client.recv().await {
            let sent = match message {
                TerminalInput::Bytes(bytes) => to_guest.send(Message::Binary(bytes)).await,
                TerminalInput::Resize(size) => {
                    control
                        .send(Message::text(exec::resize_message(size)))
                        .await
                }
            };
            if sent.is_err() {
                break;
            }
        }
        let _ = to_guest.close().await;
        let _ = control.close(None).await;
    });
    tokio::spawn(async move {
        while let Some(Ok(message)) = from_guest.next().await {
            let bytes = match message {
                Message::Binary(bytes) => bytes,
                Message::Text(text) => Bytes::copy_from_slice(text.as_bytes()),
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            };
            if to_client.send(bytes).await.is_err() {
                break;
            }
        }
    });
    Terminal { input, output }
}

/// Linux's `MemAvailable`, in KiB, from `/proc/meminfo`.
fn mem_available_kib(meminfo: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|rest| {
            rest.trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0)
}
