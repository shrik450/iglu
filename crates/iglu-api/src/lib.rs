//! iglud's HTTP API: the requests it accepts and the views it returns.
//!
//! iglud serves these types and the CLI parses them, so both sides share one
//! definition, and deserializing a response is parsing it into domain types.
//! The console gets TypeScript declarations generated from them: with the
//! `ts` feature, the tests write one file per type (`just api-types`).

pub mod browser;

use iglu_domain::attention::{AttentionState, Seen, Summary, ThreadKey};
use iglu_domain::auth::{DisplayName, Email, Permission};
use iglu_domain::capacity::{Bytes, Capacity};
use iglu_domain::column::{ColumnKind, ColumnLabel, ColumnSpec, ColumnState, ColumnWidth};
use iglu_domain::env::{BuiltImage, EnvName, EnvSource, GuestPath};
use iglu_domain::git::{GitState, Unsaved};
use iglu_domain::id::{EnvRevisionId, PrincipalId, ProjectId, RouteId, SecretId, WorkspaceId};
use iglu_domain::idle::IdleRule;
use iglu_domain::label::{ProjectName, RouteName, WorkspaceName};
use iglu_domain::lifecycle::{AdoptReason, DesiredState, Effect, Phase, Revision, TransitionError};
use iglu_domain::listener::Listener;
use iglu_domain::names::WorkspaceLabel;
use iglu_domain::port::GuestPort;
use iglu_domain::preferences::Preferences;
use iglu_domain::project::{Opening, Origin, PreviewPorts};
use iglu_domain::repo::{BranchName, Checkout, RepoUrl};
use iglu_domain::secret::{BundleError, SecretName, SecretTarget, SecretValue};
use iglu_domain::standing::Need;
use iglu_domain::terminal::{SessionName, TerminalInput};
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
    pub preview_domain: String,
    pub boot: BootId,
}

/// A workspace as the console and CLI see it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WorkspaceView {
    pub id: WorkspaceId,
    pub name: WorkspaceName,
    /// What the person calls it, shown in place of its name.
    pub label: Option<WorkspaceLabel>,
    pub project: ProjectId,
    /// What it cloned, when its project has a repository.
    pub checkout: Option<Checkout>,
    pub environment: EnvName,
    pub phase: Phase,
    pub desired: DesiredState,
    /// Increases whenever its desired state changes.
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
    /// Whether its image has a browser, for a browser column.
    pub browser: bool,
    /// Whether its environment's newest build is of another image than the
    /// one it's on, which updating upgrades it to in place.
    pub newer_build: bool,
    pub routes: Vec<RouteView>,
    /// What it may do from inside, through the `iglu` command, to itself
    /// and to its owner's other workspaces.
    pub access: Vec<AccessGrant>,
    pub created_at: Timestamp,
}

/// What a workspace may do to one workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AccessGrant {
    pub workspace: WorkspaceId,
    pub permissions: Vec<Permission>,
}

/// Everything a workspace may do, replacing what it could before.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PutAccess {
    pub grants: Vec<AccessGrant>,
}

/// Who's asking: a person, or a workspace through its channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, tag = "kind"))]
pub enum Identity {
    Person {
        id: PrincipalId,
        name: Option<DisplayName>,
        email: Option<Email>,
    },
    Workspace {
        id: WorkspaceId,
        name: WorkspaceName,
    },
}

/// What a column's terminal shows: its last lines, history included, as
/// plain text. Not a log: what a program redrew or cleared is gone.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ColumnOutput {
    pub text: String,
    /// Whether there was more than came back.
    pub truncated: bool,
}

/// Where a pasted file was kept in the workspace: the path to paste.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct KeptFile {
    pub path: GuestPath,
}

/// What to type into a column's terminal: text, exactly as given, then
/// Enter if asked. Enter goes on its own, a moment after the text, so a
/// program that tells typing from pasting by how fast it comes takes it as
/// a key.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SendInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub text: Option<TerminalInput>,
    #[serde(default)]
    pub enter: bool,
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
#[serde(tag = "kind")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, tag = "kind"))]
pub enum Condition {
    /// The last step failed; the reconciler retries with backoff.
    Error {
        step: Effect,
        failure: Failure,
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

impl std::fmt::Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error { step, failure, .. } => write!(f, "{}: {failure}", doing(*step)),
            Self::Capacity { available, needed } => write!(
                f,
                "waiting for the host to have room: {needed} needed, {available} free"
            ),
            Self::RuntimeFailed => {
                f.write_str("the runtime reports the workspace as broken; stop or delete it")
            }
            Self::HostOffline { .. } => f.write_str("the host isn't answering"),
        }
    }
}

/// Why a step on a workspace failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, tag = "kind"))]
pub enum Failure {
    /// The host or the guest failed. `detail` is in its own words, such as
    /// `git clone`'s, so it's shown as it is, never translated.
    Host { code: ErrorCode, detail: String },
    /// Two of the owner's secrets go to the same place.
    SecretsClash { clash: BundleError },
    /// iglud's fault; its log says more.
    Internal,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Host { detail, .. } => f.write_str(detail),
            Self::SecretsClash { clash } => write!(f, "{clash}"),
            Self::Internal => f.write_str("iglu failed; its log says more"),
        }
    }
}

/// A step, as a phrase that follows "while": `creating it`.
const fn doing(step: Effect) -> &'static str {
    match step {
        Effect::Create => "creating it",
        Effect::Start => "starting it",
        Effect::DeliverSecrets => "delivering its secrets",
        Effect::Provision => "setting it up",
        Effect::OpenColumns => "opening its columns",
        Effect::Freeze => "freezing it",
        Effect::Thaw => "thawing it",
        Effect::Stop => "stopping it",
        Effect::Delete => "removing it from its host",
        Effect::Upgrade => "upgrading its system to the newest build",
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
#[serde(tag = "status")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, tag = "status"))]
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
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub what: Activity,
    pub at: Timestamp,
    /// The workspace it was done from, when a workspace did it.
    pub via: Option<WorkspaceName>,
}

/// What a workspace's history records, with what each entry needs to be
/// said: stored and sent as data, so the console says each in words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, tag = "kind"))]
pub enum Activity {
    /// Asked for, in `project`, working on `checkout` when it has a repository.
    Requested {
        project: ProjectName,
        checkout: Option<Checkout>,
    },
    /// Asked to run, freeze, stop or be deleted.
    StateSet {
        state: DesiredState,
    },
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
    /// A step failed.
    Error {
        step: Effect,
        failure: Failure,
    },
    Renamed {
        from: WorkspaceName,
        to: WorkspaceName,
    },
    /// What it's called changed; `None` went back to its name.
    Relabeled {
        to: Option<WorkspaceLabel>,
    },
    /// Text typed into a column for someone.
    Typed {
        column: SessionName,
    },
    ThawedOnOpen,
    IdleFroze,
    IdleStopped,
    SecretUpdated {
        secret: SecretName,
    },
    Published {
        port: GuestPort,
        route: RouteName,
    },
    Unpublished {
        port: GuestPort,
        route: RouteName,
    },
    /// A file kept in the workspace.
    FileKept {
        path: GuestPath,
    },
    /// Its host changed it.
    Adopted {
        reason: AdoptReason,
    },
    AccessChanged,
    ColumnAdded {
        column: SessionName,
    },
    ColumnRestarted {
        column: SessionName,
    },
    ColumnClosed {
        column: SessionName,
    },
    /// Asked to move to its environment's newest build.
    UpdateAsked {
        environment: EnvName,
    },
    /// It was upgraded in place to its new build's system.
    Updated,
}

impl std::fmt::Display for Activity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Requested {
                checkout: Some(checkout),
                ..
            } => write!(f, "requested, on {} of {}", checkout.branch, checkout.repo),
            Self::Requested {
                project,
                checkout: None,
            } => write!(f, "requested, in {project}"),
            Self::StateSet { state } => write!(f, "set to {state}"),
            Self::Created => f.write_str("created"),
            Self::Started => f.write_str("started"),
            Self::SecretsDelivered => f.write_str("secrets delivered"),
            Self::Provisioned => f.write_str("set up"),
            Self::ColumnsOpened => f.write_str("columns opened"),
            Self::Frozen => f.write_str("frozen"),
            Self::Thawed => f.write_str("thawed"),
            Self::Stopped => f.write_str("stopped"),
            Self::Removed => f.write_str("removed from its host"),
            Self::Deleted => f.write_str("deleted"),
            Self::Error { step, failure } => write!(f, "failed {}: {failure}", doing(*step)),
            Self::Renamed { from, to } => write!(f, "renamed from {from} to {to}"),
            Self::Relabeled { to: Some(label) } => write!(f, "called {label}"),
            Self::Relabeled { to: None } => f.write_str("called by its name"),
            Self::Typed { column } => write!(f, "typed into {column}"),
            Self::ThawedOnOpen => f.write_str("thawed when opened"),
            Self::IdleFroze => f.write_str("frozen when idle"),
            Self::IdleStopped => f.write_str("stopped when idle"),
            Self::SecretUpdated { secret } => write!(f, "secret {secret} updated"),
            Self::Published { port, route } => write!(f, "port {port} published as {route}"),
            Self::Unpublished { port, route } => {
                write!(f, "port {port} no longer published as {route}")
            }
            Self::FileKept { path } => write!(f, "kept {path}"),
            Self::Adopted {
                reason: AdoptReason::FrozenStateLost,
            } => f.write_str("stopped: the host restarted, which lost its frozen state"),
            Self::AccessChanged => f.write_str("access changed"),
            Self::ColumnAdded { column } => write!(f, "column {column} added"),
            Self::ColumnRestarted { column } => write!(f, "column {column} restarted"),
            Self::ColumnClosed { column } => write!(f, "column {column} closed"),
            Self::UpdateAsked { environment } => {
                write!(f, "asked to update to {environment}'s newest build")
            }
            Self::Updated => f.write_str("updated"),
        }
    }
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
    /// Picked from the kind when absent: `shell`, `browser-2`.
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
    /// The person's own, so a change in one browser reaches the others.
    pub preferences: Preferences,
}

/// Every error response's body, whatever refused the request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ErrorBody {
    pub reason: Reason,
    /// The input the error is about, when it's about one.
    pub field: Option<Field>,
}

/// Why iglu refused a request. It's data, not prose: the console says each
/// in words, and `Display` says it in English for the CLI and iglud's own
/// pages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, tag = "kind"))]
pub enum Reason {
    /// An input didn't parse, or isn't there; the error's field says which
    /// when it's known.
    Invalid,
    /// The body isn't JSON at all.
    NotJson,
    /// The body couldn't be read, or took too long to arrive.
    Unreadable,
    /// The path takes only a WebSocket.
    NotWebSocket,
    /// The `Idempotency-Key` header isn't 1-200 characters.
    IdempotencyKeyInvalid,
    /// A project needs a name or a repository to be named after.
    ProjectUnnamed,
    /// Neither text to type nor Enter to press.
    NothingToType,
    /// A grant names a workspace the owner doesn't have.
    NoWorkspaceToGrant {
        workspace: WorkspaceId,
    },
    /// The workspace browser's debugging or display port: a preview of either
    /// would let anyone with the link drive the browser.
    BrowserPort {
        port: GuestPort,
    },
    /// The identity provider sent the browser back without a code.
    SignInIncomplete,
    /// The sign-in was finished in another browser than started it.
    OtherBrowser,
    SignInExpired,
    CliSignInMalformed,
    /// A preview's sign-in was asked to return somewhere not a preview.
    BadReturnAddress,

    /// No session, or it ended.
    Unauthorized,

    AccountDisabled,
    /// The workspace asking hasn't been granted this.
    NotGranted,
    /// Only a person can do this, never a workspace.
    PersonOnly,
    /// A workspace can't do this from inside; its owner can.
    NotFromWorkspace,
    /// A request from another site, which only a top-level navigation may be.
    CrossOrigin,
    SignInFailed,
    /// The identity provider refused, with its own error code.
    ProviderRefused {
        error: String,
    },
    /// The sign-in policy doesn't admit the account.
    AccountNotAllowed,

    /// No such thing, or the caller may not know it exists.
    NotFound,
    /// The path exists, but not with this method.
    MethodNotAllowed,

    /// The workspace has no running instance to do it in.
    NotRunning,
    /// The `Idempotency-Key` was used for a different request.
    IdempotencyKeyReused,
    NoSuchProject,
    NoEnvironment {
        name: EnvName,
    },
    /// The environment hasn't finished a build yet.
    NotBuilt {
        environment: EnvName,
    },
    /// A branch was asked for in a project without a repository.
    NoRepository,
    WorkspaceNameTaken,
    ProjectNameTaken,
    EnvironmentNameTaken,
    BeingDeleted,
    FreezeNeedsRunning,
    NoBuildToUpdateTo,
    /// The newest build runs as another account, whose files would be
    /// someone else's; the workspace has to be recreated to use it.
    BuildForAnotherAccount,
    /// Its image has no browser: the environment needs `iglu.browser.enable`.
    NoBrowserInImage,
    BrowserColumnExists {
        column: SessionName,
    },
    ColumnNameTaken {
        name: SessionName,
    },
    /// A layout has to name every column once.
    LayoutMismatch,
    /// Another secret already goes where this one would.
    SecretTargetTaken {
        by: SecretName,
    },
    /// The project changed since the caller loaded it.
    ProjectChanged,
    BuiltinProject,
    ProjectInUse,

    TooLarge,
    JsonExpected,
    FormExpected,

    ProviderUnreachable,
    NoHost,
    /// The workspace's host isn't in iglud's configuration.
    HostUnknown,
    /// The host couldn't do it. `detail` is in the host's own words, shown as
    /// it is, never translated.
    HostFailed {
        code: ErrorCode,
        detail: String,
    },

    /// iglud's fault; its log says more.
    Internal,
}

impl Reason {
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::Invalid
            | Self::NotJson
            | Self::Unreadable
            | Self::NotWebSocket
            | Self::IdempotencyKeyInvalid
            | Self::ProjectUnnamed
            | Self::NothingToType
            | Self::NoWorkspaceToGrant { .. }
            | Self::BrowserPort { .. }
            | Self::SignInIncomplete
            | Self::OtherBrowser
            | Self::SignInExpired
            | Self::CliSignInMalformed
            | Self::BadReturnAddress => ErrorKind::BadRequest,
            Self::Unauthorized => ErrorKind::Unauthorized,
            Self::AccountDisabled
            | Self::NotGranted
            | Self::PersonOnly
            | Self::NotFromWorkspace
            | Self::CrossOrigin
            | Self::SignInFailed
            | Self::ProviderRefused { .. }
            | Self::AccountNotAllowed => ErrorKind::Forbidden,
            Self::NotFound => ErrorKind::NotFound,
            Self::MethodNotAllowed => ErrorKind::MethodNotAllowed,
            Self::NotRunning
            | Self::IdempotencyKeyReused
            | Self::NoSuchProject
            | Self::NoEnvironment { .. }
            | Self::NotBuilt { .. }
            | Self::NoRepository
            | Self::WorkspaceNameTaken
            | Self::ProjectNameTaken
            | Self::EnvironmentNameTaken
            | Self::BeingDeleted
            | Self::FreezeNeedsRunning
            | Self::NoBuildToUpdateTo
            | Self::BuildForAnotherAccount
            | Self::NoBrowserInImage
            | Self::BrowserColumnExists { .. }
            | Self::ColumnNameTaken { .. }
            | Self::LayoutMismatch
            | Self::SecretTargetTaken { .. }
            | Self::ProjectChanged
            | Self::BuiltinProject
            | Self::ProjectInUse => ErrorKind::Conflict,
            Self::TooLarge => ErrorKind::TooLarge,
            Self::JsonExpected | Self::FormExpected => ErrorKind::UnsupportedMediaType,
            Self::ProviderUnreachable
            | Self::NoHost
            | Self::HostUnknown
            | Self::HostFailed { .. } => ErrorKind::Unavailable,
            Self::Internal => ErrorKind::Internal,
        }
    }
}

impl From<TransitionError> for Reason {
    fn from(error: TransitionError) -> Self {
        match error {
            TransitionError::Deleting => Self::BeingDeleted,
            TransitionError::FreezeNeedsRunning => Self::FreezeNeedsRunning,
        }
    }
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid => f.write_str("that isn't valid"),
            Self::NotJson => f.write_str("the body isn't JSON"),
            Self::Unreadable => f.write_str("the request body couldn't be read in time"),
            Self::NotWebSocket => f.write_str("connect with a WebSocket"),
            Self::IdempotencyKeyInvalid => f.write_str("Idempotency-Key must be 1-200 characters"),
            Self::ProjectUnnamed => f.write_str("give the project a name or a repository"),
            Self::NothingToType => f.write_str("say what to type, or to press Enter"),
            Self::NoWorkspaceToGrant { workspace } => {
                write!(f, "no workspace {workspace} to grant")
            }
            Self::BrowserPort { port } => write!(
                f,
                "port {port} is the workspace browser's DevTools port or its display, and a \
                 preview of either would let anyone with the link drive the browser"
            ),
            Self::SignInIncomplete => f.write_str("the identity provider sent no code"),
            Self::OtherBrowser => f.write_str("finish signing in in the browser that started it"),
            Self::SignInExpired => f.write_str("this sign-in expired; try again"),
            Self::CliSignInMalformed => f.write_str("malformed CLI sign-in request"),
            Self::BadReturnAddress => f.write_str("bad return address"),
            Self::Unauthorized => f.write_str("sign in first"),
            Self::AccountDisabled => f.write_str("this account is disabled"),
            Self::NotGranted => f.write_str("this workspace hasn't been granted that"),
            Self::PersonOnly => f.write_str("only a person can do this"),
            Self::NotFromWorkspace => f.write_str(
                "a workspace can't do this from inside; its owner can, in the console or \
                 with their own `iglu`",
            ),
            Self::CrossOrigin => f.write_str("refused a request from another site"),
            Self::SignInFailed => f.write_str("sign-in failed"),
            Self::ProviderRefused { error } => {
                write!(f, "the identity provider refused: {error}")
            }
            Self::AccountNotAllowed => f.write_str("this account isn't allowed to use iglu"),
            Self::NotFound => f.write_str("not found"),
            Self::MethodNotAllowed => f.write_str("this path doesn't take that method"),
            Self::NotRunning => f.write_str("the workspace isn't running"),
            Self::IdempotencyKeyReused => {
                f.write_str("Idempotency-Key reused with a different request")
            }
            Self::NoSuchProject => f.write_str("no such project"),
            Self::NoEnvironment { name } => write!(f, "no environment called {name}"),
            Self::NotBuilt { environment } => {
                write!(f, "the environment {environment} has no built image yet")
            }
            Self::NoRepository => {
                f.write_str("the project has no repository, so its workspaces have no branch")
            }
            Self::WorkspaceNameTaken => f.write_str("a workspace with that name exists"),
            Self::ProjectNameTaken => f.write_str("a project with that name exists"),
            Self::EnvironmentNameTaken => f.write_str("an environment with that name exists"),
            Self::BeingDeleted => f.write_str("the workspace is being deleted"),
            Self::FreezeNeedsRunning => f.write_str("only a running workspace can be frozen"),
            Self::NoBuildToUpdateTo => {
                f.write_str("its environment has no build ready to update to")
            }
            Self::BuildForAnotherAccount => f.write_str(
                "the environment's newest build has another account than this workspace, \
                 whose files would be someone else's; recreate it to use the build",
            ),
            Self::NoBrowserInImage => f.write_str(
                "this workspace's image has no browser: rebuild its environment with \
                 iglu.browser.enable, then update the workspace",
            ),
            Self::BrowserColumnExists { column } => {
                write!(f, "this workspace has a browser already: {column}")
            }
            Self::ColumnNameTaken { name } => write!(f, "{name} is already a column"),
            Self::LayoutMismatch => {
                f.write_str("a layout names every column once, no more and no fewer")
            }
            Self::SecretTargetTaken { by } => write!(f, "{by} already goes there"),
            Self::ProjectChanged => f.write_str("the project changed since you loaded it"),
            Self::BuiltinProject => f.write_str("the built-in project can't be removed"),
            Self::ProjectInUse => f.write_str("the project still has workspaces"),
            Self::TooLarge => f.write_str("the request body is too large"),
            Self::JsonExpected => {
                f.write_str("send the body as JSON, with Content-Type: application/json")
            }
            Self::FormExpected => f.write_str("send the body as a form"),
            Self::ProviderUnreachable => {
                f.write_str("the identity provider isn't reachable; try again shortly")
            }
            Self::NoHost => f.write_str("no execution host configured"),
            Self::HostUnknown => f.write_str("the workspace's host isn't configured"),
            Self::HostFailed { detail, .. } => f.write_str(detail),
            Self::Internal => f.write_str("internal error"),
        }
    }
}

/// Why iglud ended a terminal's or browser's WebSocket on purpose, sent as
/// the close frame's reason: its name, which the console says in words and
/// `Display` says in English.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Closed {
    /// The workspace isn't running, and didn't thaw in time.
    NotRunning,
    /// The person's access to the workspace ended while it was open.
    AccessEnded,
    /// The host couldn't attach to the session; iglud's log says why.
    HostFailed,
    /// The workspace browser's display didn't answer: it isn't running.
    BrowserNotRunning,
    /// The browser runs, but its display couldn't be reached.
    BrowserUnreachable,
}

impl Closed {
    /// Its name, as the close frame carries it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRunning => "NotRunning",
            Self::AccessEnded => "AccessEnded",
            Self::HostFailed => "HostFailed",
            Self::BrowserNotRunning => "BrowserNotRunning",
            Self::BrowserUnreachable => "BrowserUnreachable",
        }
    }
}

impl std::str::FromStr for Closed {
    type Err = serde::de::value::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use serde::de::IntoDeserializer;
        Self::deserialize(s.into_deserializer())
    }
}

impl std::fmt::Display for Closed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotRunning => "the workspace isn't running",
            Self::AccessEnded => "access to this workspace ended",
            Self::HostFailed => "the workspace's host couldn't attach to it",
            Self::BrowserNotRunning => "the browser isn't running; restart its column",
            Self::BrowserUnreachable => "the browser's display isn't answering; restart its column",
        })
    }
}

/// What kind of refusal an error is. Each kind has one HTTP status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// What to call it. Names it too when no name is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub label: Option<WorkspaceLabel>,
    /// The branch to work on. Defaults to the workspace name, and names it
    /// when no name or label is given. Only for projects with a repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub branch: Option<BranchName>,
    /// Where a new branch starts. Defaults to the remote's default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub base: Option<BranchName>,
    /// Made from the label or the branch when absent, or else generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<WorkspaceName>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SetDesiredState {
    pub state: DesiredState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RenameWorkspace {
    pub name: WorkspaceName,
}

/// What to call a workspace; `null` goes back to its name.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LabelWorkspace {
    pub label: Option<WorkspaceLabel>,
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
        let whole = r#"{"expected_revision":1,"name":"app","repo":null,"environment":"default","opening":[],"ports":[],"idle":{"kind":"Default"}}"#;
        assert!(serde_json::from_str::<ChangeProject>(whole).is_ok());
        let without_repo = whole.replace(r#""repo":null,"#, "");
        assert!(serde_json::from_str::<ChangeProject>(&without_repo).is_err());
    }

    #[test]
    fn a_revision_flattens_its_status() {
        let json = r#"{"id":"6f2c3c7e-2f7b-4a8e-9c0a-1c9f4f2b1d11","status":"Failed","log_tail":"boom","created_at":5}"#;
        let revision: RevisionView = serde_json::from_str(json).expect("parses");
        assert!(
            matches!(revision.status, RevisionStatus::Failed { ref log_tail } if log_tail == "boom")
        );
    }

    #[test]
    fn a_close_reason_is_its_serde_name() {
        for closed in [
            Closed::NotRunning,
            Closed::AccessEnded,
            Closed::HostFailed,
            Closed::BrowserNotRunning,
            Closed::BrowserUnreachable,
        ] {
            let json = serde_json::to_string(&closed).expect("serializes");
            assert_eq!(json, format!("\"{}\"", closed.as_str()));
            assert_eq!(closed.as_str().parse::<Closed>().ok(), Some(closed));
        }
        assert!("the workspace isn't running".parse::<Closed>().is_err());
    }

    #[test]
    fn unknown_request_fields_are_refused() {
        let result: Result<PublishPort, _> = serde_json::from_str(r#"{"port":3000,"host":"x"}"#);
        assert!(result.is_err());
    }
}
