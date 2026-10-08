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

use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use hyper::Method;
use iglu_domain::capacity;
use iglu_domain::env::{BuiltImage, EnvSource};
use iglu_domain::guest as guest_tools;
use iglu_domain::id::InstanceName;
use iglu_domain::port::GuestPort;
use iglu_domain::secret::FetchTokens;
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_proto::CreateSpec;
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use self::client::{Incus, IncusError, StateAction};
use self::exec::Program;
use self::guestfs::paths;
use self::observe::keys;
use crate::config::{self, Timeouts};
use crate::guest::{self, GuestCommand};
use crate::runtime::{
    BuildFailure, Claim, Guest, GuestFile, Isolating, Observed, Output, Runtime, RuntimeError,
    State, Terminal, TerminalInput,
};

pub use self::client::IncusError as Error;

pub struct IncusRuntime {
    incus: Incus,
    settings: config::Incus,
    build: config::Build,
    /// Where proxy devices put their host-side sockets.
    sockets: PathBuf,
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
        Ok(Self {
            incus,
            settings,
            build,
            sockets,
            timeouts,
        })
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
        match self
            .incus
            .get::<Value>(&format!("/1.0/images/{}", spec.image))
            .await
        {
            Ok(image) => guest_tools::compatible(build::recorded_interface(&image))?,
            Err(error) if error.is_not_found() => return Err(RuntimeError::ImageMissing),
            Err(error) => return Err(failed(error)),
        }
        let guest_user = serde_json::to_string(&spec.user).map_err(RuntimeError::failed)?;
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
        };
        guestfs::read(guest.boot.get(), path)
            .await
            .map_err(RuntimeError::failed)
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
