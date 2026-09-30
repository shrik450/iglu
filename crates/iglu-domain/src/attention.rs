//! Which workspaces need the user. Guests report per-session status; the
//! console shows each workspace's most urgent one.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::parse::{is_printable, text_type};
use crate::terminal::SessionName;
use crate::time::Timestamp;

/// What an agent or shell in a session is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "snake_case"))]
pub enum AttentionState {
    Working,
    /// Blocked on the user: a permission prompt or a question.
    Waiting,
    /// Finished a turn and wants review.
    Done,
    Idle,
    Exited,
}

impl AttentionState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Idle => "idle",
            Self::Exited => "exited",
        }
    }
}

impl FromStr for AttentionState {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "working" => Ok(Self::Working),
            "waiting" => Ok(Self::Waiting),
            "done" => Ok(Self::Done),
            "idle" => Ok(Self::Idle),
            "exited" => Ok(Self::Exited),
            _ => Err(ParseError::new(
                "attention state",
                "expected working, waiting, done, idle or exited",
            )),
        }
    }
}

impl fmt::Display for AttentionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A one-line, guest-supplied description. Untrusted: bounded and free of
/// control characters, so it can't smuggle terminal escapes into the UI.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct Summary(String);

impl Summary {
    pub const MAX_CHARS: usize = 200;

    /// Takes the first line, drops control characters and truncates, rather
    /// than rejecting: a summary is advisory and shouldn't fail a status update.
    #[must_use]
    pub fn sanitize(s: &str) -> Self {
        let line = s.lines().next().unwrap_or_default();
        let clean: String = line
            .chars()
            .filter(|c| is_printable(*c))
            .take(Self::MAX_CHARS)
            .collect();
        Self(clean.trim().to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Summary {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::sanitize(s))
    }
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(Summary);

/// The latest status one session reported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub session: SessionName,
    pub state: AttentionState,
    pub summary: Summary,
    pub updated_at: Timestamp,
}

/// Whether the user has looked at a status since it last changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "snake_case"))]
pub enum Seen {
    Seen,
    Unseen,
}

/// How much a status wants the user, least to most.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    Exited,
    Idle,
    SeenDone,
    Working,
    UnseenDone,
    Waiting,
}

#[must_use]
pub const fn urgency(state: AttentionState, seen: Seen) -> Urgency {
    match (state, seen) {
        (AttentionState::Waiting, _) => Urgency::Waiting,
        (AttentionState::Done, Seen::Unseen) => Urgency::UnseenDone,
        (AttentionState::Working, _) => Urgency::Working,
        (AttentionState::Done, Seen::Seen) => Urgency::SeenDone,
        (AttentionState::Idle, _) => Urgency::Idle,
        (AttentionState::Exited, _) => Urgency::Exited,
    }
}

/// The status that represents a whole workspace: its most urgent session.
pub fn most_urgent<'a>(
    sessions: impl IntoIterator<Item = (&'a SessionStatus, Seen)>,
) -> Option<(&'a SessionStatus, Seen)> {
    sessions
        .into_iter()
        .max_by_key(|(status, seen)| (urgency(status.state, *seen), status.updated_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(session: &str, state: AttentionState, at: i64) -> SessionStatus {
        SessionStatus {
            session: session.parse().expect("valid session"),
            state,
            summary: Summary::sanitize(""),
            updated_at: Timestamp::from_unix_millis(at),
        }
    }

    #[test]
    fn waiting_beats_everything() {
        let a = status("t1", AttentionState::Working, 5);
        let b = status("t2", AttentionState::Waiting, 1);
        let c = status("t3", AttentionState::Done, 9);
        let top = most_urgent([(&a, Seen::Unseen), (&b, Seen::Seen), (&c, Seen::Unseen)]);
        assert_eq!(top.map(|(s, _)| s.session.as_str()), Some("t2"));
    }

    #[test]
    fn unseen_done_beats_working_but_seen_done_does_not() {
        let working = status("t1", AttentionState::Working, 1);
        let done = status("t2", AttentionState::Done, 1);
        let top = most_urgent([(&working, Seen::Seen), (&done, Seen::Unseen)]);
        assert_eq!(top.map(|(s, _)| s.session.as_str()), Some("t2"));
        let top = most_urgent([(&working, Seen::Seen), (&done, Seen::Seen)]);
        assert_eq!(top.map(|(s, _)| s.session.as_str()), Some("t1"));
    }

    #[test]
    fn ties_go_to_the_newest() {
        let old = status("t1", AttentionState::Working, 1);
        let new = status("t2", AttentionState::Working, 2);
        let top = most_urgent([(&old, Seen::Seen), (&new, Seen::Seen)]);
        assert_eq!(top.map(|(s, _)| s.session.as_str()), Some("t2"));
    }

    #[test]
    fn summaries_are_sanitized_not_rejected() {
        let summary = Summary::sanitize("approve \u{1b}[31mBash?\nsecond line");
        assert_eq!(summary.as_str(), "approve [31mBash?");
        let long = Summary::sanitize(&"x".repeat(500));
        assert_eq!(long.as_str().chars().count(), Summary::MAX_CHARS);
    }
}
