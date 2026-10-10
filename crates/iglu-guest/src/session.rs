//! Terminal sessions as zmx sessions.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use iglu_domain::column::{Arg, Argv};
use iglu_domain::guest::CHANNEL_ENV;
use iglu_domain::terminal::{SessionName, TerminalInput};
use serde::{Deserialize, Serialize};

use crate::paths::Dirs;
use crate::provision::WorkspaceRecord;
use crate::status::write_atomic;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Info {
    pub name: SessionName,
    pub clients: u32,
}

/// Parses `zmx list`: one session per line, tab-separated, name first and
/// the attached-client count third. Lines that don't parse are skipped.
#[must_use]
pub fn parse_list(output: &str) -> Vec<Info> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line
                .split('\t')
                .map(|field| field.split_once('=').map_or(field, |(_, v)| v).trim());
            let name: SessionName = fields.next()?.parse().ok()?;
            let clients = fields.nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
            Some(Info { name, clients })
        })
        .collect()
}

/// The environment for a session: the delivered secrets first, then the
/// platform's own variables, which secrets can't override.
#[must_use]
pub fn session_env(
    secrets: BTreeMap<String, String>,
    zmx_dir: &Path,
    channel: Option<&Path>,
) -> BTreeMap<String, String> {
    let mut env = secrets;
    // Where `iglu` reaches the control plane from.
    if let Some(channel) = channel {
        env.insert(CHANNEL_ENV.into(), channel.display().to_string());
    }
    env.insert("ZMX_DIR".into(), zmx_dir.display().to_string());
    env.insert("ZMX_NO_DETACH_KEY".into(), "1".into());
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    env
}

fn zmx_dir(dirs: &Dirs) -> PathBuf {
    dirs.runtime().join("zmx")
}

fn zmx(dirs: &Dirs) -> Command {
    let mut command = Command::new("zmx");
    command.env("ZMX_DIR", zmx_dir(dirs));
    command
}

/// # Errors
///
/// When zmx can't run or reports a failure.
pub fn list(dirs: &Dirs) -> std::io::Result<Vec<Info>> {
    let output = zmx(dirs).arg("list").output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(parse_list(&String::from_utf8_lossy(&output.stdout)))
}

/// # Errors
///
/// When zmx can't run or reports a failure.
pub fn close(dirs: &Dirs, name: &SessionName) -> std::io::Result<()> {
    let status = zmx(dirs).arg("kill").arg(name.as_str()).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("zmx kill failed"))
    }
}

/// A session's history and screen as plain text, as zmx keeps them.
///
/// # Errors
///
/// When the session isn't open or zmx can't read it.
pub fn history(dirs: &Dirs, name: &SessionName) -> std::io::Result<String> {
    let output = zmx(dirs).arg("history").arg(name.as_str()).output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Types into an open session, exactly as given.
///
/// # Errors
///
/// When the session isn't open or zmx can't reach it.
pub fn send(dirs: &Dirs, name: &SessionName, text: &TerminalInput) -> std::io::Result<()> {
    // zmx would take a closed session's leftover socket for a busy one.
    if !list(dirs)?.iter().any(|info| info.name == *name) {
        return Err(std::io::Error::other(format!(
            "no session called \"{name}\" is open"
        )));
    }
    // As one argument: zmx sends it as it is, where from its input it would
    // drop a final newline.
    let output = zmx(dirs)
        .arg("send")
        .arg(name.as_str())
        .arg(text.as_str())
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

/// `zmx attach` for a session, in the checkout, with the delivered secrets.
/// With a command, a new session runs it as given, argument by argument;
/// without one, it runs the login shell.
fn zmx_attach(dirs: &Dirs, name: &SessionName, command: Option<&Argv>) -> Command {
    let secrets: BTreeMap<String, String> = std::fs::read(dirs.secrets_env())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let checkout = std::fs::read(dirs.workspace_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<WorkspaceRecord>(&bytes).ok())
        .map(|record| record.checkout)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| dirs.home().to_owned());
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(login_shell);
    let mut zmx = Command::new("zmx");
    zmx.arg("attach").arg(name.as_str());
    if let Some(command) = command {
        zmx.args(command.args().iter().map(Arg::as_str));
    }
    zmx.envs(session_env(secrets, &zmx_dir(dirs), dirs.channel()))
        .env("SHELL", shell)
        .current_dir(checkout);
    zmx
}

/// Replaces this process with `zmx attach` to a session that's open.
/// Attaching never opens one: a column whose program ended stays ended
/// rather than quietly becoming a shell, and a column closed while a client
/// is on its way in stays closed. zmx itself refuses, so there's no gap
/// between looking for the session and attaching to it.
#[must_use]
pub fn attach(dirs: &Dirs, name: &SessionName) -> std::io::Error {
    zmx_attach(dirs, name, None)
        .env("ZMX_NO_CREATE", "1")
        .exec()
}

/// A session to open, as hostd sends it.
#[derive(Debug, Deserialize)]
pub struct OpenSpec {
    pub name: SessionName,
    pub command: Option<Argv>,
}

#[derive(Debug, Deserialize)]
struct OpenRequest {
    sessions: Vec<OpenSpec>,
}

/// Whether an open is a boot's opening of its columns, which gets recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opening {
    Boot,
    Column,
}

/// A session that didn't open, and why.
#[derive(Debug, Serialize)]
pub struct Failure {
    pub name: SessionName,
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("unreadable request: {0}")]
    Request(#[from] serde_json::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Opens every requested session that isn't open yet. A session whose
/// program fails is reported and skipped, not fatal. A boot's opening is
/// recorded either way, so the workspace comes up with what did open.
///
/// # Errors
///
/// When the request is unreadable or zmx can't list sessions.
pub fn open(dirs: &Dirs, request: &[u8], opening: Opening) -> Result<Vec<Failure>, OpenError> {
    let request: OpenRequest = serde_json::from_slice(request)?;
    let already = list(dirs)?;
    let mut failures = Vec::new();
    for spec in request.sessions {
        if already.iter().any(|info| info.name == spec.name) {
            continue;
        }
        if let Err(error) = start(dirs, &spec) {
            failures.push(Failure {
                name: spec.name,
                reason: error.to_string(),
            });
        }
    }
    if opening == Opening::Boot {
        let marker = dirs.columns_opened();
        if let Some(parent) = marker.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(&marker, b"opened\n")?;
    }
    Ok(failures)
}

/// zmx only creates a session from a client attached to a terminal, so this
/// attaches one on a pseudo-terminal of its own and has it detach.
///
/// The client has to detach rather than be killed: the daemon replays a
/// session's screen to later clients only once some client's `Init` has
/// reached it, and a killed client may not have sent one. The client clears
/// its screen just before its loop, which sends `Init` first and then
/// anything typed, so the detach key typed after the clear reaches the daemon
/// behind `Init`.
fn start(dirs: &Dirs, spec: &OpenSpec) -> std::io::Result<()> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

    const CLEARED: &[u8] = b"\x1b[2J\x1b[H";
    const DETACH_KEY: u8 = 0x1c;

    let controller = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    grantpt(&controller)?;
    unlockpt(&controller)?;
    let path = ptsname(&controller, Vec::new())?;
    let terminal = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(OsStr::from_bytes(path.as_bytes()))?;
    rustix::termios::tcsetwinsize(
        &terminal,
        rustix::termios::Winsize {
            ws_row: 40,
            ws_col: 120,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )?;
    let mut client = zmx_attach(dirs, &spec.name, spec.command.as_ref())
        .env_remove("ZMX_NO_DETACH_KEY")
        .stdin(terminal.try_clone()?)
        .stdout(terminal.try_clone()?)
        .stderr(terminal)
        .process_group(0)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    let mut detached = false;
    let left = loop {
        if client.try_wait()?.is_some() {
            break true;
        }
        if Instant::now() > deadline {
            break false;
        }
        // Drained the whole time, so a chatty program can't block the client.
        let mut ready = [PollFd::new(&controller, PollFlags::IN)];
        let tick = Timespec {
            tv_sec: 0,
            tv_nsec: 50_000_000,
        };
        if poll(&mut ready, Some(&tick))? == 0 {
            continue;
        }
        let mut buffer = [0; 4096];
        let Ok(read) = rustix::io::read(&controller, &mut buffer) else {
            continue;
        };
        if !detached {
            seen.extend_from_slice(&buffer[..read]);
            if seen.windows(CLEARED.len()).any(|w| w == CLEARED) {
                rustix::io::write(&controller, &[DETACH_KEY])?;
                detached = true;
            }
        }
    };
    if !left {
        let _ = client.kill();
        let _ = client.wait();
        return Err(std::io::Error::other("its client never detached"));
    }
    drop(controller);
    if list(dirs)?.iter().any(|info| info.name == spec.name) {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "its program ended before the session opened",
        ))
    }
}

/// The user's login shell from the passwd database.
fn login_shell() -> String {
    let user = std::env::var("USER").unwrap_or_default();
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|passwd| {
            passwd.lines().find_map(|line| {
                let fields: Vec<&str> = line.split(':').collect();
                (fields.first() == Some(&user.as_str()))
                    .then(|| fields.get(6).map(|s| (*s).to_owned()))
                    .flatten()
            })
        })
        .unwrap_or_else(|| "/bin/sh".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_zmx_list() {
        let output = "t1\t1234\t2\t/home/alice/app\nbad name\t1\t0\t/\nt2\t99\t0\t/home/alice\n";
        let sessions = parse_list(output);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name.as_str(), "t1");
        assert_eq!(sessions[0].clients, 2);
        assert_eq!(sessions[1].clients, 0);
    }

    #[test]
    fn parses_labelled_fields_too() {
        let sessions = parse_list("session_name=t3\tpid=1\tclients=1\tstart_dir=/tmp\n");
        assert_eq!(
            sessions,
            [Info {
                name: "t3".parse().expect("valid"),
                clients: 1
            }]
        );
    }

    #[test]
    fn secrets_cannot_override_platform_variables() {
        let secrets = BTreeMap::from([
            ("TERM".to_owned(), "dumb".to_owned()),
            ("API_KEY".to_owned(), "k".to_owned()),
        ]);
        let env = session_env(
            secrets,
            Path::new("/run/user/1000/zmx"),
            Some(Path::new("/var/lib/iglu-api.sock")),
        );
        assert_eq!(env["TERM"], "xterm-256color");
        assert_eq!(env["API_KEY"], "k");
        assert_eq!(env["ZMX_DIR"], "/run/user/1000/zmx");
        assert_eq!(env[CHANNEL_ENV], "/var/lib/iglu-api.sock");
    }
}
