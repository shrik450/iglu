//! What happened to a workspace, as its history tells it: each entry is one
//! of these kinds, with a detail whose meaning the kind gives.

use serde::{Deserialize, Serialize};

use crate::parse::serde_text;

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
    /// It was upgraded in place to its new build's system.
    Updated,
}

serde_text!(ActivityKind, "activity kind", "not a kind iglu records");
