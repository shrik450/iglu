//! iglud's HTTP API: the requests it accepts and the views it returns.
//!
//! iglud serves these types and the CLI parses them, so both sides share one
//! definition, and deserializing a response is parsing it into domain types.
//! The console gets TypeScript declarations generated from them: with the
//! `ts` feature, the tests write one file per type (`just api-types`).

use iglu_domain::attention::{AttentionState, Seen, Summary};
use iglu_domain::auth::{DisplayName, Email};
use iglu_domain::capacity::{Bytes, Capacity};
use iglu_domain::env::{BuiltImage, EnvName, EnvSource};
use iglu_domain::id::{EnvRevisionId, PrincipalId, RouteId, SecretId, WorkspaceId};
use iglu_domain::label::{RouteName, WorkspaceName};
use iglu_domain::lifecycle::{DesiredState, Phase, Revision};
use iglu_domain::port::GuestPort;
use iglu_domain::repo::{BranchName, RepoUrl};
use iglu_domain::secret::{SecretName, SecretTarget, SecretValue};
use iglu_domain::terminal::SessionName;
use iglu_domain::time::Timestamp;
use iglu_proto::ErrorCode;
use serde::{Deserialize, Serialize};

/// The signed-in person, and what the console needs to act for them.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Me {
    pub id: PrincipalId,
    pub name: Option<DisplayName>,
    pub email: Option<Email>,
    /// Sent back in `x-csrf-token` on every mutating console request.
    pub csrf_token: String,
    pub preview_domain: String,
}

/// A workspace as the console and CLI see it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WorkspaceView {
    pub id: WorkspaceId,
    pub name: WorkspaceName,
    pub repo: RepoUrl,
    pub branch: BranchName,
    pub environment: EnvName,
    pub phase: Phase,
    pub desired: DesiredState,
    /// Sent back as `expected_revision` so a change applies only to what
    /// the caller saw.
    pub revision: Revision,
    pub condition: Option<Condition>,
    pub memory: Option<Bytes>,
    /// When the host last reported on the workspace.
    pub observed_at: Option<Timestamp>,
    /// The most urgent session, if any has reported.
    pub attention: Option<AttentionView>,
    pub sessions: Vec<AttentionView>,
    pub routes: Vec<RouteView>,
    pub created_at: Timestamp,
}

/// What one terminal session last reported.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AttentionView {
    pub session: SessionName,
    pub state: AttentionState,
    pub summary: Summary,
    pub updated_at: Timestamp,
    pub seen: Seen,
}

/// A published port.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RouteView {
    pub id: RouteId,
    pub name: RouteName,
    pub port: GuestPort,
    pub url: String,
}

/// Something about a workspace the owner should know, beyond its phase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, tag = "kind", rename_all = "snake_case")
)]
pub enum Condition {
    /// The last step failed; the reconciler retries with backoff.
    Error {
        code: ErrorCode,
        message: String,
        at: Timestamp,
    },
    /// Waiting for the host to have room.
    Capacity { available: Bytes, needed: Bytes },
    /// Incus reports the instance as broken; stop or delete it.
    RuntimeFailed,
    /// The host hasn't answered since `last_seen`.
    HostOffline { last_seen: Option<Timestamp> },
}

impl Condition {
    #[must_use]
    pub const fn from_capacity(capacity: Capacity) -> Option<Self> {
        match capacity {
            Capacity::Fits => None,
            Capacity::Short { available, needed } => Some(Self::Capacity { available, needed }),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct EnvironmentView {
    pub name: EnvName,
    pub source: EnvSource,
    /// The newest build, if there has been one.
    pub latest: Option<RevisionView>,
}

/// One build of an environment.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RevisionView {
    pub id: EnvRevisionId,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub status: RevisionStatus,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, tag = "status", rename_all = "snake_case")
)]
pub enum RevisionStatus {
    Building,
    Ready { image: BuiltImage },
    Failed { log_tail: String },
}

/// A stored secret: where it goes, never its value.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SecretView {
    pub id: SecretId,
    pub name: SecretName,
    pub target: SecretTarget,
    pub updated_at: Timestamp,
}

/// One entry in a workspace's history.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ActivityEntry {
    pub kind: String,
    pub detail: String,
    pub at: Timestamp,
}

/// A terminal session in a running workspace.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TerminalView {
    pub name: SessionName,
    /// Browsers attached right now.
    pub clients: u32,
}

/// The session a new-terminal request created.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct NewTerminal {
    pub name: SessionName,
}

/// An environment build that has started; poll the environment for its outcome.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BuildStarted {
    pub revision: EnvRevisionId,
}

/// Every error response's body.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ErrorBody {
    /// A stable, machine-readable code such as `not_found`.
    pub error: String,
    /// Safe to show the user.
    pub message: String,
}

// ---- requests ----

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CreateWorkspace {
    pub environment: EnvName,
    pub repo: RepoUrl,
    /// The branch to work on. Defaults to the workspace name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub branch: Option<BranchName>,
    /// Where a new branch starts. Defaults to the remote's default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub base: Option<BranchName>,
    /// Generated when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<WorkspaceName>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SetDesiredState {
    pub state: DesiredState,
    /// Refuses the change if the workspace moved on since this revision, so
    /// it applies only to what the caller saw.
    pub expected_revision: Revision,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PublishPort {
    pub port: GuestPort,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CreateEnvironment {
    pub name: EnvName,
    pub source: EnvSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PutSecret {
    pub target: SecretTarget,
    pub value: SecretValue,
}

/// The CLI's half of the loopback sign-in: the code the console handed its
/// listener, and the PKCE verifier only the CLI knows.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliTokenRequest {
    pub code: String,
    pub verifier: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CliToken {
    pub token: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_request_fields_can_be_left_out() {
        let request: CreateWorkspace = serde_json::from_str(
            r#"{"environment":"default","repo":"https://github.com/acme/app.git"}"#,
        )
        .expect("the optional fields default");
        assert!(request.branch.is_none() && request.base.is_none() && request.name.is_none());
        let back = serde_json::to_string(&request).expect("serializes");
        assert!(!back.contains("branch"), "{back}");
    }

    #[test]
    fn a_revision_flattens_its_status() {
        let json = r#"{"id":"6f2c3c7e-2f7b-4a8e-9c0a-1c9f4f2b1d11","status":"failed","log_tail":"boom","created_at":5}"#;
        let revision: RevisionView = serde_json::from_str(json).expect("parses");
        assert!(
            matches!(revision.status, RevisionStatus::Failed { ref log_tail } if log_tail == "boom")
        );
    }

    #[test]
    fn unknown_request_fields_are_refused() {
        let result: Result<PublishPort, _> = serde_json::from_str(r#"{"port":3000,"host":"x"}"#);
        assert!(result.is_err());
    }
}
