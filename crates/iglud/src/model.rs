//! Records iglud stores, as domain types. What the API shows lives in
//! `iglu-api`; `views` assembles it from these.

use iglu_domain::agent::Prompt;
use iglu_domain::attention::{Seen, SessionStatus};
use iglu_domain::auth::{DisplayName, Email, PrincipalStatus};
use iglu_domain::capacity::Bytes;
use iglu_domain::column::ColumnSpec;
use iglu_domain::env::{EnvName, EnvSource};
use iglu_domain::id::{EnvRevisionId, PrincipalId, ProjectId, RouteId, WorkspaceId};
use iglu_domain::idle::IdleRule;
use iglu_domain::label::{AgentName, HostId, ProjectName, RouteName, WorkspaceName};
use iglu_domain::lifecycle::{DesiredState, Instance, Phase, Revision, SecretsGeneration};
use iglu_domain::port::GuestPort;
use iglu_domain::project::{Opening, Origin, PreviewPorts};
use iglu_domain::repo::{Checkout, RepoUrl};
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
    pub project: ProjectId,
    pub checkout: Option<Checkout>,
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
pub struct ProjectRecord {
    pub id: ProjectId,
    pub name: ProjectName,
    pub origin: Origin,
    pub repo: Option<RepoUrl>,
    pub environment_id: uuid::Uuid,
    pub environment: EnvName,
    pub opening: Opening,
    pub agent: Option<AgentName>,
    pub ports: PreviewPorts,
    pub idle: IdleRule,
    pub revision: Revision,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug)]
pub struct EnvironmentRecord {
    pub id: uuid::Uuid,
    pub name: EnvName,
    pub source: EnvSource,
}

/// A column as stored: its spec, and the prompt its agent starts with until
/// it first opens.
#[derive(Clone, Debug)]
pub struct ColumnRecord {
    pub spec: ColumnSpec,
    pub prompt: Option<Prompt>,
}

#[derive(Clone, Debug)]
pub struct RouteRecord {
    pub id: RouteId,
    pub workspace: WorkspaceId,
    pub name: RouteName,
    pub port: GuestPort,
}
