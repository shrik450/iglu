//! Records iglud stores, as domain types, and what the API shows.

use iglu_domain::attention::{AttentionState, Seen, SessionStatus, Summary};
use iglu_domain::auth::{DisplayName, Email, PrincipalStatus};
use iglu_domain::capacity::{Bytes, Capacity};
use iglu_domain::env::{BuiltImage, EnvName, EnvSource};
use iglu_domain::id::{EnvRevisionId, PrincipalId, RouteId, SecretId, WorkspaceId};
use iglu_domain::label::{HostId, RouteName, WorkspaceName};
use iglu_domain::lifecycle::{DesiredState, Instance, Phase, Revision, SecretsGeneration};
use iglu_domain::port::GuestPort;
use iglu_domain::repo::{BranchName, RepoUrl};
use iglu_domain::secret::{SecretName, SecretTarget};
use iglu_domain::terminal::SessionName;
use iglu_domain::time::Timestamp;
use iglu_proto::ErrorCode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct PrincipalRecord {
    pub id: PrincipalId,
    pub email: Option<Email>,
    pub name: Option<DisplayName>,
    pub status: PrincipalStatus,
    pub secrets_generation: SecretsGeneration,
}

impl PrincipalRecord {
    pub const fn principal(&self) -> iglu_domain::auth::Principal {
        iglu_domain::auth::Principal {
            id: self.id,
            status: self.status,
        }
    }
}

/// Something about a workspace the owner should know, beyond its phase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
    pub const fn from_capacity(capacity: Capacity) -> Option<Self> {
        match capacity {
            Capacity::Fits => None,
            Capacity::Short { available, needed } => Some(Self::Capacity { available, needed }),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceRecord {
    pub id: WorkspaceId,
    pub owner: PrincipalId,
    pub host: HostId,
    pub env_revision: EnvRevisionId,
    pub name: WorkspaceName,
    pub repo: RepoUrl,
    pub branch: BranchName,
    pub base: Option<BranchName>,
    pub desired: DesiredState,
    pub revision: Revision,
    /// `None` until the host has reported on it once.
    pub observed: Option<Instance>,
    pub observed_at: Option<Timestamp>,
    pub memory: Option<Bytes>,
    pub condition: Option<Condition>,
    pub created_at: Timestamp,
}

impl WorkspaceRecord {
    pub fn phase(&self) -> Phase {
        iglu_domain::lifecycle::phase(self.desired, self.observed.unwrap_or(Instance::Absent))
    }
}

#[derive(Clone, Debug)]
pub struct AttentionRecord {
    pub status: SessionStatus,
    pub seen: Seen,
}

#[derive(Clone, Debug)]
pub struct EnvironmentRecord {
    pub id: uuid::Uuid,
    pub name: EnvName,
    pub source: EnvSource,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RevisionStatus {
    Building,
    Ready { image: BuiltImage },
    Failed { log_tail: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct RevisionRecord {
    pub id: EnvRevisionId,
    #[serde(flatten)]
    pub status: RevisionStatus,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteRecord {
    pub id: RouteId,
    pub workspace: WorkspaceId,
    pub name: RouteName,
    pub port: GuestPort,
}

#[derive(Clone, Debug, Serialize)]
pub struct SecretSummary {
    pub id: SecretId,
    pub name: SecretName,
    pub target: SecretTarget,
    pub updated_at: Timestamp,
}

/// A workspace as the console and CLI see it.
#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceView {
    pub id: WorkspaceId,
    pub name: WorkspaceName,
    pub repo: RepoUrl,
    pub branch: BranchName,
    pub environment: EnvName,
    pub phase: Phase,
    pub desired: DesiredState,
    pub revision: Revision,
    pub condition: Option<Condition>,
    pub memory: Option<Bytes>,
    /// When the host last reported on the workspace.
    pub observed_at: Option<Timestamp>,
    pub attention: Option<AttentionView>,
    pub sessions: Vec<AttentionView>,
    pub routes: Vec<RouteView>,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Serialize)]
pub struct AttentionView {
    pub session: SessionName,
    pub state: AttentionState,
    pub summary: Summary,
    pub updated_at: Timestamp,
    pub seen: Seen,
}

impl From<&AttentionRecord> for AttentionView {
    fn from(record: &AttentionRecord) -> Self {
        Self {
            session: record.status.session.clone(),
            state: record.status.state,
            summary: record.status.summary.clone(),
            updated_at: record.status.updated_at,
            seen: record.seen,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteView {
    pub id: RouteId,
    pub name: RouteName,
    pub port: GuestPort,
    pub url: String,
}
