//! A workspace's Git state, as its guest reports it.

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::parse::is_printable;

/// A branch name as Git shows it. Not a [`crate::repo::BranchName`]: Git
/// allows names iglu's parser wouldn't, and this is only ever shown.
/// Untrusted: printable and bounded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct ObservedBranch(String);

/// Off the wire, only a name that's already clean: deserializing parses.
impl TryFrom<String> for ObservedBranch {
    type Error = ParseError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match Self::sanitize(&s) {
            Some(clean) if clean.0 == s => Ok(clean),
            Some(_) | None => Err(ParseError::new(
                "branch",
                "must be printable, non-empty and at most 100 characters",
            )),
        }
    }
}

impl From<ObservedBranch> for String {
    fn from(branch: ObservedBranch) -> Self {
        branch.0
    }
}

impl ObservedBranch {
    pub const MAX_CHARS: usize = 100;

    /// `None` when nothing printable is left.
    #[must_use]
    pub fn sanitize(s: &str) -> Option<Self> {
        let clean: String = s
            .trim()
            .chars()
            .filter(|c| is_printable(*c))
            .take(Self::MAX_CHARS)
            .collect();
        (!clean.is_empty()).then_some(Self(clean))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where a checkout stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GitState {
    /// `None` when the checkout is detached.
    pub branch: Option<ObservedBranch>,
    pub upstream: Option<ObservedBranch>,
    /// Commits the upstream doesn't have yet, and the other way around.
    pub ahead: u32,
    pub behind: u32,
    /// Tracked files with changes, staged or not.
    pub changed: u32,
    pub untracked: u32,
    pub conflicted: u32,
}

/// Work in a checkout that deleting the workspace would lose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Unsaved {
    /// Files changed, untracked or conflicted.
    pub uncommitted: u32,
    pub unpushed: Unpushed,
}

/// Commits only this checkout has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(tag = "kind", rename_all = "snake_case")
)]
pub enum Unpushed {
    None,
    /// Commits the upstream doesn't have.
    Commits {
        count: u32,
    },
    /// The branch has never been pushed, so nothing it has is anywhere else.
    NoUpstream,
}

/// What deleting the workspace would lose, or `None` when everything is
/// committed and pushed. A detached checkout counts as pushed: its commits
/// came from somewhere.
#[must_use]
pub fn unsaved(git: &GitState) -> Option<Unsaved> {
    let uncommitted = git
        .changed
        .saturating_add(git.untracked)
        .saturating_add(git.conflicted);
    let unpushed = match (&git.branch, &git.upstream) {
        (Some(_), None) => Unpushed::NoUpstream,
        (Some(_) | None, _) if git.ahead > 0 => Unpushed::Commits { count: git.ahead },
        (Some(_) | None, _) => Unpushed::None,
    };
    (uncommitted > 0 || unpushed != Unpushed::None).then_some(Unsaved {
        uncommitted,
        unpushed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(branch: Option<&str>, upstream: Option<&str>, ahead: u32, changed: u32) -> GitState {
        GitState {
            branch: branch.and_then(ObservedBranch::sanitize),
            upstream: upstream.and_then(ObservedBranch::sanitize),
            ahead,
            behind: 0,
            changed,
            untracked: 0,
            conflicted: 0,
        }
    }

    #[test]
    fn only_clean_branches_come_off_the_wire() {
        assert!(serde_json::from_str::<ObservedBranch>(r#""fix-login""#).is_ok());
        assert!(serde_json::from_str::<ObservedBranch>(r#""""#).is_err());
        assert!(serde_json::from_str::<ObservedBranch>(r#""main[31m""#).is_err());
    }

    #[test]
    fn clean_and_pushed_loses_nothing() {
        assert_eq!(unsaved(&state(Some("fix"), Some("origin/fix"), 0, 0)), None);
        assert_eq!(unsaved(&state(None, None, 0, 0)), None, "detached");
    }

    #[test]
    fn changes_unpushed_commits_and_unpushed_branches_are_unsaved() {
        assert_eq!(
            unsaved(&state(Some("fix"), Some("origin/fix"), 2, 3)),
            Some(Unsaved {
                uncommitted: 3,
                unpushed: Unpushed::Commits { count: 2 }
            })
        );
        assert_eq!(
            unsaved(&state(Some("fix"), None, 0, 0)),
            Some(Unsaved {
                uncommitted: 0,
                unpushed: Unpushed::NoUpstream
            })
        );
    }

    #[test]
    fn branches_are_printable_and_bounded() {
        assert_eq!(
            ObservedBranch::sanitize("feat/x\u{1b}[2J").map(|b| b.0),
            Some("feat/x[2J".into())
        );
        assert_eq!(ObservedBranch::sanitize("  "), None);
        assert_eq!(
            ObservedBranch::sanitize(&"b".repeat(300)).map(|b| b.0.len()),
            Some(ObservedBranch::MAX_CHARS)
        );
    }
}
