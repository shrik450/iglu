//! The guest tools' interface, as hostd uses it: `iglu-guest`'s command
//! line, which every runtime runs with its own path to the tool, and the
//! files the tools write for hosts. Each argument is a constant or a parsed
//! domain value; nothing goes through a shell. Everything read back is
//! untrusted.

use std::collections::BTreeMap;

use iglu_domain::attention::{self, AttentionState, SessionStatus, ThreadKey};
use iglu_domain::git::{GitState, ObservedBranch};
use iglu_domain::guest::{GitReport, ListenerReport, StatusEntry};
use iglu_domain::lifecycle::SecretsGeneration;
use iglu_domain::listener::{self, Listener, ProcessName};
use iglu_domain::pasted::FileName;
use iglu_domain::port::GuestPort;
use iglu_domain::secret::SecretBundle;
use iglu_domain::terminal::{OutputLines, SessionName, TerminalInput};
use iglu_domain::time::Timestamp;
use iglu_proto::{ProvisionSpec, SessionSpec, TerminalInfo};

/// A guest tool command that runs to completion.
#[derive(Clone, Copy, Debug)]
pub enum GuestCommand<'a> {
    /// Replaces the delivered secrets; the bundle goes on the input.
    InstallSecrets(&'a SecretBundle),
    /// Clones the repository and checks out the branch.
    Provision(&'a ProvisionSpec),
    /// Opens the sessions that aren't open yet; the specs go on the input.
    Open(&'a [SessionSpec], Opening),
    /// Lists terminal sessions as JSON.
    Sessions,
    /// Lists what the workspace user is listening on, as JSON.
    Listeners,
    /// Where the checkout stands, as JSON.
    GitState,
    Close(&'a SessionName),
    /// Prints a session's last lines as plain text.
    Output(&'a SessionName, OutputLines),
    /// Types into a session; the text goes on the input.
    Input(&'a SessionName, &'a TerminalInput),
    /// Keeps a pasted file, which goes on the input, and prints its path.
    Keep(&'a FileName, &'a [u8]),
}

/// Whether an open is the boot's opening of the workspace's columns, which
/// the guest records, or one more column later on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opening {
    Boot,
    Column,
}

#[derive(serde::Serialize)]
struct InstallRequest<'a> {
    bundle: &'a SecretBundle,
}

#[derive(serde::Serialize)]
struct OpenRequest<'a> {
    sessions: &'a [SessionSpec],
}

impl GuestCommand<'_> {
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        match self {
            Self::InstallSecrets(_) => vec!["install-secrets".into()],
            Self::Provision(spec) => {
                let mut args = vec!["provision".into()];
                if let Some(checkout) = &spec.checkout {
                    args.extend([
                        "--repo".into(),
                        checkout.repo.to_string(),
                        "--branch".into(),
                        checkout.branch.to_string(),
                    ]);
                    if let Some(base) = &checkout.base {
                        args.extend(["--base".into(), base.to_string()]);
                    }
                }
                args
            }
            Self::Open(_, Opening::Boot) => vec!["open".into(), "--boot".into()],
            Self::Open(_, Opening::Column) => vec!["open".into()],
            Self::Sessions => vec!["sessions".into()],
            Self::Listeners => vec!["listeners".into()],
            Self::GitState => vec!["git-state".into()],
            Self::Close(session) => vec!["close".into(), session.to_string()],
            Self::Output(session, lines) => vec![
                "output".into(),
                session.to_string(),
                "--lines".into(),
                lines.to_string(),
            ],
            Self::Input(session, _) => vec!["input".into(), session.to_string()],
            Self::Keep(name, _) => vec!["keep".into(), name.to_string()],
        }
    }

    /// What the command reads on its input, if anything.
    ///
    /// # Panics
    ///
    /// Never: bundles and session specs have only string keys, so they
    /// always serialize.
    #[must_use]
    pub fn input(&self) -> Option<Vec<u8>> {
        match self {
            Self::InstallSecrets(bundle) => Some(
                serde_json::to_vec(&InstallRequest { bundle })
                    .expect("a bundle has only string keys, so it always serializes"),
            ),
            Self::Open(sessions, _) => Some(
                serde_json::to_vec(&OpenRequest { sessions })
                    .expect("session specs have only string keys, so they always serialize"),
            ),
            Self::Input(_, text) => Some(text.as_str().as_bytes().to_vec()),
            Self::Keep(_, bytes) => Some(bytes.to_vec()),
            Self::Provision(_)
            | Self::Sessions
            | Self::Listeners
            | Self::GitState
            | Self::Close(_)
            | Self::Output(..) => None,
        }
    }
}

/// Attaches to a session that's open. Runs on a terminal.
#[must_use]
pub fn attach_args(session: &SessionName) -> Vec<String> {
    vec!["attach".into(), session.to_string()]
}

/// Parses the secrets generation file. Untrusted: anything but a decimal
/// number means the guest holds no known delivery.
#[must_use]
pub fn parse_generation(bytes: &[u8]) -> Option<SecretsGeneration> {
    std::str::from_utf8(bytes)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(SecretsGeneration::from_u64)
}

/// Parses the guest's status file: sessions, each with its threads.
/// Untrusted: malformed entries are dropped individually, each session keeps
/// only the threads the core says to, and the whole file is capped.
#[must_use]
pub fn parse_statuses(bytes: &[u8]) -> Vec<SessionStatus> {
    const MAX_STATUSES: usize = 64;
    let Ok(raw) = serde_json::from_slice::<BTreeMap<String, serde_json::Value>>(bytes) else {
        return Vec::new();
    };
    let mut statuses = Vec::new();
    for (session, threads) in raw {
        let Ok(threads) = serde_json::from_value::<BTreeMap<String, serde_json::Value>>(threads)
        else {
            continue;
        };
        let Ok(session) = session.parse::<SessionName>() else {
            continue;
        };
        let parsed: Vec<SessionStatus> = threads
            .into_iter()
            .filter_map(|(thread, value)| {
                let status: StatusEntry = serde_json::from_value(value).ok()?;
                Some(SessionStatus {
                    session: session.clone(),
                    thread: thread.parse::<ThreadKey>().ok()?,
                    title: status.title,
                    state: status.state,
                    summary: status.summary,
                    updated_at: Timestamp::from_unix_millis(status.at),
                })
            })
            .collect();
        let listed: Vec<(usize, AttentionState, Timestamp)> = parsed
            .iter()
            .enumerate()
            .map(|(index, status)| (index, status.state, status.updated_at))
            .collect();
        let mut kept = attention::keep(&listed);
        kept.sort_unstable();
        statuses.extend(
            parsed
                .into_iter()
                .enumerate()
                .filter(|(index, _)| kept.binary_search(index).is_ok())
                .map(|(_, status)| status),
        );
    }
    attention::cap(statuses, MAX_STATUSES)
}

/// Parses `iglu-guest listeners`. Untrusted: malformed entries are dropped,
/// each port shows once, and the list is capped.
#[must_use]
pub fn parse_listeners(stdout: &str) -> Vec<Listener> {
    const MAX_LISTENERS: usize = 64;
    let Ok(raw) = serde_json::from_str::<Vec<serde_json::Value>>(stdout) else {
        return Vec::new();
    };
    let parsed = raw
        .into_iter()
        .filter_map(|value| {
            let raw: ListenerReport = serde_json::from_value(value).ok()?;
            Some(Listener {
                port: GuestPort::try_from(raw.port).ok()?,
                process: ProcessName::sanitize(&raw.process),
                column: raw.session.and_then(|s| s.parse().ok()),
                reachable: listener::reachable(raw.address),
            })
        })
        .collect();
    listener::merge(parsed, MAX_LISTENERS)
}

/// Parses `iglu-guest git-state`. Untrusted: names are sanitized, and
/// anything malformed reads as no state.
#[must_use]
pub fn parse_git_state(stdout: &str) -> Option<GitState> {
    let raw: GitReport = serde_json::from_str::<Option<GitReport>>(stdout).ok()??;
    Some(GitState {
        branch: raw.branch.as_deref().and_then(ObservedBranch::sanitize),
        upstream: raw.upstream.as_deref().and_then(ObservedBranch::sanitize),
        ahead: raw.ahead,
        behind: raw.behind,
        changed: raw.changed,
        untracked: raw.untracked,
        conflicted: raw.conflicted,
    })
}

/// Parses `iglu-guest sessions`. Untrusted, like everything from a guest.
///
/// # Errors
///
/// When the output isn't a session list.
pub fn parse_sessions(stdout: &str) -> Result<Vec<TerminalInfo>, serde_json::Error> {
    serde_json::from_str(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_drop_malformed_entries() {
        let file = br#"{
            "t1": {
                "s1": {"state": "waiting", "summary": "approve Bash?", "title": "fix login", "at": 5},
                "s2": {"state": "exploding", "at": 5},
                "../s": {"state": "done", "at": 5}
            },
            "../x": {"session": {"state": "done", "at": 1}},
            "t3": {"session": {"state": "done", "summary": "", "title": "", "at": 7}},
            "t4": "not threads"
        }"#;
        let parsed = parse_statuses(file);
        let names: Vec<_> = parsed
            .iter()
            .map(|s| (s.session.as_str(), s.thread.as_str()))
            .collect();
        assert_eq!(names, [("t1", "s1"), ("t3", "session")]);
        assert_eq!(parsed[0].summary.as_str(), "approve Bash?");
        assert_eq!(parsed[0].title.as_str(), "fix login");
        assert!(parse_statuses(b"not json").is_empty());
    }

    #[test]
    fn listeners_parse_and_merge() {
        let stdout = r#"[
            {"port": 3000, "address": "127.0.0.1", "process": "node", "session": "server"},
            {"port": 3000, "address": "::1", "process": "node", "session": null},
            {"port": 0, "address": "0.0.0.0", "process": "x"},
            {"port": 8080, "address": "::1", "process": "java"},
            {"port": "nope"}
        ]"#;
        let parsed = parse_listeners(stdout);
        let seen: Vec<_> = parsed
            .iter()
            .map(|l| {
                (
                    l.port.get(),
                    l.reachable,
                    l.column.as_ref().map(|c| c.as_str().to_owned()),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [(3000, true, Some("server".into())), (8080, false, None)]
        );
        assert!(parse_listeners("not json").is_empty());
    }

    #[test]
    fn git_state_parses_or_is_absent() {
        let state = parse_git_state(r#"{"branch":"fix\u001b","upstream":null,"ahead":2,"behind":0,"changed":3,"untracked":1,"conflicted":0}"#)
            .expect("a state");
        assert_eq!(
            (
                state.branch.map(|b| b.as_str().to_owned()),
                state.ahead,
                state.changed
            ),
            (Some("fix".into()), 2, 3)
        );
        assert_eq!(parse_git_state("null"), None);
        assert_eq!(parse_git_state(r#"{"branch":"x"}"#), None);
    }

    #[test]
    fn generations_are_decimal_numbers() {
        assert_eq!(
            parse_generation(b"12\n"),
            Some(SecretsGeneration::from_u64(12))
        );
        assert_eq!(parse_generation(b"-1"), None);
        assert_eq!(parse_generation(b"\xff"), None);
    }
}
