//! What happened to a workspace, as its history tells it: each entry is one
//! of these kinds, with a detail whose meaning the kind gives.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;

/// The kinds of thing a workspace's history records. Stored and sent by
/// name; the console says each in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "kebab-case"))]
pub enum ActivityKind {
    /// Asked for; the detail says where it works.
    Requested,
    /// Asked to run, freeze or stop; the detail says which.
    StateSet,
    Created,
    Started,
    SecretsDelivered,
    Provisioned,
    ColumnsOpened,
    Frozen,
    Thawed,
    Stopped,
    /// Its instance was deleted from its host.
    Removed,
    /// Gone for good.
    Deleted,
    /// A step failed; the detail says which, and why.
    Error,
    Renamed,
    /// Text typed into a column for someone; the detail says which.
    Typed,
    ThawedOnOpen,
    IdleFroze,
    IdleStopped,
    SecretUpdated,
    /// A port published; the detail says which, under what name.
    Published,
    Unpublished,
    /// A file kept in the workspace; the detail is its path.
    FileKept,
    /// Its host changed it; the detail says why.
    Adopted,
    AccessChanged,
    /// The detail names the column.
    ColumnAdded,
    ColumnRestarted,
    ColumnClosed,
    /// Asked to move to its environment's newest build; the detail names it.
    UpdateAsked,
    /// Its root disk was rebuilt from its new image, its home kept.
    Rebuilt,
}

impl ActivityKind {
    pub const ALL: [Self; 29] = [
        Self::Requested,
        Self::StateSet,
        Self::Created,
        Self::Started,
        Self::SecretsDelivered,
        Self::Provisioned,
        Self::ColumnsOpened,
        Self::Frozen,
        Self::Thawed,
        Self::Stopped,
        Self::Removed,
        Self::Deleted,
        Self::Error,
        Self::Renamed,
        Self::Typed,
        Self::ThawedOnOpen,
        Self::IdleFroze,
        Self::IdleStopped,
        Self::SecretUpdated,
        Self::Published,
        Self::Unpublished,
        Self::FileKept,
        Self::Adopted,
        Self::AccessChanged,
        Self::ColumnAdded,
        Self::ColumnRestarted,
        Self::ColumnClosed,
        Self::UpdateAsked,
        Self::Rebuilt,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::StateSet => "state-set",
            Self::Created => "created",
            Self::Started => "started",
            Self::SecretsDelivered => "secrets-delivered",
            Self::Provisioned => "provisioned",
            Self::ColumnsOpened => "columns-opened",
            Self::Frozen => "frozen",
            Self::Thawed => "thawed",
            Self::Stopped => "stopped",
            Self::Removed => "removed",
            Self::Deleted => "deleted",
            Self::Error => "error",
            Self::Renamed => "renamed",
            Self::Typed => "typed",
            Self::ThawedOnOpen => "thawed-on-open",
            Self::IdleFroze => "idle-froze",
            Self::IdleStopped => "idle-stopped",
            Self::SecretUpdated => "secret-updated",
            Self::Published => "published",
            Self::Unpublished => "unpublished",
            Self::FileKept => "file-kept",
            Self::Adopted => "adopted",
            Self::AccessChanged => "access-changed",
            Self::ColumnAdded => "column-added",
            Self::ColumnRestarted => "column-restarted",
            Self::ColumnClosed => "column-closed",
            Self::UpdateAsked => "update-asked",
            Self::Rebuilt => "rebuilt",
        }
    }
}

impl FromStr for ActivityKind {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == s)
            .ok_or(ParseError::new("activity kind", "not a kind iglu records"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_reads_back_as_itself_by_the_name_it_is_sent_as() {
        for kind in ActivityKind::ALL {
            assert_eq!(kind.as_str().parse::<ActivityKind>(), Ok(kind));
            assert_eq!(
                serde_json::to_value(kind).expect("serializes"),
                kind.as_str()
            );
        }
        assert!("exited".parse::<ActivityKind>().is_err());
    }
}
