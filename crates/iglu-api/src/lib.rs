//! iglud's HTTP API: the requests it accepts and the views it returns.
//!
//! iglud serves these types and the CLI parses them, so both sides share one
//! definition, and deserializing a response is parsing it into domain types.
//! The console gets TypeScript declarations generated from them: with the
//! `ts` feature, the tests write one file per type (`just api-types`).

use iglu_domain::agent::Prompt;
use iglu_domain::attention::{AttentionState, Seen, Summary, ThreadKey};
use iglu_domain::auth::{DisplayName, Email};
use iglu_domain::capacity::{Bytes, Capacity};
use iglu_domain::column::{ColumnKind, ColumnLabel, ColumnSpec, ColumnState, ColumnWidth};
use iglu_domain::env::{BuiltImage, EnvName, EnvSource};
use iglu_domain::git::{GitState, Unsaved};
use iglu_domain::id::{EnvRevisionId, PrincipalId, ProjectId, RouteId, SecretId, WorkspaceId};
use iglu_domain::idle::IdleRule;
use iglu_domain::label::{AgentName, ProjectName, RouteName, WorkspaceName};
use iglu_domain::lifecycle::{DesiredState, Phase, Revision};
use iglu_domain::listener::Listener;
use iglu_domain::port::GuestPort;
use iglu_domain::project::{Opening, Origin, PreviewPorts};
use iglu_domain::repo::{BranchName, Checkout, RepoUrl};
use iglu_domain::secret::{SecretName, SecretTarget, SecretValue};
use iglu_domain::standing::Need;
use iglu_domain::terminal::SessionName;
use iglu_domain::time::Timestamp;
use iglu_proto::ErrorCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    pub boot: BootId,
}

/// A workspace as the console and CLI see it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WorkspaceView {
    pub id: WorkspaceId,
    pub name: WorkspaceName,
    pub project: ProjectId,
    /// What it cloned, when its project has a repository.
    pub checkout: Option<Checkout>,
    pub environment: EnvName,
    pub phase: Phase,
    pub desired: DesiredState,
    /// Sent back as `expected_revision` so a change applies only to what
    /// the caller saw.
    pub revision: Revision,
    pub condition: Option<Condition>,
    /// Waiting on the person: a thread needs an answer or review, or the
    /// workspace is in trouble. Decided by the core's standing rule.
    pub needs_you: Option<Need>,
    pub memory: Option<Bytes>,
    /// When the host last reported on the workspace.
    pub observed_at: Option<Timestamp>,
    /// The most urgent thread, if any has reported.
    pub attention: Option<AttentionView>,
    /// Every thread, most urgent first.
    pub threads: Vec<AttentionView>,
    /// The columns the workspace opens, in order.
    pub columns: Vec<ColumnSpec>,
    /// The agents its image can run, for agent columns.
    pub agents: Vec<AgentName>,
    pub routes: Vec<RouteView>,
    pub created_at: Timestamp,
}

/// What one terminal session last reported.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AttentionView {
    pub session: SessionName,
    /// One conversation of the session's agent, or `session` for the
    /// session itself.
    pub thread: ThreadKey,
    /// What the thread was started to do, or empty.
    pub title: Summary,
    pub state: AttentionState,
    pub summary: Summary,
    pub updated_at: Timestamp,
    pub seen: Seen,
}

/// Something listening in a workspace, and its preview if it has one.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ListenerView {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub listener: Listener,
    pub route: Option<RouteId>,
}

/// What's going on inside a running workspace now.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LiveView {
    pub listeners: Vec<ListenerView>,
    /// `None` without a repository.
    pub git: Option<GitState>,
    /// What deleting it would lose, decided from `git`.
    pub unsaved: Option<Unsaved>,
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

/// A project: what its workspaces clone and how they start.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ProjectView {
    pub id: ProjectId,
    pub name: ProjectName,
    pub origin: Origin,
    pub repo: Option<RepoUrl>,
    pub environment: EnvName,
    /// The columns its new workspaces open with.
    pub opening: Opening,
    /// The agent its new workspaces start, if any.
    pub agent: Option<AgentName>,
    /// Ports its new workspaces publish as previews.
    pub ports: PreviewPorts,
    pub idle: IdleRule,
    /// Sent back as `expected_revision` when changing it.
    pub revision: Revision,
    pub created_at: Timestamp,
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

/// A column with its session's state, from the workspace's host.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ColumnStatus {
    pub name: SessionName,
    pub state: ColumnState,
    /// Terminals attached right now.
    pub clients: u32,
}

/// Opens one more column in a running workspace.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AddColumn {
    pub kind: ColumnKind,
    /// Picked from the kind when absent: `shell`, `claude-2`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<SessionName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub width: Option<ColumnWidth>,
    /// The column to put it after; the end when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub after: Option<SessionName>,
}

/// What to call a column; `null` goes back to its session name.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LabelColumn {
    pub label: Option<ColumnLabel>,
}

/// A workspace's columns in their new order, with their widths. Names every
/// column once, no more and no fewer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PutLayout {
    pub columns: Vec<LayoutEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LayoutEntry {
    pub name: SessionName,
    pub width: ColumnWidth,
}

/// An environment build that has started; poll the environment for its outcome.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BuildStarted {
    pub revision: EnvRevisionId,
}

/// Everything the console shows, sent whole on every change. Workspaces come
/// in the core's standing order, most pressing first.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Snapshot {
    /// Which run of iglud sent it; see [`BootId`].
    pub boot: BootId,
    pub workspaces: Vec<WorkspaceView>,
    pub projects: Vec<ProjectView>,
    pub environments: Vec<EnvironmentView>,
}

/// Every error response's body, whatever refused the request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ErrorBody {
    pub error: ErrorKind,
    /// Safe to show the user: a lowercase phrase such as `a project with
    /// that name exists`.
    pub message: String,
    /// The input the error is about, when it's about one.
    pub field: Option<Field>,
}

/// What kind of refusal an error is. Each kind has one HTTP status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, rename_all = "snake_case")
)]
pub enum ErrorKind {
    /// 400: the request didn't parse, or asked for something impossible.
    BadRequest,
    /// 401: no session, or it ended.
    Unauthorized,
    /// 403
    Forbidden,
    /// 404: no such thing, or the caller may not know it exists.
    NotFound,
    /// 405: the path exists, but not with this method.
    MethodNotAllowed,
    /// 409: the request conflicts with what's there now.
    Conflict,
    /// 413: the request body is too large.
    TooLarge,
    /// 415: the request body isn't JSON.
    UnsupportedMediaType,
    /// 503: try again later.
    Unavailable,
    /// 500: iglud's fault; its log says more.
    Internal,
}

/// Which input an error is about: a path into the request body such as
/// `source`, `ports[1]` or `opening[0].kind`, or a path parameter's name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, type = "string"))]
#[serde(transparent)]
pub struct Field(String);

impl Field {
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which run of iglud is answering. It changes whenever iglud restarts, which
/// is how a console left open across an upgrade knows to reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, type = "string"))]
#[serde(transparent)]
pub struct BootId(Uuid);

impl BootId {
    /// Wraps a UUID; iglud picks a fresh one when it starts.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

// ---- requests ----

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CreateWorkspace {
    pub project: ProjectId,
    /// What to work on: it starts an agent with it, and names the workspace
    /// when no name is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub prompt: Option<Prompt>,
    /// The agent to start. Defaults to the opening's first agent column, or
    /// the environment's first agent when there's a prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub agent: Option<AgentName>,
    /// The branch to work on. Defaults to the workspace name. Only for
    /// projects with a repository.
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
pub struct RenameWorkspace {
    pub name: WorkspaceName,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CreateProject {
    /// Named after the repository when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<ProjectName>,
    /// Without one, workspaces start in an empty home directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub repo: Option<RepoUrl>,
    pub environment: EnvName,
}

/// A project's settings, whole. Applies only if it's still at
/// `expected_revision`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ChangeProject {
    pub expected_revision: Revision,
    pub name: ProjectName,
    /// Required, though it can be null: leaving it out mustn't clear it.
    #[serde(deserialize_with = "Option::deserialize")]
    pub repo: Option<RepoUrl>,
    pub environment: EnvName,
    pub opening: Opening,
    #[serde(deserialize_with = "Option::deserialize")]
    pub agent: Option<AgentName>,
    pub ports: PreviewPorts,
    pub idle: IdleRule,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_request_fields_can_be_left_out() {
        let request: CreateWorkspace =
            serde_json::from_str(r#"{"project":"6f2c3c7e-2f7b-4a8e-9c0a-1c9f4f2b1d11"}"#)
                .expect("the optional fields default");
        assert!(request.branch.is_none() && request.base.is_none() && request.name.is_none());
        let back = serde_json::to_string(&request).expect("serializes");
        assert!(!back.contains("branch"), "{back}");
    }

    #[test]
    fn a_whole_project_change_names_every_field() {
        let whole = r#"{"expected_revision":1,"name":"app","repo":null,"environment":"default","opening":[],"agent":null,"ports":[],"idle":{"kind":"default"}}"#;
        assert!(serde_json::from_str::<ChangeProject>(whole).is_ok());
        let without_repo = whole.replace(r#""repo":null,"#, "");
        assert!(serde_json::from_str::<ChangeProject>(&without_repo).is_err());
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
