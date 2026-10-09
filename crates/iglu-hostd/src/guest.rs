//! The guest tools' interface, as hostd uses it: `iglu-guest`'s command
//! line, which every runtime runs with its own path to the tool, and the
//! files the tools write for hosts. Each argument is a constant or a parsed
//! domain value; nothing goes through a shell. Everything read back is
//! untrusted.

use std::collections::HashMap;

use iglu_domain::attention::{AttentionState, SessionStatus, Summary};
use iglu_domain::lifecycle::SecretsGeneration;
use iglu_domain::secret::SecretBundle;
use iglu_domain::terminal::SessionName;
use iglu_domain::time::Timestamp;
use iglu_proto::{ProvisionSpec, TerminalInfo};
use serde::Deserialize;

/// A guest tool command that runs to completion.
#[derive(Clone, Copy, Debug)]
pub enum GuestCommand<'a> {
    /// Replaces the delivered secrets; the bundle goes on the input.
    InstallSecrets(&'a SecretBundle),
    /// Clones the repository and checks out the branch.
    Provision(&'a ProvisionSpec),
    /// Lists terminal sessions as JSON.
    Sessions,
    Close(&'a SessionName),
}

#[derive(serde::Serialize)]
struct InstallRequest<'a> {
    bundle: &'a SecretBundle,
}

impl GuestCommand<'_> {
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        match self {
            Self::InstallSecrets(_) => vec!["install-secrets".into()],
            Self::Provision(spec) => {
                let mut args = vec![
                    "provision".into(),
                    "--repo".into(),
                    spec.repo.to_string(),
                    "--branch".into(),
                    spec.branch.to_string(),
                ];
                if let Some(base) = &spec.base {
                    args.extend(["--base".into(), base.to_string()]);
                }
                args
            }
            Self::Sessions => vec!["sessions".into()],
            Self::Close(session) => vec!["close".into(), session.to_string()],
        }
    }

    /// What the command reads on its input, if anything.
    ///
    /// # Panics
    ///
    /// Never: a bundle has only string keys, so it always serializes.
    #[must_use]
    pub fn input(&self) -> Option<Vec<u8>> {
        match self {
            Self::InstallSecrets(bundle) => Some(
                serde_json::to_vec(&InstallRequest { bundle })
                    .expect("a bundle has only string keys, so it always serializes"),
            ),
            Self::Provision(_) | Self::Sessions | Self::Close(_) => None,
        }
    }
}

/// Attaches to a session, creating it on first attach. Runs on a terminal.
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

/// One entry of the guest's status file, as `iglu-status` writes it.
#[derive(Deserialize)]
struct RawStatus {
    state: AttentionState,
    #[serde(default)]
    summary: Option<Summary>,
    at: i64,
}

/// Parses the guest's status file. Untrusted: malformed entries are dropped
/// individually, and the whole file is capped.
#[must_use]
pub fn parse_statuses(bytes: &[u8]) -> Vec<SessionStatus> {
    const MAX_SESSIONS: usize = 64;
    let Ok(raw) = serde_json::from_slice::<HashMap<String, serde_json::Value>>(bytes) else {
        return Vec::new();
    };
    let mut statuses: Vec<SessionStatus> = raw
        .into_iter()
        .filter_map(|(session, value)| {
            let session: SessionName = session.parse().ok()?;
            let status: RawStatus = serde_json::from_value(value).ok()?;
            Some(SessionStatus {
                session,
                state: status.state,
                summary: status.summary.unwrap_or_else(|| Summary::sanitize("")),
                updated_at: Timestamp::from_unix_millis(status.at),
            })
        })
        .collect();
    statuses.sort_by(|a, b| a.session.cmp(&b.session));
    statuses.truncate(MAX_SESSIONS);
    statuses
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
            "t1": {"state": "waiting", "summary": "approve Bash?", "at": 5},
            "t2": {"state": "exploding", "at": 5},
            "../x": {"state": "done", "at": 1},
            "t3": {"state": "done", "at": 7}
        }"#;
        let parsed = parse_statuses(file);
        let names: Vec<_> = parsed.iter().map(|s| s.session.as_str()).collect();
        assert_eq!(names, ["t1", "t3"]);
        assert_eq!(parsed[0].summary.as_str(), "approve Bash?");
        assert!(parse_statuses(b"not json").is_empty());
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
