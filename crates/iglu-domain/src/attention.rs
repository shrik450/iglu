//! Which workspaces need the user. Guests report status per thread: one
//! conversation of the agent in a session, or the session itself for agents
//! that don't have conversations. The console shows each workspace's most
//! urgent thread.

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

/// One conversation in a session, as its agent identifies it, such as Claude
/// Code's session ID. Untrusted: a bounded word.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct ThreadKey(String);

impl ThreadKey {
    pub const MAX_LEN: usize = 64;

    /// The thread of a session whose agent doesn't name its conversations.
    ///
    /// # Panics
    ///
    /// Never: the name is a valid key.
    #[must_use]
    pub fn session() -> Self {
        "session".parse().expect("'session' is a thread key")
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ThreadKey {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty()
            || s.len() > Self::MAX_LEN
            || !s
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(ParseError::new(
                "thread key",
                "must be 1 to 64 of A-Z, a-z, 0-9, '-' and '_'",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for ThreadKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(ThreadKey);

/// The most threads one session keeps, and how many of them may have exited.
pub const MAX_THREADS: usize = 16;
pub const KEPT_EXITED: usize = 5;

/// Which statuses go first when there are too many: those waiting on the
/// person, then working ones, then the rest; newest first within each.
const fn rank(state: AttentionState) -> u8 {
    match state {
        AttentionState::Waiting => 0,
        AttentionState::Working => 1,
        AttentionState::Done => 2,
        AttentionState::Idle => 3,
        AttentionState::Exited => 4,
    }
}

/// Which of a session's threads to keep, given each one's state and when it
/// last changed: waiting and working ones first, then the rest, newest
/// first, with at most [`KEPT_EXITED`] exited and [`MAX_THREADS`] in all.
#[must_use]
pub fn keep<K: Clone>(threads: &[(K, AttentionState, Timestamp)]) -> Vec<K> {
    let mut newest: Vec<&(K, AttentionState, Timestamp)> = threads.iter().collect();
    newest.sort_by_key(|(_, state, at)| (rank(*state), std::cmp::Reverse(*at)));
    let mut exited = 0;
    newest
        .into_iter()
        .filter(|(_, state, _)| match state {
            AttentionState::Exited => {
                exited += 1;
                exited <= KEPT_EXITED
            }
            AttentionState::Working
            | AttentionState::Waiting
            | AttentionState::Done
            | AttentionState::Idle => true,
        })
        .take(MAX_THREADS)
        .map(|(key, _, _)| key.clone())
        .collect()
}

/// At most `limit` statuses, keeping the same ones [`keep`] would first.
#[must_use]
pub fn cap(mut statuses: Vec<SessionStatus>, limit: usize) -> Vec<SessionStatus> {
    statuses.sort_by_key(|s| (rank(s.state), std::cmp::Reverse(s.updated_at)));
    statuses.truncate(limit);
    statuses
}

/// The latest status one thread reported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub session: SessionName,
    pub thread: ThreadKey,
    /// What the thread was started to do: its first prompt, or empty.
    pub title: Summary,
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
            thread: ThreadKey::session(),
            title: Summary::sanitize(""),
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
    fn live_threads_stay_and_old_exited_ones_go() {
        let at = Timestamp::from_unix_millis;
        let mut threads: Vec<(u32, AttentionState, Timestamp)> = (0..8)
            .map(|n| (n, AttentionState::Exited, at(i64::from(n))))
            .collect();
        threads.push((100, AttentionState::Waiting, at(-5)));
        let kept = keep(&threads);
        assert_eq!(kept, [100, 7, 6, 5, 4, 3]);
        let many: Vec<(u32, AttentionState, Timestamp)> = (0..40)
            .map(|n| (n, AttentionState::Working, at(i64::from(n))))
            .collect();
        assert_eq!(keep(&many).len(), MAX_THREADS);
    }

    #[test]
    fn a_waiting_thread_outlasts_newer_finished_ones() {
        let at = Timestamp::from_unix_millis;
        let mut threads: Vec<(u32, AttentionState, Timestamp)> = (1..=20)
            .map(|n| (n, AttentionState::Done, at(i64::from(n))))
            .collect();
        threads.push((0, AttentionState::Waiting, at(0)));
        let kept = keep(&threads);
        assert_eq!(kept.len(), MAX_THREADS);
        assert_eq!(kept.first(), Some(&0));
        let statuses: Vec<SessionStatus> = threads
            .iter()
            .map(|(n, state, when)| SessionStatus {
                session: "claude".parse().expect("valid session"),
                thread: format!("t{n}").parse().expect("valid thread"),
                title: Summary::sanitize(""),
                state: *state,
                summary: Summary::sanitize(""),
                updated_at: *when,
            })
            .collect();
        let capped = cap(statuses, 4);
        assert_eq!(capped.len(), 4);
        assert_eq!(capped[0].state, AttentionState::Waiting);
    }

    #[test]
    fn thread_keys_are_bounded_words() {
        assert!(
            "6f2c3c7e-2f7b-4a8e-9c0a-1c9f4f2b1d11"
                .parse::<ThreadKey>()
                .is_ok()
        );
        assert!("".parse::<ThreadKey>().is_err());
        assert!("../x".parse::<ThreadKey>().is_err());
        assert!("x".repeat(65).parse::<ThreadKey>().is_err());
    }

    #[test]
    fn summaries_are_sanitized_not_rejected() {
        let summary = Summary::sanitize("approve \u{1b}[31mBash?\nsecond line");
        assert_eq!(summary.as_str(), "approve [31mBash?");
        let long = Summary::sanitize(&"x".repeat(500));
        assert_eq!(long.as_str().chars().count(), Summary::MAX_CHARS);
    }
}
