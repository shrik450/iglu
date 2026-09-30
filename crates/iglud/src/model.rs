//! Records iglud stores, as domain types. What the API shows lives in
//! `iglu-api`; `views` assembles it from these.

use iglu_domain::attention::{Seen, SessionStatus};
use iglu_domain::auth::{DisplayName, Email, PrincipalStatus};
use iglu_domain::capacity::Bytes;
use iglu_domain::env::{EnvName, EnvSource};
use iglu_domain::id::{EnvRevisionId, PrincipalId, RouteId, WorkspaceId};
use iglu_domain::label::{HostId, RouteName, WorkspaceName};
use iglu_domain::lifecycle::{DesiredState, Instance, Phase, Revision, SecretsGeneration};
use iglu_domain::port::GuestPort;
use iglu_domain::repo::{BranchName, RepoUrl};
use iglu_domain::time::Timestamp;

pub use iglu_api::Condition;

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

#[derive(Clone, Debug)]
pub struct RouteRecord {
    pub id: RouteId,
    pub workspace: WorkspaceId,
    pub name: RouteName,
    pub port: GuestPort,
}
