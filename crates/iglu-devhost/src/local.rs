//! The local runtime: each workspace is a directory, and its processes are
//! ordinary processes of the user running the devhost. A workspace is
//! relocated by its environment alone: `HOME` is its directory and
//! `XDG_RUNTIME_DIR` a directory that lasts one boot, which the guest tools,
//! zmx and Git all honor.
//!
//! It runs the same guest tools as an Incus workspace, with this machine's
//! own tools on the `PATH`. It isolates nothing, so it isn't
//! [`Isolating`](iglu_hostd::runtime::Isolating): workspaces share the
//! user's files, network and loopback ports.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use bytes::Bytes;
use iglu_domain::capacity;
use iglu_domain::env::{BuiltImage, EnvSource, GuestPath, GuestUser, ImageFingerprint, Uid};
use iglu_domain::guest as guest_tools;
use iglu_domain::id::InstanceName;
use iglu_domain::lifecycle::Provisioning;
use iglu_domain::port::GuestPort;
use iglu_domain::secret::FetchTokens;
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_hostd::config::Timeouts;
use iglu_hostd::guest::{self, GuestCommand};
use iglu_hostd::runtime::{
    BuildFailure, Claim, Guest, GuestFile, MAX_GUEST_FILE, Observed, Output, Ownership, Runtime,
    RuntimeError, Terminal, TerminalInput,
};
use iglu_proto::CreateSpec;
use rustix::process::Signal;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc};

use crate::config::{AbsolutePath, Settings};
use crate::processes;
use crate::store::{
    self, Boots, Image, Layout, MAX_SOCKET_PATH, MachineBoot, Power, Record, Remains, Slot,
    longest_socket_path,
};

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
    #[error("the runtime directory {0} must be a directory only you can use")]
    Unsafe(PathBuf),
    #[error(
        "the runtime directory {path} is too long: terminal sessions are Unix sockets inside it, \
         whose paths can be at most {MAX_SOCKET_PATH} bytes, and the longest would be {longest}; \
         choose a shorter runtime_dir"
    )]
    TooLong { path: PathBuf, longest: usize },
    #[error("no guest tools in {0}; build them with cargo build -p iglu-guest")]
    NoGuestTools(PathBuf),
    #[error("this isn't a usable account: {0}")]
    User(String),
    #[error("the devhost runs workspaces as you, so it mustn't run as root")]
    Root,
    #[error(
        "the devhost is running inside the local workspace {0}, so stopping that workspace would \
         stop it too; start it from outside any workspace"
    )]
    InsideWorkspace(String),
}

pub struct LocalRuntime {
    layout: Layout,
    tools: PathBuf,
    /// Whom workspaces run as: the devhost's own user.
    user: GuestUser,
    /// The agents every environment gets; see [`Settings::agents`].
    agents: Vec<iglu_domain::agent::AgentSpec>,
    memory_available: capacity::Bytes,
    timeouts: Timeouts,
    /// The account's login shell, which terminal sessions run.
    shell: PathBuf,
    /// The account's Nix profile, linked into every workspace home so login
    /// shells, which rebuild `PATH` from `HOME`, still find its tools.
    profile: Option<PathBuf>,
    /// The devhost's own `PATH` and locale, which workspaces get.
    inherited: BTreeMap<&'static str, OsString>,
    /// The machine boot the devhost is running in.
    machine: MachineBoot,
    /// Held while handing out a slot.
    slots: Mutex<()>,
}

fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> SetupError + '_ {
    move |error| SetupError::Io(path.to_owned(), error)
}

/// Makes `dir` if needed and checks no one else can use it: other users
/// mustn't plant sockets or links in a workspace's runtime directory.
async fn private_dir(dir: &Path) -> Result<(), SetupError> {
    tokio::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .await
        .map_err(io_at(dir))?;
    let metadata = tokio::fs::symlink_metadata(dir).await.map_err(io_at(dir))?;
    let others = metadata.mode() & 0o077;
    if metadata.is_dir() && metadata.uid() == rustix::process::getuid().as_raw() && others == 0 {
        Ok(())
    } else {
        Err(SetupError::Unsafe(dir.to_owned()))
    }
}

fn guest_path(path: &Path) -> Result<GuestPath, String> {
    path.to_str()
        .ok_or_else(|| format!("{} isn't UTF-8", path.display()))?
        .parse()
        .map_err(|e| format!("{e}"))
}

/// The Git configuration every workspace gets: the guest tools' credential
/// helper, and, when given, a CA that Git trusts for `https://*.localhost`
/// alone. A URL-scoped `sslCAInfo` replaces Git's trust only for those hosts.
fn system_gitconfig(tools: &Path, localhost_ca: Option<&Path>) -> String {
    let credential = format!(
        "[credential]\n\thelper = {}/iglu-guest git-credential\n",
        tools.display()
    );
    match localhost_ca {
        None => credential,
        Some(ca) => {
            // Quoted as a Git config value, so any path reads back as itself.
            let path = ca
                .display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            format!("{credential}[http \"https://*.localhost\"]\n\tsslCAInfo = \"{path}\"\n")
        }
    }
}

/// A local environment's fingerprint: the same source is the same image.
fn fingerprint(source: &EnvSource) -> ImageFingerprint {
    let digest = Sha256::digest(format!("iglu-devhost:{source}"));
    hex::encode(digest)
        .parse()
        .expect("a SHA-256 in hex is a fingerprint")
}

/// Reads a small regular file the guest wrote, as Incus's guest reads do:
/// without following a final symlink or blocking on a FIFO. Anything else
/// reads as absent.
fn read_guest_file(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    use rustix::fs::{Mode, OFlags};

    let fd = match rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NOCTTY,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT | rustix::io::Errno::LOOP) => return Ok(None),
        Err(errno) => return Err(errno.into()),
    };
    let file = std::fs::File::from(fd);
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Ok(None);
    }
    if metadata.len() > MAX_GUEST_FILE {
        return Err(std::io::Error::other(format!(
            "guest file is larger than {MAX_GUEST_FILE} bytes"
        )));
    }
    let mut content = Vec::new();
    file.take(MAX_GUEST_FILE).read_to_end(&mut content)?;
    Ok(Some(content))
}

impl LocalRuntime {
    /// # Errors
    ///
    /// When the directories can't be made, aren't private or are too long,
    /// the guest tools are missing, the user can't run workspaces, or the
    /// devhost is itself running in a workspace.
    pub async fn new(settings: &Settings, timeouts: Timeouts) -> Result<Self, SetupError> {
        if let Some(workspace) = std::env::var_os(processes::MARKER) {
            return Err(SetupError::InsideWorkspace(
                workspace.to_string_lossy().into_owned(),
            ));
        }
        let layout = Layout::new(
            settings.state_dir.as_path().to_owned(),
            settings.runtime_dir.as_path().to_owned(),
        );
        let longest = longest_socket_path(layout.runtime());
        if longest > MAX_SOCKET_PATH {
            return Err(SetupError::TooLong {
                path: layout.runtime().to_owned(),
                longest,
            });
        }
        for dir in [layout.images(), layout.instances()] {
            tokio::fs::create_dir_all(&dir).await.map_err(io_at(&dir))?;
        }
        private_dir(layout.runtime()).await?;
        let tools = settings.guest_tools.as_path().to_owned();
        if !tokio::fs::metadata(tools.join("iglu-guest"))
            .await
            .is_ok_and(|m| m.is_file())
        {
            return Err(SetupError::NoGuestTools(tools));
        }
        let gitconfig = layout.gitconfig();
        tokio::fs::write(
            &gitconfig,
            system_gitconfig(
                &tools,
                settings.localhost_ca.as_ref().map(AbsolutePath::as_path),
            ),
        )
        .await
        .map_err(io_at(&gitconfig))?;

        let account = nix::unistd::User::from_uid(nix::unistd::getuid())
            .ok()
            .flatten()
            .ok_or_else(|| SetupError::User("no account for this user".into()))?;
        let user = GuestUser {
            name: account
                .name
                .parse()
                .map_err(|_| SetupError::User(account.name.clone()))?,
            uid: Uid::try_from(account.uid.as_raw()).map_err(|_| SetupError::Root)?,
            gid: Uid::try_from(account.gid.as_raw()).map_err(|_| SetupError::Root)?,
            home: guest_path(&layout.instances()).map_err(SetupError::User)?,
        };
        let profile = account.dir.join(".nix-profile");
        let profile = tokio::fs::try_exists(&profile)
            .await
            .unwrap_or(false)
            .then_some(profile);
        let inherited = ["PATH", "LANG", "TMPDIR"]
            .into_iter()
            .filter_map(|key| std::env::var_os(key).map(|value| (key, value)))
            .collect();
        let runtime = Self {
            layout,
            tools,
            user,
            agents: settings.agents.clone(),
            shell: settings
                .shell
                .as_ref()
                .map_or(account.shell, |shell| shell.as_path().to_owned()),
            profile,
            memory_available: settings.memory_available,
            timeouts,
            inherited,
            machine: MachineBoot::current(),
            slots: Mutex::new(()),
        };
        runtime.forget_earlier_boots().await?;
        Ok(runtime)
    }

    /// Workspaces started before the machine last booted lost their
    /// processes with it; their runtime directories, which hold delivered
    /// secrets, go too, and they're recorded as stopped.
    async fn forget_earlier_boots(&self) -> Result<(), SetupError> {
        let instances = self.layout.instances();
        let mut entries = tokio::fs::read_dir(&instances)
            .await
            .map_err(io_at(&instances))?;
        while let Some(entry) = entries.next_entry().await.map_err(io_at(&instances))? {
            let Some(name) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<InstanceName>().ok())
            else {
                continue;
            };
            let (Ok(record), Ok(boots)) = (self.record(name).await, self.boots(name).await) else {
                continue;
            };
            if boots.machine == self.machine || boots.power == Power::Stopped {
                continue;
            }
            let run = self.layout.run(record.slot);
            match tokio::fs::remove_dir_all(&run).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(SetupError::Io(run, error)),
            }
            let stopped = Boots {
                power: Power::Stopped,
                ..boots
            };
            store::write_json(&self.layout.power(name), &stopped)
                .await
                .map_err(io_at(&self.layout.power(name)))?;
        }
        Ok(())
    }

    fn tool(&self, name: &str) -> PathBuf {
        self.tools.join(name)
    }

    /// Everything a workspace's processes start with. Nothing else of the
    /// devhost's environment leaks in.
    fn environment(&self, name: InstanceName, run: &Path) -> BTreeMap<OsString, OsString> {
        let mut env: BTreeMap<OsString, OsString> = self
            .inherited
            .iter()
            .map(|(key, value)| ((*key).into(), value.clone()))
            .collect();
        let mut path = OsString::from(self.tools.as_os_str());
        if let Some(inherited) = self.inherited.get("PATH") {
            path.push(":");
            path.push(inherited);
        }
        env.insert("PATH".into(), path);
        env.insert("HOME".into(), self.layout.home(name).into());
        env.insert("XDG_RUNTIME_DIR".into(), run.into());
        env.insert(
            iglu_domain::guest::CHANNEL_ENV.into(),
            self.layout.channel(name).into(),
        );
        env.insert("USER".into(), self.user.name.as_str().into());
        env.insert("LOGNAME".into(), self.user.name.as_str().into());
        env.insert("SHELL".into(), self.shell.clone().into());
        env.insert("GIT_CONFIG_SYSTEM".into(), self.layout.gitconfig().into());
        env.insert(processes::MARKER.into(), name.to_string().into());
        env
    }

    async fn record(&self, name: InstanceName) -> Result<Record, RuntimeError> {
        store::read_json(&self.layout.record(name))
            .await
            .map_err(RuntimeError::failed)
    }

    /// The workspace's runtime directory, whether or not it exists now.
    async fn run_dir(&self, name: InstanceName) -> Result<PathBuf, RuntimeError> {
        Ok(self.layout.run(self.record(name).await?.slot))
    }

    async fn boots(&self, name: InstanceName) -> Result<Boots, RuntimeError> {
        store::read_json(&self.layout.power(name))
            .await
            .map_err(RuntimeError::failed)
    }

    async fn set_power(&self, name: InstanceName, boots: Boots) -> Result<(), RuntimeError> {
        store::write_json(&self.layout.power(name), &boots)
            .await
            .map_err(RuntimeError::failed)
    }

    async fn claim(&self, name: InstanceName) -> Claim {
        let damaged = |why| Ownership::Damaged(name.to_string(), why);
        let record: Record = store::read_json(&self.layout.record(name))
            .await
            .map_err(|_| damaged("record"))?;
        if record.workspace != name.workspace() {
            return Err(Ownership::Foreign);
        }
        let boots = self.boots(name).await.map_err(|_| damaged("power"))?;
        let remains = if boots.machine != self.machine {
            // Restarting the machine ended its processes.
            Remains::Nothing
        } else if is_dir(&self.layout.run(record.slot)).await {
            Remains::RuntimeDir
        } else if boots.power == Power::Stopped {
            Remains::Nothing
        } else {
            // Rare, so the process table is only read for it. One that
            // can't be read counts as processes left over.
            match processes::of(name).await {
                Ok(pids) if pids.is_empty() => Remains::Nothing,
                Ok(_) | Err(_) => Remains::Processes,
            }
        };
        Ok(Observed {
            name,
            owner: record.owner,
            user: record.user,
            provisioning: if record.provisioned {
                Provisioning::Complete
            } else {
                Provisioning::Pending
            },
            state: store::state(boots, self.machine, remains),
            guest_interface: record.guest_interface,
            memory: None,
        })
    }

    async fn ensure_exists(&self, name: InstanceName) -> Result<(), RuntimeError> {
        if is_dir(&self.layout.instance(name)).await {
            Ok(())
        } else {
            Err(RuntimeError::NotFound)
        }
    }

    /// Hands out the next slot, once.
    async fn allocate_slot(&self) -> Result<Slot, RuntimeError> {
        let _held = self.slots.lock().await;
        let path = self.layout.slots();
        let last = match store::read_json::<Slot>(&path).await {
            Ok(slot) => slot,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Slot::NONE_YET,
            Err(error) => return Err(RuntimeError::failed(error)),
        };
        let slot = last
            .next()
            .ok_or_else(|| RuntimeError::Failed("out of runtime slots".into()))?;
        store::write_json(&path, &slot)
            .await
            .map_err(RuntimeError::failed)?;
        Ok(slot)
    }

    /// Claude Code's hooks, as the workspace module installs them, so the
    /// console sees agents' attention. Local workspaces have no system
    /// settings of their own, so they go in the user's.
    fn claude_settings(&self) -> serde_json::Value {
        let hook = serde_json::json!({
            "type": "command",
            "command": format!("{} claude-hook", self.tool("iglu-status").display()),
        });
        let mut hooks = serde_json::Map::new();
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "Notification",
            "Stop",
            "SessionEnd",
        ] {
            hooks.insert(event.into(), serde_json::json!([{ "hooks": [hook] }]));
        }
        for event in ["PreToolUse", "PostToolUse"] {
            hooks.insert(
                event.into(),
                serde_json::json!([{ "matcher": "*", "hooks": [hook] }]),
            );
        }
        serde_json::json!({ "hooks": hooks })
    }

    /// Lays the workspace out beside the others, then moves it into place,
    /// so it never exists half made.
    async fn make(&self, name: InstanceName, spec: &CreateSpec, slot: Slot) -> std::io::Result<()> {
        let staging = self.layout.instances().join(format!(".new-{name}"));
        if tokio::fs::try_exists(&staging).await? {
            tokio::fs::remove_dir_all(&staging).await?;
        }
        let home = staging.join("home");
        tokio::fs::create_dir_all(home.join(".claude")).await?;
        if let Some(profile) = &self.profile {
            tokio::fs::symlink(profile, home.join(".nix-profile")).await?;
        }
        // Login shells rebuild PATH, so the guest tools, which an image
        // installs system-wide, go back on it from the shells' startup files.
        let tools = self.tools.display();
        let fish = home.join(".config/fish/conf.d");
        tokio::fs::create_dir_all(&fish).await?;
        tokio::fs::write(
            fish.join("iglu.fish"),
            format!("set -gx PATH '{tools}' $PATH\n"),
        )
        .await?;
        for file in [".profile", ".bash_profile", ".zprofile"] {
            tokio::fs::write(
                home.join(file),
                format!("export PATH='{tools}':\"$PATH\"\n"),
            )
            .await?;
        }
        store::write_json(&home.join(".claude/settings.json"), &self.claude_settings()).await?;
        let user = GuestUser {
            home: guest_path(&self.layout.home(name)).map_err(std::io::Error::other)?,
            ..spec.user.clone()
        };
        let record = Record {
            workspace: name.workspace(),
            owner: spec.owner,
            user,
            image: spec.image.clone(),
            provisioned: false,
            slot,
            // The image was just checked to speak this host's interface.
            guest_interface: Some(guest_tools::INTERFACE),
        };
        store::write_json(&staging.join("record.json"), &record).await?;
        store::write_json(&staging.join("power.json"), &Boots::NEVER).await?;
        tokio::fs::rename(&staging, self.layout.instance(name)).await
    }

    async fn remove_runtime_dir(&self, name: InstanceName) -> Result<(), RuntimeError> {
        match tokio::fs::remove_dir_all(self.run_dir(name).await?).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(RuntimeError::failed(error)),
        }
    }

    async fn power(
        &self,
        name: InstanceName,
        power: Power,
        signal: Option<Signal>,
    ) -> Result<(), RuntimeError> {
        let boots = self.boots(name).await?;
        if let Some(signal) = signal {
            processes::signal(
                &processes::of(name).await.map_err(RuntimeError::failed)?,
                signal,
            );
        }
        self.set_power(name, Boots { power, ..boots }).await
    }

    /// Ends every process the workspace has, frozen or not.
    async fn end_processes(&self, name: InstanceName) -> Result<(), RuntimeError> {
        let grace = Duration::from_secs(self.timeouts.stop_secs.into());
        processes::end(name, grace)
            .await
            .map_err(RuntimeError::failed)
    }

    /// Runs `iglu-guest attach` on a new terminal.
    fn open_terminal(
        &self,
        guest: &Guest,
        run: &Path,
        session: &SessionName,
        size: TerminalSize,
    ) -> Result<Terminal, RuntimeError> {
        let (pty, pts) = pty_process::open().map_err(RuntimeError::failed)?;
        pty.resize(pty_process::Size::new(size.rows(), size.cols()))
            .map_err(RuntimeError::failed)?;
        let child = pty_process::Command::new(self.tool("iglu-guest"))
            .args(guest::attach_args(session))
            .env_clear()
            .envs(self.environment(guest.name, run))
            .current_dir(self.layout.home(guest.name))
            .kill_on_drop(true)
            .spawn(pts)
            .map_err(RuntimeError::failed)?;
        Ok(relay(pty, child))
    }
}

async fn is_dir(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok_and(|m| m.is_dir())
}

impl Runtime for LocalRuntime {
    type Stream = TcpStream;

    fn memory_available(&self) -> impl Future<Output = capacity::Bytes> + Send {
        std::future::ready(self.memory_available)
    }

    async fn instances(&self) -> Result<Vec<Claim>, RuntimeError> {
        let mut entries = tokio::fs::read_dir(self.layout.instances())
            .await
            .map_err(RuntimeError::failed)?;
        let mut claims = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(RuntimeError::failed)? {
            // Half-made and half-deleted workspaces have names of their own.
            if let Some(name) = entry.file_name().to_str().and_then(|n| n.parse().ok()) {
                claims.push(self.claim(name).await);
            }
        }
        Ok(claims)
    }

    async fn instance(&self, name: InstanceName) -> Result<Option<Claim>, RuntimeError> {
        if is_dir(&self.layout.instance(name)).await {
            Ok(Some(self.claim(name).await))
        } else {
            Ok(None)
        }
    }

    /// Records the environment without building it: this machine can't build
    /// a Linux image, and its workspaces use its own tools anyway.
    async fn build(
        &self,
        source: &EnvSource,
        _tokens: &FetchTokens,
    ) -> Result<BuiltImage, BuildFailure> {
        let fingerprint = fingerprint(source);
        let image = Image {
            source: source.clone(),
            guest_interface: Some(guest_tools::INTERFACE),
        };
        store::write_json(&self.layout.image(&fingerprint), &image)
            .await
            .map_err(|e| BuildFailure(format!("recording the environment: {e}")))?;
        Ok(BuiltImage {
            fingerprint,
            arch: iglu_hostd::host_arch().map_err(|e| BuildFailure(e.to_string()))?,
            user: self.user.clone(),
            agents: self.agents.clone(),
            // Nothing was built.
            store_path: String::new(),
        })
    }

    async fn create(&self, name: InstanceName, spec: &CreateSpec) -> Result<(), RuntimeError> {
        let image: Image = match store::read_json(&self.layout.image(&spec.image)).await {
            Ok(image) => image,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(RuntimeError::ImageMissing);
            }
            Err(error) => return Err(RuntimeError::failed(error)),
        };
        guest_tools::compatible(image.guest_interface)?;
        let slot = self.allocate_slot().await?;
        self.make(name, spec, slot)
            .await
            .map_err(RuntimeError::failed)
    }

    async fn start(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.ensure_exists(name).await?;
        let boots = self.boots(name).await?;
        // A boot starts clean: nothing left from one that lost its directory.
        self.end_processes(name).await?;
        self.remove_runtime_dir(name).await?;
        private_dir(&self.run_dir(name).await?)
            .await
            .map_err(RuntimeError::failed)?;
        let boots = Boots {
            power: Power::Running,
            count: boots.count.saturating_add(1),
            machine: self.machine,
        };
        self.set_power(name, boots).await
    }

    async fn freeze(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.power(name, Power::Frozen, Some(Signal::STOP)).await
    }

    async fn thaw(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.power(name, Power::Running, Some(Signal::CONT)).await
    }

    async fn stop(&self, name: InstanceName) -> Result<(), RuntimeError> {
        // Frozen processes are continued before they're asked to end.
        self.end_processes(name).await?;
        self.remove_runtime_dir(name).await?;
        self.power(name, Power::Stopped, None).await
    }

    async fn delete(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.ensure_exists(name).await?;
        let doomed = self.layout.instances().join(format!(".deleting-{name}"));
        tokio::fs::rename(self.layout.instance(name), &doomed)
            .await
            .map_err(RuntimeError::failed)?;
        tokio::fs::remove_dir_all(&doomed)
            .await
            .map_err(RuntimeError::failed)
    }

    async fn mark_provisioned(&self, name: InstanceName) -> Result<(), RuntimeError> {
        self.ensure_exists(name).await?;
        let mut record = self.record(name).await?;
        record.provisioned = true;
        store::write_json(&self.layout.record(name), &record)
            .await
            .map_err(RuntimeError::failed)
    }

    async fn run(
        &self,
        guest: &Guest,
        command: &GuestCommand<'_>,
        timeout: Duration,
    ) -> Result<Output, RuntimeError> {
        let run = self.run_dir(guest.name).await?;
        let input = command.input();
        let mut child = tokio::process::Command::new(self.tool("iglu-guest"))
            .args(command.args())
            .env_clear()
            .envs(self.environment(guest.name, &run))
            .current_dir(self.layout.home(guest.name))
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(RuntimeError::failed)?;
        let stdin = child.stdin.take();
        let feed = async move {
            if let (Some(mut stdin), Some(input)) = (stdin, input) {
                // A program that stops reading shows in its exit status.
                let _ = stdin.write_all(&input).await;
            }
        };
        let (output, ()) = tokio::time::timeout(timeout, async {
            tokio::join!(child.wait_with_output(), feed)
        })
        .await
        .map_err(|_| RuntimeError::Failed(format!("timed out after {}s", timeout.as_secs())))?;
        let output = output.map_err(RuntimeError::failed)?;
        Ok(Output {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    async fn attach(
        &self,
        guest: &Guest,
        session: &SessionName,
        size: TerminalSize,
    ) -> Result<Terminal, RuntimeError> {
        let run = self.run_dir(guest.name).await?;
        self.open_terminal(guest, &run, session, size)
    }

    async fn read(&self, guest: &Guest, file: GuestFile) -> Result<Option<Vec<u8>>, RuntimeError> {
        let relative = match file {
            // Local processes have no boot to wait for.
            GuestFile::Ready => return Ok(Some(Vec::new())),
            GuestFile::SecretsGeneration => guest_tools::SECRETS_GENERATION,
            GuestFile::Status => guest_tools::STATUS,
            GuestFile::ColumnsOpened => guest_tools::COLUMNS_OPENED,
        };
        let path = self.run_dir(guest.name).await?.join(relative);
        tokio::task::spawn_blocking(move || read_guest_file(&path))
            .await
            .map_err(RuntimeError::failed)?
            .map_err(RuntimeError::failed)
    }

    fn channel(&self, name: InstanceName) -> PathBuf {
        self.layout.channel(name)
    }

    async fn connect(&self, _guest: &Guest, port: GuestPort) -> Result<TcpStream, RuntimeError> {
        TcpStream::connect(("127.0.0.1", port.get()))
            .await
            .map_err(RuntimeError::failed)
    }
}

/// Pumps a terminal's PTY to and from a [`Terminal`]'s channels. Dropping
/// the input ends the attach process, which detaches from the session.
fn relay(pty: pty_process::Pty, mut child: tokio::process::Child) -> Terminal {
    let (input, mut from_client) = mpsc::channel(64);
    let (to_client, output) = mpsc::channel::<Bytes>(64);
    let (mut reader, mut writer) = pty.into_split();
    tokio::spawn(async move {
        while let Some(message) = from_client.recv().await {
            let sent = match message {
                TerminalInput::Bytes(bytes) => writer.write_all(&bytes).await,
                TerminalInput::Resize(size) => writer
                    .resize(pty_process::Size::new(size.rows(), size.cols()))
                    .map_err(std::io::Error::other),
            };
            if sent.is_err() {
                break;
            }
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    });
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if to_client
                        .send(Bytes::copy_from_slice(&buffer[..read]))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    Terminal { input, output }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_environment_is_identified_by_its_source() {
        let a: EnvSource = "github:alice/env#dev".parse().expect("valid");
        let b: EnvSource = "github:alice/env#ops".parse().expect("valid");
        assert_eq!(fingerprint(&a), fingerprint(&a));
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn workspaces_trust_a_localhost_ca_only_for_localhost() {
        let tools = Path::new("/tools");
        assert_eq!(
            system_gitconfig(tools, None),
            "[credential]\n\thelper = /tools/iglu-guest git-credential\n"
        );
        let with_ca = system_gitconfig(tools, Some(Path::new("/dev ca/\"ca\".crt")));
        assert!(with_ca.ends_with(
            "[http \"https://*.localhost\"]\n\tsslCAInfo = \"/dev ca/\\\"ca\\\".crt\"\n"
        ));
    }
}
