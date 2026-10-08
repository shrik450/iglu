//! Terminal sessions as zmx sessions.

use std::collections::BTreeMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use iglu_domain::terminal::SessionName;
use serde::Serialize;

use crate::paths::Dirs;
use crate::provision::WorkspaceRecord;

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
pub fn session_env(secrets: BTreeMap<String, String>, zmx_dir: &Path) -> BTreeMap<String, String> {
    let mut env = secrets;
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

/// Replaces this process with `zmx attach`, creating the session in the
/// repository checkout on first attach.
pub fn attach(dirs: &Dirs, name: &SessionName) -> std::io::Error {
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
    Command::new("zmx")
        .arg("attach")
        .arg(name.as_str())
        .envs(session_env(secrets, &zmx_dir(dirs)))
        .env("SHELL", shell)
        .current_dir(checkout)
        .exec()
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
        let env = session_env(secrets, Path::new("/run/user/1000/zmx"));
        assert_eq!(env["TERM"], "xterm-256color");
        assert_eq!(env["API_KEY"], "k");
        assert_eq!(env["ZMX_DIR"], "/run/user/1000/zmx");
    }
}
