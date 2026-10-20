//! The workspace's browser: Chromium on a display of its own, a headless
//! sway, which wayvnc shows the console and which agents drive over
//! Chrome's debugging protocol (CDP) on loopback. A browser column runs
//! all three, and they end with it.
//!
//! The display draws on the host's GPU when the host gives the workspace
//! one and the environment has its drivers, and on the CPU otherwise.

use std::io::{Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use iglu_domain::column::{BROWSER_DEBUG_PORT, BROWSER_DISPLAY_PORT};
use iglu_domain::display::{DisplayReport, DisplayScale};
use iglu_domain::download;
use serde_json::{Value, json};

use crate::paths::Dirs;

/// Where the environment's GPU drivers are, as NixOS installs them.
const DRIVERS: &str = "/run/opengl-driver/lib/dri";

/// Where render nodes are: a GPU for drawing, without its display.
const RENDER_NODES: &str = "/dev/dri";

/// What the display found it draws with, for `browser-display` to report.
const REPORT: &str = "display.json";

/// How long sway has to make its sockets.
const STARTING: Duration = Duration::from_secs(10);

/// A program on the display that ends sooner than this after starting is
/// waited for longer each time before it's started again.
const STEADY: Duration = Duration::from_secs(10);

/// sway's configuration: one output, no bars or borders, and tabs, so a
/// window an agent opens doesn't cover the person's for good. No primary
/// selection: wayvnc sends the console whatever the browser selects as
/// though it were copied, which took the person's clipboard, so selecting
/// a field's text before pasting pasted that text back.
#[must_use]
pub fn sway_config() -> String {
    [
        "output * mode 1280x800 scale 1 bg #1e1e1e solid_color",
        "default_border none",
        "default_floating_border none",
        "hide_edge_borders --i3 smart",
        "workspace_layout tabbed",
        "focus_on_window_activation focus",
        "xwayland disable",
        "primary_selection disabled",
    ]
    .join("\n")
        + "\n"
}

/// The render node to draw on: the first there is, when the environment
/// has drivers to use it with.
#[must_use]
pub fn hardware(nodes: Vec<PathBuf>, drivers: bool) -> Option<PathBuf> {
    if drivers {
        nodes.into_iter().min()
    } else {
        None
    }
}

/// sway's environment: headless, with its sockets in `dir`, drawing on
/// `node` or on the CPU.
#[must_use]
pub fn sway_env(dir: &Path, node: Option<&Path>) -> Vec<(&'static str, String)> {
    let mut env = vec![
        ("XDG_RUNTIME_DIR", dir.display().to_string()),
        ("WLR_BACKENDS", "headless".to_owned()),
        ("WLR_HEADLESS_OUTPUTS", "1".to_owned()),
        ("WLR_LIBINPUT_NO_DEVICES", "1".to_owned()),
    ];
    match node {
        Some(node) => {
            env.push(("WLR_RENDERER", "gles2".to_owned()));
            env.push(("WLR_RENDER_DRM_DEVICE", node.display().to_string()));
        }
        None => env.push(("WLR_RENDERER", "pixman".to_owned())),
    }
    env
}

/// wayvnc's arguments: on the workspace's loopback, where only iglu reaches
/// it from outside, using the GPU to encode H.264 when there is one.
#[must_use]
pub fn wayvnc_args(hardware: bool) -> Vec<String> {
    let mut args = Vec::new();
    if hardware {
        args.push("--gpu".to_owned());
    }
    args.extend(["127.0.0.1".to_owned(), BROWSER_DISPLAY_PORT.to_string()]);
    args
}

/// Chromium's arguments: on the display, debuggable only from inside the
/// workspace, with a profile kept in the home directory so sign-ins last.
#[must_use]
pub fn chromium_args(profile: &Path) -> Vec<String> {
    vec![
        "--ozone-platform=wayland".into(),
        "--remote-debugging-address=127.0.0.1".into(),
        format!("--remote-debugging-port={BROWSER_DEBUG_PORT}"),
        format!("--user-data-dir={}", profile.display()),
        // Cookies are kept in the profile as they are: no keyring to wait on.
        "--password-store=basic".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        // Ending a column ends the browser, which isn't a crash to recover.
        "--hide-crash-restore-bubble".into(),
    ]
}

/// The profile's preferences, as Chromium keeps them in `Default/Preferences`,
/// with downloads going to `dir` without asking: the person saves a
/// download to their own computer from there. The rest are kept;
/// preferences Chromium can't read are started afresh, as Chromium would.
#[must_use]
pub fn with_downloads(preferences: Option<&str>, dir: &Path) -> String {
    let mut preferences = preferences
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let download = &mut preferences["download"];
    if !download.is_object() {
        *download = json!({});
    }
    download["default_directory"] = json!(dir.display().to_string());
    download["prompt_for_download"] = json!(false);
    download["directory_upgrade"] = json!(true);
    preferences.to_string()
}

/// The name of the file in `names` that's a socket of sway's: its Wayland
/// display, `wayland-<n>`, or its IPC socket, `sway-ipc.<uid>.<pid>.sock`.
#[must_use]
pub fn socket(names: &[String], ipc: bool) -> Option<String> {
    names
        .iter()
        .find(|name| {
            if ipc {
                name.starts_with("sway-ipc.")
                    && Path::new(name).extension().is_some_and(|e| e == "sock")
            } else {
                name.strip_prefix("wayland-")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            }
        })
        .cloned()
}

/// What sway's IPC is sent to run `command`: i3's framing, type 0.
#[must_use]
pub fn ipc_command(command: &str) -> Vec<u8> {
    let mut message = b"i3-ipc".to_vec();
    let length = u32::try_from(command.len()).unwrap_or(u32::MAX);
    message.extend(length.to_ne_bytes());
    message.extend(0_u32.to_ne_bytes());
    message.extend(command.as_bytes());
    message
}

/// Whether sway ran every part of a command, from its reply's payload.
///
/// # Errors
///
/// What sway said went wrong.
pub fn ipc_succeeded(payload: &[u8]) -> Result<(), String> {
    let results: Vec<Value> =
        serde_json::from_slice(payload).map_err(|e| format!("sway's reply: {e}"))?;
    match results.iter().find(|r| r["success"] != true) {
        None => Ok(()),
        Some(failed) => Err(failed["error"]
            .as_str()
            .unwrap_or("sway refused the command")
            .to_owned()),
    }
}

/// The command that draws the display at `scale`.
#[must_use]
pub fn scale_command(scale: DisplayScale) -> String {
    format!("output * scale {scale}")
}

fn names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Render nodes this user can open, to draw on.
fn render_nodes() -> Vec<PathBuf> {
    names(Path::new(RENDER_NODES))
        .into_iter()
        .filter(|name| name.starts_with("renderD"))
        .map(|name| Path::new(RENDER_NODES).join(name))
        .filter(|node| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(node)
                .is_ok()
        })
        .collect()
}

fn spawn(
    program: &str,
    args: &[String],
    env: &[(&str, String)],
    log: &Path,
) -> std::io::Result<Child> {
    // A program started again writes after what it said before it ended.
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)?;
    Command::new(program)
        .args(args)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                std::io::Error::other(format!(
                    "this workspace's environment has no {program}; the iglu module adds it unless iglu.browser.enable is off"
                ))
            } else {
                error
            }
        })
}

/// Sets the profile up as the browser expects it: its downloads in the
/// user's Downloads, and no tabs from last time.
fn prepare_profile(dirs: &Dirs, profile: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(profile)?;
    // Ending a column kills the browser, which takes it for a crash and
    // would bring the last session's tabs back, unloaded. It starts afresh
    // instead, with one blank tab; sign-ins and history stay.
    match std::fs::remove_dir_all(profile.join("Default/Sessions")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let downloads = dirs.home().join(download::DIR);
    let preferences = profile.join("Default/Preferences");
    std::fs::create_dir_all(&downloads)?;
    std::fs::create_dir_all(profile.join("Default"))?;
    let now = std::fs::read_to_string(&preferences).ok();
    std::fs::write(&preferences, with_downloads(now.as_deref(), &downloads))
}

fn end(children: &mut [&mut Child]) {
    for child in children.iter_mut() {
        let _ = child.kill();
    }
    for child in children.iter_mut() {
        let _ = child.wait();
    }
}

/// A program that shows or draws on the display, started again whenever it
/// ends: the browser when its last window is closed, and the VNC server when
/// the display drops it for a request it refuses, which leaves the browser
/// as it was. One that keeps ending at once is waited for longer each time.
struct Kept {
    program: &'static str,
    args: Vec<String>,
    log: PathBuf,
    child: Option<Child>,
    started: Instant,
    next: Instant,
    wait: Duration,
}

impl Kept {
    fn new(program: &'static str, args: Vec<String>, log: PathBuf) -> Self {
        let now = Instant::now();
        Self {
            program,
            args,
            log,
            child: None,
            started: now,
            next: now,
            wait: Duration::ZERO,
        }
    }

    /// Starts the program if it isn't running and its wait is over.
    fn keep(&mut self, env: &[(&str, String)]) -> std::io::Result<()> {
        if let Some(child) = self.child.as_mut()
            && let Some(status) = child.try_wait()?
        {
            eprintln!(
                "iglu-guest: {} ended ({status}); starting it again",
                self.program
            );
            self.child = None;
            self.wait = if self.started.elapsed() < STEADY {
                (self.wait * 2).clamp(Duration::from_millis(500), STEADY)
            } else {
                Duration::ZERO
            };
            self.next = Instant::now() + self.wait;
        }
        if self.child.is_none() && Instant::now() >= self.next {
            self.started = Instant::now();
            self.child = Some(spawn(self.program, &self.args, env, &self.log)?);
        }
        Ok(())
    }
}

/// Runs the display, the VNC server that shows it, and the browser on it
/// until the column ends, starting the browser or the VNC server again
/// whenever either ends.
///
/// # Errors
///
/// When the display can't start or ends on its own, or a program on it
/// can't be started.
pub fn run(dirs: &Dirs) -> std::io::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(signal, Arc::clone(&stop))?;
    }
    let profile = dirs.home().join(".cache/iglu/browser");
    prepare_profile(dirs, &profile)?;
    let dir = dirs.display();
    let (mut sway, display, report) = start_display(&dir)?;
    // Chromium and wayvnc find the display in its own runtime directory.
    let env = vec![
        ("XDG_RUNTIME_DIR", dir.display().to_string()),
        ("WAYLAND_DISPLAY", display),
    ];
    let mut kept = [
        Kept::new(
            "wayvnc",
            wayvnc_args(report.hardware),
            dir.join("wayvnc.log"),
        ),
        Kept::new(
            "chromium",
            chromium_args(&profile),
            dir.join("chromium.log"),
        ),
    ];
    let failed = loop {
        if stop.load(Ordering::Relaxed) {
            break None;
        }
        if let Err(error) = tend(&mut sway, &mut kept, &env) {
            break Some(error);
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let mut children: Vec<&mut Child> = kept.iter_mut().filter_map(|k| k.child.as_mut()).collect();
    children.push(&mut sway);
    end(&mut children);
    failed.map_or(Ok(()), Err)
}

/// Checks the display is still there, and keeps what's on it running.
fn tend(sway: &mut Child, kept: &mut [Kept], env: &[(&str, String)]) -> std::io::Result<()> {
    if let Some(status) = sway.try_wait()? {
        return Err(std::io::Error::other(format!(
            "the browser's display ended: {status}"
        )));
    }
    kept.iter_mut().try_for_each(|program| program.keep(env))
}

/// Starts sway in `dir`, a display of its own, afresh: the last one's
/// sockets go with it. Returns once it's taking clients, with its Wayland
/// display's name and what it draws with: the GPU if it can, and the CPU
/// if there's none or drawing on it fails.
fn start_display(dir: &Path) -> std::io::Result<(Child, String, DisplayReport)> {
    let found = hardware(render_nodes(), Path::new(DRIVERS).is_dir());
    let tries = if found.is_some() {
        vec![found, None]
    } else {
        vec![None]
    };
    let mut failed = None;
    for node in tries {
        match start_sway(dir, node.as_deref()) {
            Ok((sway, display)) => {
                let report = DisplayReport {
                    hardware: node.is_some(),
                };
                std::fs::write(dir.join(REPORT), serde_json::to_vec(&report)?)?;
                return Ok((sway, display, report));
            }
            Err(error) => {
                eprintln!("iglu-guest: {error}");
                failed = Some(error);
            }
        }
    }
    Err(failed.unwrap_or_else(|| std::io::Error::other("the browser's display didn't start")))
}

fn start_sway(dir: &Path, node: Option<&Path>) -> std::io::Result<(Child, String)> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let config = dir.join("sway.conf");
    std::fs::write(&config, sway_config())?;
    let log = dir.join("sway.log");
    let args = ["-c".to_owned(), config.display().to_string()];
    let mut sway = spawn("sway", &args, &sway_env(dir, node), &log)?;
    let deadline = Instant::now() + STARTING;
    loop {
        if let Some(display) = socket(&names(dir), false) {
            return Ok((sway, display));
        }
        if Instant::now() > deadline || sway.try_wait()?.is_some() {
            end(&mut [&mut sway]);
            let said = std::fs::read_to_string(&log).unwrap_or_default();
            let on = node.map_or("the CPU".to_owned(), |n| n.display().to_string());
            return Err(std::io::Error::other(format!(
                "the browser's display didn't start drawing on {on}: {}",
                said.lines().rev().take(3).collect::<Vec<_>>().join(" / ")
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Draws the browser's display at `scale`, and says what it draws with.
///
/// # Errors
///
/// When the browser isn't running, or sway refuses the scale.
pub fn set_scale(dirs: &Dirs, scale: DisplayScale) -> Result<DisplayReport, String> {
    let dir = dirs.display();
    let report: DisplayReport = std::fs::read(dir.join(REPORT))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("the browser isn't running; open or restart its column")?;
    let ipc = socket(&names(&dir), true).ok_or("the browser's display hasn't started yet")?;
    let mut stream =
        UnixStream::connect(dir.join(ipc)).map_err(|e| format!("reaching the display: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(&ipc_command(&scale_command(scale)))
        .map_err(|e| format!("asking the display: {e}"))?;
    let mut header = [0_u8; 14];
    stream
        .read_exact(&mut header)
        .map_err(|e| format!("the display's answer: {e}"))?;
    let mut length = [0_u8; 4];
    length.copy_from_slice(&header[6..10]);
    let length = usize::try_from(u32::from_ne_bytes(length)).map_err(|e| e.to_string())?;
    let mut payload = vec![0_u8; length.min(64 * 1024)];
    stream
        .read_exact(&mut payload)
        .map_err(|e| format!("the display's answer: {e}"))?;
    ipc_succeeded(&payload)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_is_on_its_display_and_debuggable_only_from_inside() {
        let args = chromium_args(Path::new("/home/dev/.cache/iglu/browser"));
        assert!(args.contains(&"--ozone-platform=wayland".to_owned()));
        assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_owned()));
        assert!(args.contains(&format!("--remote-debugging-port={BROWSER_DEBUG_PORT}")));
        assert!(args.contains(&"--user-data-dir=/home/dev/.cache/iglu/browser".to_owned()));
        assert!(!args.iter().any(|a| a.starts_with("--headless")));
        assert_eq!(wayvnc_args(false), ["127.0.0.1", "5900"]);
        assert_eq!(wayvnc_args(true), ["--gpu", "127.0.0.1", "5900"]);
    }

    #[test]
    fn the_display_draws_on_a_gpu_only_with_drivers_for_it() {
        let nodes = vec![
            PathBuf::from("/dev/dri/renderD129"),
            PathBuf::from("/dev/dri/renderD128"),
        ];
        assert_eq!(
            hardware(nodes.clone(), true),
            Some(PathBuf::from("/dev/dri/renderD128"))
        );
        assert_eq!(hardware(nodes, false), None);
        assert_eq!(hardware(Vec::new(), true), None);
        let dir = Path::new("/run/user/1000/iglu/display");
        let env = sway_env(dir, Some(Path::new("/dev/dri/renderD128")));
        assert!(env.contains(&("WLR_RENDERER", "gles2".to_owned())));
        assert!(env.contains(&("WLR_RENDER_DRM_DEVICE", "/dev/dri/renderD128".to_owned())));
        assert!(env.contains(&("XDG_RUNTIME_DIR", dir.display().to_string())));
        let env = sway_env(dir, None);
        assert!(env.contains(&("WLR_RENDERER", "pixman".to_owned())));
        assert!(!env.iter().any(|(k, _)| *k == "WLR_RENDER_DRM_DEVICE"));
    }

    #[test]
    fn sways_sockets_are_found_by_name() {
        let names: Vec<String> = [
            "wayland-1.lock",
            "sway-ipc.1000.42.sock",
            "wayland-1",
            "display.json",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(socket(&names, false).as_deref(), Some("wayland-1"));
        assert_eq!(
            socket(&names, true).as_deref(),
            Some("sway-ipc.1000.42.sock")
        );
        assert_eq!(socket(&names[..1], false), None);
    }

    #[test]
    fn a_scale_is_asked_of_sway_in_its_framing() {
        let command = scale_command("1.5".parse().expect("a scale"));
        assert_eq!(command, "output * scale 1.5");
        let message = ipc_command(&command);
        assert_eq!(&message[..6], b"i3-ipc");
        assert_eq!(
            u32::from_ne_bytes(message[6..10].try_into().expect("4")),
            18
        );
        assert_eq!(
            u32::from_ne_bytes(message[10..14].try_into().expect("4")),
            0
        );
        assert_eq!(&message[14..], command.as_bytes());
        assert_eq!(ipc_succeeded(br#"[{"success":true}]"#), Ok(()));
        assert_eq!(
            ipc_succeeded(br#"[{"success":false,"error":"no output"}]"#),
            Err("no output".to_owned())
        );
        assert!(ipc_succeeded(b"nonsense").is_err());
    }

    #[test]
    fn downloads_go_to_downloads_without_asking_and_other_preferences_stay() {
        let dir = Path::new("/home/dev/Downloads");
        let kept = r#"{"profile":{"name":"Person"},"download":{"extensions_to_open":"x"}}"#;
        let set: Value = serde_json::from_str(&with_downloads(Some(kept), dir)).expect("JSON");
        assert_eq!(set["profile"]["name"], "Person");
        assert_eq!(set["download"]["extensions_to_open"], "x");
        assert_eq!(set["download"]["default_directory"], "/home/dev/Downloads");
        assert_eq!(set["download"]["prompt_for_download"], false);
        for unreadable in [
            None,
            Some("not json"),
            Some("[1]"),
            Some(r#"{"download":3}"#),
        ] {
            let set: Value = serde_json::from_str(&with_downloads(unreadable, dir)).expect("JSON");
            assert_eq!(
                set["download"]["default_directory"], "/home/dev/Downloads",
                "{unreadable:?}"
            );
        }
    }
}
