//! SQLite storage. Every row is parsed into domain types on the way out, so
//! nothing past this module handles raw column text.
//!
//! One connection, used from blocking threads: iglud is a single process and
//! SQLite serializes writes anyway.

use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use iglu_api::{ActivityEntry, RevisionStatus, RevisionView, SecretView};
use iglu_domain::agent::Prompt;
use iglu_domain::attention::{Seen, SessionStatus};
use iglu_domain::auth::VerifiedIdentity;
use iglu_domain::capacity::Bytes;
use iglu_domain::column::ColumnSpec;
use iglu_domain::env::{BuiltImage, EnvName, EnvSource};
use iglu_domain::id::{EnvRevisionId, PrincipalId, ProjectId, RouteId, SecretId, WorkspaceId};
use iglu_domain::idle::IdleRule;
use iglu_domain::label::{AgentName, ProjectName, RouteName, WorkspaceName};
use iglu_domain::lifecycle::{DesiredState, Instance, Revision, SecretsGeneration};
use iglu_domain::port::GuestPort;
use iglu_domain::project::{self, Opening, Origin, PreviewPorts};
use iglu_domain::repo::{BranchName, Checkout, RepoUrl};
use iglu_domain::secret::{SecretName, SecretTarget};
use iglu_domain::terminal::SessionName;
use iglu_domain::time::Timestamp;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::model::{
    AttentionRecord, ColumnRecord, Condition, EnvironmentRecord, PrincipalRecord, ProjectRecord,
    RouteRecord, WorkspaceRecord,
};

/// One step of the schema's history: SQL, or a data change SQL can't express.
enum Migration {
    Sql(&'static str),
    Data(fn(&Connection) -> Result<(), DbError>),
}

/// The schema's history, oldest first. `user_version` records how many have
/// been applied. Never edit one that has shipped; add another.
const MIGRATIONS: &[Migration] = &[
    Migration::Sql(include_str!("migrations/0001_initial.sql")),
    Migration::Sql(include_str!("migrations/0002_columns.sql")),
    Migration::Sql(include_str!("migrations/0003_projects.sql")),
    Migration::Data(projects_for_existing_workspaces),
    Migration::Sql(include_str!("migrations/0005_threads.sql")),
    Migration::Sql(include_str!("migrations/0006_stored_shapes.sql")),
];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "the database has schema version {found}, newer than this iglud ({}); run a newer iglud",
        MIGRATIONS.len()
    )]
    TooNew { found: usize },
    #[error("database worker stopped")]
    Worker,
}

#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

/// Brings the schema up to date, one migration per transaction.
fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    migrate_until(conn, MIGRATIONS.len())
}

/// Applies the migrations up to version `until`.
fn migrate_until(conn: &mut Connection, until: usize) -> Result<(), DbError> {
    let raw: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let applied = usize::try_from(raw).unwrap_or(usize::MAX);
    if applied > MIGRATIONS.len() {
        return Err(DbError::TooNew { found: applied });
    }
    for (index, migration) in MIGRATIONS.iter().enumerate().take(until).skip(applied) {
        let tx = conn.transaction()?;
        match migration {
            Migration::Sql(sql) => tx.execute_batch(sql)?,
            Migration::Data(change) => change(&tx)?,
        }
        let version = i64::try_from(index + 1).expect("there are fewer migrations than i64::MAX");
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        tracing::info!(version, "applied a database migration");
    }
    Ok(())
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(mut conn: Connection) -> Result<Self, DbError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&mut conn)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    /// Runs `f` on the connection in a blocking thread, inside a transaction.
    pub async fn call<T, F>(&self, f: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T, DbError> + Send + 'static,
    {
        let conn = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().map_err(|_| DbError::Worker)?;
            let tx = conn.transaction()?;
            let value = f(&tx)?;
            tx.commit()?;
            Ok(value)
        })
        .await
        .map_err(|_| DbError::Worker)?
    }
}

fn conversion(
    index: usize,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
}

fn text<T>(row: &Row<'_>, index: usize) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let raw: String = row.get(index)?;
    raw.parse().map_err(|e| conversion(index, e))
}

fn opt_text<T>(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<T>>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let raw: Option<String> = row.get(index)?;
    raw.map(|r| r.parse().map_err(|e| conversion(index, e)))
        .transpose()
}

fn json<T: DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let raw: String = row.get(index)?;
    serde_json::from_str(&raw).map_err(|e| conversion(index, e))
}

fn opt_json<T: DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<T>> {
    let raw: Option<String> = row.get(index)?;
    raw.map(|r| serde_json::from_str(&r).map_err(|e| conversion(index, e)))
        .transpose()
}

fn to_json(value: &impl Serialize) -> Result<String, DbError> {
    Ok(serde_json::to_string(value)?)
}

fn timestamp(row: &Row<'_>, index: usize) -> rusqlite::Result<Timestamp> {
    row.get(index).map(Timestamp::from_unix_millis)
}

fn uuid_text(id: Uuid) -> String {
    id.hyphenated().to_string()
}

fn u64_col(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|e| conversion(index, e))
}

fn i64_of(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

// ---- principals ----

const PRINCIPAL_COLUMNS: &str = "id, email, name, status, secrets_generation";

fn principal_row(row: &Row<'_>) -> rusqlite::Result<PrincipalRecord> {
    Ok(PrincipalRecord {
        id: text(row, 0)?,
        email: opt_text(row, 1)?,
        name: opt_text(row, 2)?,
        status: text(row, 3)?,
        secrets_generation: SecretsGeneration::from_u64(u64_col(row, 4)?),
    })
}

pub fn upsert_principal(
    tx: &Connection,
    new_id: PrincipalId,
    identity: &VerifiedIdentity,
    now: Timestamp,
) -> Result<PrincipalRecord, DbError> {
    tx.execute(
        "INSERT INTO principal (id, issuer, subject, email, name, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6)
         ON CONFLICT (issuer, subject) DO UPDATE SET email = excluded.email, name = excluded.name",
        params![
            new_id.to_string(),
            identity.issuer.as_str(),
            identity.subject.as_str(),
            identity
                .email
                .as_ref()
                .map(iglu_domain::auth::Email::as_str),
            identity
                .name
                .as_ref()
                .map(iglu_domain::auth::DisplayName::as_str),
            now.unix_millis(),
        ],
    )?;
    Ok(tx.query_row(
        &format!("SELECT {PRINCIPAL_COLUMNS} FROM principal WHERE issuer = ?1 AND subject = ?2"),
        params![identity.issuer.as_str(), identity.subject.as_str()],
        principal_row,
    )?)
}

pub fn principal(tx: &Connection, id: PrincipalId) -> Result<Option<PrincipalRecord>, DbError> {
    Ok(tx
        .query_row(
            &format!("SELECT {PRINCIPAL_COLUMNS} FROM principal WHERE id = ?1"),
            [id.to_string()],
            principal_row,
        )
        .optional()?)
}

fn bump_secrets_generation(tx: &Connection, owner: PrincipalId) -> Result<(), DbError> {
    tx.execute(
        "UPDATE principal SET secrets_generation = secrets_generation + 1 WHERE id = ?1",
        [owner.to_string()],
    )?;
    Ok(())
}

// ---- sessions ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Console,
    Preview,
    Api,
}

impl SessionKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Console => "console",
            Self::Preview => "preview",
            Self::Api => "api",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("expected a session kind: console, preview or api")]
pub struct UnknownSessionKind;

impl FromStr for SessionKind {
    type Err = UnknownSessionKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "console" => Ok(Self::Console),
            "preview" => Ok(Self::Preview),
            "api" => Ok(Self::Api),
            _ => Err(UnknownSessionKind),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionRow {
    pub principal: PrincipalId,
    pub csrf: String,
    pub last_seen_at: Timestamp,
    pub expires_at: Timestamp,
}

pub fn create_session(
    tx: &Connection,
    token_hash: &str,
    kind: SessionKind,
    principal: PrincipalId,
    csrf: &str,
    now: Timestamp,
    expires_at: Timestamp,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO web_session (token_hash, principal_id, kind, csrf, created_at, last_seen_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
        params![token_hash, principal.to_string(), kind.as_str(), csrf, now.unix_millis(), expires_at.unix_millis()],
    )?;
    Ok(())
}

pub fn session(
    tx: &Connection,
    token_hash: &str,
    kind: SessionKind,
) -> Result<Option<SessionRow>, DbError> {
    Ok(tx
        .query_row(
            "SELECT principal_id, csrf, last_seen_at, expires_at FROM web_session
             WHERE token_hash = ?1 AND kind = ?2",
            params![token_hash, kind.as_str()],
            |row| {
                Ok(SessionRow {
                    principal: text(row, 0)?,
                    csrf: row.get(1)?,
                    last_seen_at: timestamp(row, 2)?,
                    expires_at: timestamp(row, 3)?,
                })
            },
        )
        .optional()?)
}

pub fn touch_session(tx: &Connection, token_hash: &str, now: Timestamp) -> Result<(), DbError> {
    tx.execute(
        "UPDATE web_session SET last_seen_at = ?2 WHERE token_hash = ?1",
        params![token_hash, now.unix_millis()],
    )?;
    Ok(())
}

pub fn delete_session(tx: &Connection, token_hash: &str) -> Result<(), DbError> {
    tx.execute(
        "DELETE FROM web_session WHERE token_hash = ?1",
        [token_hash],
    )?;
    Ok(())
}

pub fn expire_sessions(tx: &Connection, now: Timestamp) -> Result<(), DbError> {
    tx.execute(
        "DELETE FROM web_session WHERE expires_at < ?1",
        [now.unix_millis()],
    )?;
    tx.execute(
        "DELETE FROM login WHERE created_at < ?1",
        [now.unix_millis() - 600_000],
    )?;
    tx.execute(
        "DELETE FROM cli_code WHERE expires_at < ?1",
        [now.unix_millis()],
    )?;
    Ok(())
}

// ---- logins and CLI codes ----

#[derive(Clone, Debug)]
pub struct LoginRow {
    pub kind: SessionKind,
    pub nonce: String,
    pub pkce_verifier: String,
    pub return_to: String,
}

pub fn insert_login(
    tx: &Connection,
    state: &str,
    login: &LoginRow,
    now: Timestamp,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO login (state, kind, nonce, pkce_verifier, return_to, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![state, login.kind.as_str(), login.nonce, login.pkce_verifier, login.return_to, now.unix_millis()],
    )?;
    Ok(())
}

/// Consumes a pending login: each state is usable once.
pub fn take_login(tx: &Connection, state: &str) -> Result<Option<LoginRow>, DbError> {
    let row = tx
        .query_row(
            "SELECT kind, nonce, pkce_verifier, return_to FROM login WHERE state = ?1",
            [state],
            |row| {
                Ok(LoginRow {
                    kind: text(row, 0)?,
                    nonce: row.get(1)?,
                    pkce_verifier: row.get(2)?,
                    return_to: row.get(3)?,
                })
            },
        )
        .optional()?;
    tx.execute("DELETE FROM login WHERE state = ?1", [state])?;
    Ok(row)
}

pub fn insert_cli_code(
    tx: &Connection,
    code_hash: &str,
    principal: PrincipalId,
    challenge: &str,
    expires: Timestamp,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO cli_code (code_hash, principal_id, challenge, expires_at) VALUES (?1, ?2, ?3, ?4)",
        params![code_hash, principal.to_string(), challenge, expires.unix_millis()],
    )?;
    Ok(())
}

pub fn take_cli_code(
    tx: &Connection,
    code_hash: &str,
    now: Timestamp,
) -> Result<Option<(PrincipalId, String)>, DbError> {
    let row = tx
        .query_row(
            "SELECT principal_id, challenge FROM cli_code WHERE code_hash = ?1 AND expires_at >= ?2",
            params![code_hash, now.unix_millis()],
            |row| Ok((text(row, 0)?, row.get(1)?)),
        )
        .optional()?;
    tx.execute("DELETE FROM cli_code WHERE code_hash = ?1", [code_hash])?;
    Ok(row)
}

// ---- environments ----

fn environment_row(row: &Row<'_>) -> rusqlite::Result<EnvironmentRecord> {
    Ok(EnvironmentRecord {
        id: text(row, 0)?,
        name: text(row, 2)?,
        source: text(row, 3)?,
    })
}

pub fn create_environment(
    tx: &Connection,
    id: Uuid,
    owner: PrincipalId,
    name: &EnvName,
    source: &EnvSource,
    now: Timestamp,
) -> Result<bool, DbError> {
    let inserted = tx.execute(
        "INSERT INTO environment (id, owner_id, name, source, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (owner_id, name) DO NOTHING",
        params![uuid_text(id), owner.to_string(), name.as_str(), source.to_string(), now.unix_millis()],
    )?;
    Ok(inserted == 1)
}

pub fn environment(
    tx: &Connection,
    owner: PrincipalId,
    name: &EnvName,
) -> Result<Option<EnvironmentRecord>, DbError> {
    Ok(tx
        .query_row(
            "SELECT id, owner_id, name, source FROM environment WHERE owner_id = ?1 AND name = ?2",
            params![owner.to_string(), name.as_str()],
            environment_row,
        )
        .optional()?)
}

pub fn environments(
    tx: &Connection,
    owner: PrincipalId,
) -> Result<Vec<(EnvironmentRecord, Option<RevisionView>)>, DbError> {
    let mut statement = tx.prepare(
        "SELECT id, owner_id, name, source FROM environment WHERE owner_id = ?1 ORDER BY name",
    )?;
    let envs: Vec<EnvironmentRecord> = statement
        .query_map([owner.to_string()], environment_row)?
        .collect::<Result<_, _>>()?;
    envs.into_iter()
        .map(|env| {
            let latest = latest_revision(tx, env.id)?;
            Ok((env, latest))
        })
        .collect()
}

fn revision_row(row: &Row<'_>) -> rusqlite::Result<RevisionView> {
    let status: String = row.get(1)?;
    let status = match status.as_str() {
        "ready" => RevisionStatus::Ready {
            image: opt_json::<BuiltImage>(row, 2)?
                .ok_or_else(|| conversion(2, std::io::Error::other("missing image")))?,
        },
        "failed" => RevisionStatus::Failed {
            log_tail: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        },
        "building" => RevisionStatus::Building,
        _ => {
            return Err(conversion(
                1,
                std::io::Error::other("unknown revision status"),
            ));
        }
    };
    Ok(RevisionView {
        id: text(row, 0)?,
        status,
        created_at: timestamp(row, 4)?,
    })
}

fn latest_revision(tx: &Connection, env: Uuid) -> Result<Option<RevisionView>, DbError> {
    Ok(tx
        .query_row(
            "SELECT id, status, built, log_tail, created_at FROM env_revision WHERE environment_id = ?1
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            [uuid_text(env)],
            revision_row,
        )
        .optional()?)
}

pub fn latest_ready_revision(
    tx: &Connection,
    env: Uuid,
) -> Result<Option<(EnvRevisionId, BuiltImage)>, DbError> {
    Ok(tx
        .query_row(
            "SELECT id, built FROM env_revision WHERE environment_id = ?1 AND status = 'ready'
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            [uuid_text(env)],
            |row| {
                let image = opt_json::<BuiltImage>(row, 1)?
                    .ok_or_else(|| conversion(1, std::io::Error::other("missing image")))?;
                Ok((text(row, 0)?, image))
            },
        )
        .optional()?)
}

pub fn insert_revision(
    tx: &Connection,
    id: EnvRevisionId,
    env: Uuid,
    now: Timestamp,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO env_revision (id, environment_id, status, created_at) VALUES (?1, ?2, 'building', ?3)",
        params![id.to_string(), uuid_text(env), now.unix_millis()],
    )?;
    Ok(())
}

pub fn finish_revision(
    tx: &Connection,
    id: EnvRevisionId,
    outcome: Result<&BuiltImage, &str>,
    now: Timestamp,
) -> Result<(), DbError> {
    match outcome {
        Ok(image) => tx.execute(
            "UPDATE env_revision SET status = 'ready', built = ?2, finished_at = ?3 WHERE id = ?1",
            params![id.to_string(), to_json(image)?, now.unix_millis()],
        )?,
        Err(log) => tx.execute(
            "UPDATE env_revision SET status = 'failed', log_tail = ?2, finished_at = ?3 WHERE id = ?1",
            params![id.to_string(), log, now.unix_millis()],
        )?,
    };
    Ok(())
}

pub fn revision_image(tx: &Connection, id: EnvRevisionId) -> Result<Option<BuiltImage>, DbError> {
    let built: Option<Option<String>> = tx
        .query_row(
            "SELECT built FROM env_revision WHERE id = ?1 AND status = 'ready'",
            [id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    built
        .flatten()
        .map(|json| serde_json::from_str(&json).map_err(DbError::from))
        .transpose()
}

pub fn environment_name_of_revision(
    tx: &Connection,
    id: EnvRevisionId,
) -> Result<Option<EnvName>, DbError> {
    Ok(tx
        .query_row(
            "SELECT e.name FROM env_revision r JOIN environment e ON e.id = r.environment_id WHERE r.id = ?1",
            [id.to_string()],
            |row| text(row, 0),
        )
        .optional()?)
}

// ---- workspaces ----

// The checkout's columns are named apart from the workspace's, so queries can
// name either without qualifying.
const WORKSPACE_COLUMNS: &str =
    "id, owner_id, host_id, env_revision_id, name, repo, branch, base, desired, revision,
    observed, observed_at, memory, condition, created_at, project_id";
const WORKSPACE_FROM: &str =
    "workspace LEFT JOIN workspace_checkout ON workspace_checkout.workspace_id = workspace.id";

fn workspace_row(row: &Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    let memory: Option<i64> = row.get(12)?;
    Ok(WorkspaceRecord {
        id: text(row, 0)?,
        owner: text(row, 1)?,
        host: text(row, 2)?,
        env_revision: text(row, 3)?,
        name: text(row, 4)?,
        project: text(row, 15)?,
        checkout: match opt_text(row, 5)? {
            Some(repo) => Some(Checkout {
                repo,
                branch: text(row, 6)?,
                base: opt_text(row, 7)?,
            }),
            None => None,
        },
        desired: text(row, 8)?,
        revision: Revision::from_u64(u64_col(row, 9)?),
        observed: opt_json::<Instance>(row, 10)?,
        observed_at: row
            .get::<_, Option<i64>>(11)?
            .map(Timestamp::from_unix_millis),
        memory: memory.and_then(|m| u64::try_from(m).ok()).map(Bytes::new),
        condition: opt_json::<Condition>(row, 13)?,
        created_at: timestamp(row, 14)?,
    })
}

pub fn insert_workspace(
    tx: &Connection,
    ws: &WorkspaceRecord,
    create_key: Option<&str>,
    create_hash: &str,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO workspace (id, owner_id, host_id, env_revision_id, name, project_id, desired, revision,
                                create_key, create_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            ws.id.to_string(),
            ws.owner.to_string(),
            ws.host.as_str(),
            ws.env_revision.to_string(),
            ws.name.as_str(),
            ws.project.to_string(),
            ws.desired.as_str(),
            i64_of(ws.revision.get()),
            create_key,
            create_hash,
            ws.created_at.unix_millis(),
        ],
    )?;
    if let Some(checkout) = &ws.checkout {
        tx.execute(
            "INSERT INTO workspace_checkout (workspace_id, repo, branch, base) VALUES (?1, ?2, ?3, ?4)",
            params![
                ws.id.to_string(),
                checkout.repo.as_str(),
                checkout.branch.as_str(),
                checkout.base.as_ref().map(BranchName::as_str),
            ],
        )?;
    }
    Ok(())
}

pub fn workspace_by_create_key(
    tx: &Connection,
    owner: PrincipalId,
    key: &str,
) -> Result<Option<(WorkspaceRecord, String)>, DbError> {
    Ok(tx
        .query_row(
            &format!("SELECT {WORKSPACE_COLUMNS}, create_hash FROM {WORKSPACE_FROM} WHERE owner_id = ?1 AND create_key = ?2"),
            params![owner.to_string(), key],
            |row| Ok((workspace_row(row)?, row.get::<_, Option<String>>(16)?.unwrap_or_default())),
        )
        .optional()?)
}

pub fn workspace_name_taken(
    tx: &Connection,
    owner: PrincipalId,
    name: &str,
) -> Result<bool, DbError> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM workspace WHERE owner_id = ?1 AND name = ?2 AND deleted_at IS NULL",
            params![owner.to_string(), name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// What happened to a rename.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameOutcome {
    Done,
    /// Another live workspace of the same owner already has the name.
    NameTaken,
    Missing,
}

/// Renames a live workspace. Names are unique per owner among live
/// workspaces; the instance keeps its ID-derived name, so nothing else moves.
pub fn rename_workspace(
    tx: &Connection,
    id: WorkspaceId,
    owner: PrincipalId,
    name: &WorkspaceName,
) -> Result<RenameOutcome, DbError> {
    let holder: Option<String> = tx
        .query_row(
            "SELECT id FROM workspace WHERE owner_id = ?1 AND name = ?2 AND deleted_at IS NULL",
            params![owner.to_string(), name.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if holder.is_some_and(|other| other != id.to_string()) {
        return Ok(RenameOutcome::NameTaken);
    }
    let changed = tx.execute(
        "UPDATE workspace SET name = ?2 WHERE id = ?1 AND deleted_at IS NULL",
        params![id.to_string(), name.as_str()],
    )?;
    Ok(if changed == 0 {
        RenameOutcome::Missing
    } else {
        RenameOutcome::Done
    })
}

pub fn workspace(tx: &Connection, id: WorkspaceId) -> Result<Option<WorkspaceRecord>, DbError> {
    Ok(tx
        .query_row(
            &format!(
                "SELECT {WORKSPACE_COLUMNS} FROM {WORKSPACE_FROM} WHERE id = ?1 AND deleted_at IS NULL"
            ),
            [id.to_string()],
            workspace_row,
        )
        .optional()?)
}

pub fn workspaces(tx: &Connection, owner: PrincipalId) -> Result<Vec<WorkspaceRecord>, DbError> {
    let mut statement = tx.prepare(&format!(
        "SELECT {WORKSPACE_COLUMNS} FROM {WORKSPACE_FROM} WHERE owner_id = ?1 AND deleted_at IS NULL ORDER BY created_at"
    ))?;
    Ok(statement
        .query_map([owner.to_string()], workspace_row)?
        .collect::<Result<_, _>>()?)
}

pub fn live_workspaces(tx: &Connection) -> Result<Vec<WorkspaceRecord>, DbError> {
    let mut statement = tx.prepare(&format!(
        "SELECT {WORKSPACE_COLUMNS} FROM {WORKSPACE_FROM} WHERE deleted_at IS NULL ORDER BY created_at"
    ))?;
    Ok(statement
        .query_map([], workspace_row)?
        .collect::<Result<_, _>>()?)
}

// ---- projects ----

const PROJECT_COLUMNS: &str = "project.id, project.name, project.origin, project.repo,
    project.environment_id, environment.name, project.opening, project.revision, project.created_at,
    project.agent, project.ports, project.idle";
const PROJECT_FROM: &str = "project JOIN environment ON environment.id = project.environment_id";

fn project_row(row: &Row<'_>) -> rusqlite::Result<ProjectRecord> {
    Ok(ProjectRecord {
        id: text(row, 0)?,
        name: text(row, 1)?,
        origin: text(row, 2)?,
        repo: opt_text(row, 3)?,
        environment_id: text(row, 4)?,
        environment: text(row, 5)?,
        opening: json(row, 6)?,
        revision: Revision::from_u64(u64_col(row, 7)?),
        created_at: timestamp(row, 8)?,
        agent: opt_text(row, 9)?,
        ports: json(row, 10)?,
        idle: json(row, 11)?,
    })
}

/// A person's projects, by name.
pub fn projects(tx: &Connection, owner: PrincipalId) -> Result<Vec<ProjectRecord>, DbError> {
    let mut statement = tx.prepare(&format!(
        "SELECT {PROJECT_COLUMNS} FROM {PROJECT_FROM} WHERE project.owner_id = ?1 ORDER BY project.name"
    ))?;
    Ok(statement
        .query_map([owner.to_string()], project_row)?
        .collect::<Result<_, _>>()?)
}

pub fn project(
    tx: &Connection,
    owner: PrincipalId,
    id: ProjectId,
) -> Result<Option<ProjectRecord>, DbError> {
    Ok(tx
        .query_row(
            &format!(
                "SELECT {PROJECT_COLUMNS} FROM {PROJECT_FROM} WHERE project.owner_id = ?1 AND project.id = ?2"
            ),
            params![owner.to_string(), id.to_string()],
            project_row,
        )
        .optional()?)
}

/// How a project's workspaces idle; `None` if it's gone.
pub fn project_idle(tx: &Connection, id: ProjectId) -> Result<Option<IdleRule>, DbError> {
    Ok(tx
        .query_row(
            "SELECT idle FROM project WHERE id = ?1",
            [id.to_string()],
            |row| json(row, 0),
        )
        .optional()?)
}

fn project_names(tx: &Connection, owner: PrincipalId) -> Result<Vec<ProjectName>, DbError> {
    let mut statement = tx.prepare("SELECT name FROM project WHERE owner_id = ?1")?;
    Ok(statement
        .query_map([owner.to_string()], |row| text(row, 0))?
        .collect::<Result<_, _>>()?)
}

/// A project as it's added: the name is decided, or suggested from the repository.
pub struct NewProject {
    pub id: ProjectId,
    pub owner: PrincipalId,
    pub name: Option<ProjectName>,
    pub origin: Origin,
    pub repo: Option<RepoUrl>,
    pub environment_id: Uuid,
    pub created_at: Timestamp,
}

#[derive(Debug)]
pub enum AddOutcome {
    Added,
    NameTaken,
}

/// Adds a project that opens one shell. Without a name, it's named after
/// its repository, or after the built-in project's name.
pub fn add_project(tx: &Connection, new: &NewProject) -> Result<AddOutcome, DbError> {
    let taken = project_names(tx, new.owner)?;
    let name = match (&new.name, &new.repo) {
        (Some(name), _) if taken.contains(name) => return Ok(AddOutcome::NameTaken),
        (Some(name), _) => name.clone(),
        (None, Some(repo)) => project::suggest_name(repo, &taken),
        (None, None) => project::unique(&project::general(), &taken),
    };
    tx.execute(
        "INSERT INTO project (id, owner_id, name, origin, repo, environment_id, opening, revision, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            new.id.to_string(),
            new.owner.to_string(),
            name.as_str(),
            new.origin.as_str(),
            new.repo.as_ref().map(RepoUrl::as_str),
            uuid_text(new.environment_id),
            to_json(&Opening::shell())?,
            i64_of(Revision::INITIAL.get()),
            new.created_at.unix_millis(),
        ],
    )?;
    Ok(AddOutcome::Added)
}

/// Gives a person their built-in project, on the environment given, unless
/// they have one.
pub fn ensure_builtin(
    tx: &Connection,
    owner: PrincipalId,
    environment_id: Uuid,
    id: ProjectId,
    now: Timestamp,
) -> Result<(), DbError> {
    let has: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM project WHERE owner_id = ?1 AND origin = 'builtin'",
            [owner.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    if has.is_none() {
        add_project(
            tx,
            &NewProject {
                id,
                owner,
                name: None,
                origin: Origin::Builtin,
                repo: None,
                environment_id,
                created_at: now,
            },
        )?;
    }
    Ok(())
}

/// A project's settings as a person changes them.
pub struct ProjectChange {
    pub name: ProjectName,
    pub repo: Option<RepoUrl>,
    pub environment_id: Uuid,
    pub opening: Opening,
    pub agent: Option<AgentName>,
    pub ports: PreviewPorts,
    pub idle: IdleRule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeOutcome {
    Done,
    /// Someone changed it since the caller looked.
    Stale,
    NameTaken,
    Missing,
}

pub fn change_project(
    tx: &Connection,
    owner: PrincipalId,
    id: ProjectId,
    expected: Revision,
    change: &ProjectChange,
) -> Result<ChangeOutcome, DbError> {
    let Some(current) = project(tx, owner, id)? else {
        return Ok(ChangeOutcome::Missing);
    };
    if current.revision != expected {
        return Ok(ChangeOutcome::Stale);
    }
    if change.name != current.name && project_names(tx, owner)?.contains(&change.name) {
        return Ok(ChangeOutcome::NameTaken);
    }
    tx.execute(
        "UPDATE project SET name = ?3, repo = ?4, environment_id = ?5, opening = ?6, revision = ?7,
                            agent = ?8, ports = ?9, idle = ?10
         WHERE owner_id = ?1 AND id = ?2",
        params![
            owner.to_string(),
            id.to_string(),
            change.name.as_str(),
            change.repo.as_ref().map(RepoUrl::as_str),
            uuid_text(change.environment_id),
            to_json(&change.opening)?,
            i64_of(expected.next().get()),
            change.agent.as_ref().map(AgentName::as_str),
            to_json(&change.ports)?,
            to_json(&change.idle)?,
        ],
    )?;
    Ok(ChangeOutcome::Done)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveOutcome {
    Done,
    Builtin,
    /// Live workspaces are in it.
    InUse,
    Missing,
}

/// Removes a project without live workspaces. Deleted ones move to the
/// built-in project, which nothing removes.
pub fn remove_project(
    tx: &Connection,
    owner: PrincipalId,
    id: ProjectId,
) -> Result<RemoveOutcome, DbError> {
    let Some(current) = project(tx, owner, id)? else {
        return Ok(RemoveOutcome::Missing);
    };
    match current.origin {
        Origin::Builtin => return Ok(RemoveOutcome::Builtin),
        Origin::Added => {}
    }
    let live: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM workspace WHERE project_id = ?1 AND deleted_at IS NULL LIMIT 1",
            [id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    if live.is_some() {
        return Ok(RemoveOutcome::InUse);
    }
    tx.execute(
        "UPDATE workspace SET project_id = (SELECT id FROM project WHERE owner_id = ?1 AND origin = 'builtin')
         WHERE project_id = ?2",
        params![owner.to_string(), id.to_string()],
    )?;
    tx.execute("DELETE FROM project WHERE id = ?1", [id.to_string()])?;
    Ok(RemoveOutcome::Done)
}

/// Migration 4: everyone with an environment gets the built-in project, and
/// each repository and environment live workspaces use becomes a project.
/// Deleted workspaces go to the built-in one.
fn projects_for_existing_workspaces(tx: &Connection) -> Result<(), DbError> {
    let mut statement = tx.prepare(
        "SELECT owner_id, id FROM environment e
         WHERE created_at = (SELECT MIN(created_at) FROM environment WHERE owner_id = e.owner_id)
         GROUP BY owner_id",
    )?;
    let oldest: Vec<(PrincipalId, Uuid)> = statement
        .query_map([], |row| Ok((text(row, 0)?, text(row, 1)?)))?
        .collect::<Result<_, _>>()?;
    let now = Timestamp::from_unix_millis(0);
    for (owner, environment) in oldest {
        ensure_builtin(
            tx,
            owner,
            environment,
            ProjectId::from_uuid(Uuid::new_v4()),
            now,
        )?;
    }
    let mut statement = tx.prepare(
        "SELECT w.id, w.owner_id, r.environment_id, c.repo, w.deleted_at IS NOT NULL
         FROM workspace w
         JOIN env_revision r ON r.id = w.env_revision_id
         LEFT JOIN workspace_checkout c ON c.workspace_id = w.id
         ORDER BY w.created_at",
    )?;
    let workspaces: Vec<(WorkspaceId, PrincipalId, Uuid, Option<RepoUrl>, bool)> = statement
        .query_map([], |row| {
            Ok((
                text(row, 0)?,
                text(row, 1)?,
                text(row, 2)?,
                opt_text(row, 3)?,
                row.get(4)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    let mut made: HashMap<(PrincipalId, Uuid, String), ProjectId> = HashMap::new();
    for (workspace, owner, environment, repo, deleted) in workspaces {
        let project = match repo.filter(|_| !deleted) {
            None => tx.query_row(
                "SELECT id FROM project WHERE owner_id = ?1 AND origin = 'builtin'",
                [owner.to_string()],
                |row| text(row, 0),
            )?,
            Some(repo) => {
                let key = (owner, environment, repo.as_str().to_owned());
                if let Some(id) = made.get(&key) {
                    *id
                } else {
                    let id = ProjectId::from_uuid(Uuid::new_v4());
                    add_project(
                        tx,
                        &NewProject {
                            id,
                            owner,
                            name: None,
                            origin: Origin::Added,
                            repo: Some(repo),
                            environment_id: environment,
                            created_at: now,
                        },
                    )?;
                    made.insert(key, id);
                    id
                }
            }
        };
        tx.execute(
            "UPDATE workspace SET project_id = ?2 WHERE id = ?1",
            params![workspace.to_string(), project.to_string()],
        )?;
    }
    Ok(())
}

/// Whether a workspace existed and has been deleted, as opposed to one this
/// database has never heard of.
pub fn is_deleted(tx: &Connection, id: WorkspaceId) -> Result<bool, DbError> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM workspace WHERE id = ?1 AND deleted_at IS NOT NULL",
            [id.to_string()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Sets the desired state if the revision still matches `expected`.
/// Returns the new revision, or `None` on a revision conflict.
pub fn set_desired(
    tx: &Connection,
    id: WorkspaceId,
    desired: DesiredState,
    expected: Revision,
) -> Result<Option<Revision>, DbError> {
    let current: Option<i64> = tx
        .query_row(
            "SELECT revision FROM workspace WHERE id = ?1 AND deleted_at IS NULL",
            [id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(current) = current
        .and_then(|r| u64::try_from(r).ok())
        .map(Revision::from_u64)
    else {
        return Ok(None);
    };
    if expected != current {
        return Ok(None);
    }
    let next = current.next();
    tx.execute(
        "UPDATE workspace SET desired = ?2, revision = ?3, condition = NULL WHERE id = ?1",
        params![id.to_string(), desired.as_str(), i64_of(next.get())],
    )?;
    Ok(Some(next))
}

/// Records what the host reported at `at`, unless something newer is already
/// recorded, such as the outcome of a command that finished after the report
/// was asked for.
pub fn record_observation(
    tx: &Connection,
    id: WorkspaceId,
    instance: &Instance,
    memory: Option<Bytes>,
    at: Timestamp,
) -> Result<(), DbError> {
    tx.execute(
        "UPDATE workspace SET observed = ?2, observed_at = ?3, memory = ?4
         WHERE id = ?1 AND (observed_at IS NULL OR observed_at <= ?3)",
        params![
            id.to_string(),
            to_json(instance)?,
            at.unix_millis(),
            memory.map(|m| i64_of(m.get()))
        ],
    )?;
    Ok(())
}

pub fn set_condition(
    tx: &Connection,
    id: WorkspaceId,
    condition: Option<&Condition>,
) -> Result<(), DbError> {
    let json = condition.map(to_json).transpose()?;
    tx.execute(
        "UPDATE workspace SET condition = ?2 WHERE id = ?1",
        params![id.to_string(), json],
    )?;
    Ok(())
}

/// Marks a workspace deleted and removes its routes; their names stay taken.
pub fn mark_deleted(tx: &Connection, id: WorkspaceId, now: Timestamp) -> Result<(), DbError> {
    tx.execute(
        "DELETE FROM route WHERE workspace_id = ?1",
        [id.to_string()],
    )?;
    tx.execute(
        "DELETE FROM attention WHERE workspace_id = ?1",
        [id.to_string()],
    )?;
    tx.execute(
        "UPDATE workspace SET deleted_at = ?2 WHERE id = ?1",
        params![id.to_string(), now.unix_millis()],
    )?;
    Ok(())
}

// ---- attention ----

/// Replaces a workspace's thread statuses with what the guest reports now.
/// A status whose timestamp changed becomes unseen again.
pub fn sync_attention(
    tx: &Connection,
    id: WorkspaceId,
    statuses: &[SessionStatus],
) -> Result<bool, DbError> {
    let before: Vec<(String, String, i64)> = {
        let mut statement = tx
            .prepare("SELECT session, thread, updated_at FROM attention WHERE workspace_id = ?1")?;
        statement
            .query_map([id.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<_, _>>()?
    };
    let mut before = before;
    before.sort();
    let mut after: Vec<(String, String, i64)> = statuses
        .iter()
        .map(|s| {
            (
                s.session.to_string(),
                s.thread.to_string(),
                s.updated_at.unix_millis(),
            )
        })
        .collect();
    after.sort();
    if before == after {
        return Ok(false);
    }
    let current: Vec<(&str, &str)> = statuses
        .iter()
        .map(|s| (s.session.as_str(), s.thread.as_str()))
        .collect();
    tx.execute(
        "DELETE FROM attention WHERE workspace_id = ?1 AND NOT EXISTS (
           SELECT 1 FROM json_each(?2) WHERE json_extract(value, '$[0]') = session AND json_extract(value, '$[1]') = thread)",
        params![id.to_string(), to_json(&current)?],
    )?;
    for status in statuses {
        tx.execute(
            "INSERT INTO attention (workspace_id, session, thread, title, state, summary, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (workspace_id, session, thread) DO UPDATE SET
               title = excluded.title, state = excluded.state, summary = excluded.summary,
               updated_at = excluded.updated_at",
            params![
                id.to_string(),
                status.session.as_str(),
                status.thread.as_str(),
                status.title.as_str(),
                status.state.as_str(),
                status.summary.as_str(),
                status.updated_at.unix_millis()
            ],
        )?;
    }
    Ok(true)
}

pub fn attention(tx: &Connection, id: WorkspaceId) -> Result<Vec<AttentionRecord>, DbError> {
    let mut statement = tx.prepare(
        "SELECT session, thread, title, state, summary, updated_at, seen_at FROM attention
         WHERE workspace_id = ?1 ORDER BY session, thread",
    )?;
    Ok(statement
        .query_map([id.to_string()], |row| {
            let updated_at = timestamp(row, 5)?;
            let seen_at = timestamp(row, 6)?;
            Ok(AttentionRecord {
                status: SessionStatus {
                    session: text(row, 0)?,
                    thread: text(row, 1)?,
                    title: text(row, 2)?,
                    state: text(row, 3)?,
                    summary: text(row, 4)?,
                    updated_at,
                },
                seen: if seen_at >= updated_at {
                    Seen::Seen
                } else {
                    Seen::Unseen
                },
            })
        })?
        .collect::<Result<_, _>>()?)
}

pub fn mark_seen(tx: &Connection, id: WorkspaceId, now: Timestamp) -> Result<(), DbError> {
    tx.execute(
        "UPDATE attention SET seen_at = ?2 WHERE workspace_id = ?1",
        params![id.to_string(), now.unix_millis()],
    )?;
    Ok(())
}

// ---- columns ----

fn column_row(row: &Row<'_>) -> rusqlite::Result<ColumnRecord> {
    Ok(ColumnRecord {
        spec: ColumnSpec {
            name: text(row, 0)?,
            kind: json(row, 1)?,
            width: text(row, 2)?,
        },
        prompt: opt_text(row, 3)?,
    })
}

/// A workspace's columns, in order.
pub fn columns(tx: &Connection, workspace: WorkspaceId) -> Result<Vec<ColumnRecord>, DbError> {
    let mut statement = tx.prepare(
        "SELECT name, kind, width, prompt FROM workspace_column WHERE workspace_id = ?1 ORDER BY position",
    )?;
    Ok(statement
        .query_map([workspace.to_string()], column_row)?
        .collect::<Result<_, _>>()?)
}

/// Replaces a workspace's columns with `columns`, in that order.
pub fn replace_columns(
    tx: &Connection,
    workspace: WorkspaceId,
    columns: &[ColumnRecord],
) -> Result<(), DbError> {
    tx.execute(
        "DELETE FROM workspace_column WHERE workspace_id = ?1",
        [workspace.to_string()],
    )?;
    let mut insert = tx.prepare(
        "INSERT INTO workspace_column (workspace_id, position, name, kind, width, prompt)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (position, column) in columns.iter().enumerate() {
        insert.execute(params![
            workspace.to_string(),
            i64::try_from(position).unwrap_or(i64::MAX),
            column.spec.name.as_str(),
            to_json(&column.spec.kind)?,
            column.spec.width.as_str(),
            column.prompt.as_ref().map(Prompt::as_str),
        ])?;
    }
    Ok(())
}

/// Forgets the prompts of the columns whose sessions opened: a restart
/// mustn't hand an agent its task again. A column that didn't open keeps
/// its prompt for the next try.
pub fn clear_prompts(
    tx: &Connection,
    workspace: WorkspaceId,
    opened: &[SessionName],
) -> Result<(), DbError> {
    let names: Vec<&str> = opened.iter().map(SessionName::as_str).collect();
    tx.execute(
        "UPDATE workspace_column SET prompt = NULL
         WHERE workspace_id = ?1 AND name IN (SELECT value FROM json_each(?2))",
        params![workspace.to_string(), to_json(&names)?],
    )?;
    Ok(())
}

// ---- routes ----

fn route_row(row: &Row<'_>) -> rusqlite::Result<RouteRecord> {
    let port: i64 = row.get(3)?;
    let port = u16::try_from(port).map_err(|e| conversion(3, e))?;
    Ok(RouteRecord {
        id: text(row, 0)?,
        workspace: text(row, 1)?,
        name: text(row, 2)?,
        port: GuestPort::try_from(port).map_err(|e| conversion(3, e))?,
    })
}

#[derive(Debug)]
pub enum NewRoute {
    Created(RouteRecord),
    Exists(RouteRecord),
    NameTaken,
}

/// Publishes a port under a fresh name. Publishing the same port twice returns the existing route.
pub fn create_route(
    tx: &Connection,
    id: RouteId,
    workspace: WorkspaceId,
    owner: PrincipalId,
    name: &RouteName,
    port: GuestPort,
    now: Timestamp,
) -> Result<NewRoute, DbError> {
    if let Some(existing) = tx
        .query_row(
            "SELECT id, workspace_id, name, port FROM route WHERE workspace_id = ?1 AND port = ?2",
            params![workspace.to_string(), i64::from(port.get())],
            route_row,
        )
        .optional()?
    {
        return Ok(NewRoute::Exists(existing));
    }
    let inserted = tx.execute(
        "INSERT INTO route_name (name, owner_id) VALUES (?1, ?2) ON CONFLICT (name) DO NOTHING",
        params![name.as_str(), owner.to_string()],
    )?;
    if inserted == 0 {
        return Ok(NewRoute::NameTaken);
    }
    tx.execute(
        "INSERT INTO route (id, workspace_id, name, port, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            id.to_string(),
            workspace.to_string(),
            name.as_str(),
            i64::from(port.get()),
            now.unix_millis()
        ],
    )?;
    Ok(NewRoute::Created(RouteRecord {
        id,
        workspace,
        name: name.clone(),
        port,
    }))
}

pub fn routes(tx: &Connection, workspace: WorkspaceId) -> Result<Vec<RouteRecord>, DbError> {
    let mut statement = tx.prepare(
        "SELECT id, workspace_id, name, port FROM route WHERE workspace_id = ?1 ORDER BY port",
    )?;
    Ok(statement
        .query_map([workspace.to_string()], route_row)?
        .collect::<Result<_, _>>()?)
}

pub fn route_by_name(tx: &Connection, name: &RouteName) -> Result<Option<RouteRecord>, DbError> {
    Ok(tx
        .query_row(
            "SELECT id, workspace_id, name, port FROM route WHERE name = ?1",
            [name.as_str()],
            route_row,
        )
        .optional()?)
}

pub fn delete_route(tx: &Connection, workspace: WorkspaceId, id: RouteId) -> Result<bool, DbError> {
    let deleted = tx.execute(
        "DELETE FROM route WHERE id = ?1 AND workspace_id = ?2",
        params![id.to_string(), workspace.to_string()],
    )?;
    Ok(deleted > 0)
}

// ---- secrets ----

pub struct SealedSecret {
    pub name: SecretName,
    pub target: SecretTarget,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

pub fn put_secret(
    tx: &Connection,
    id: SecretId,
    owner: PrincipalId,
    sealed: &SealedSecret,
    now: Timestamp,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO secret (id, owner_id, name, target, nonce, ciphertext, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (owner_id, name) DO UPDATE SET
           target = excluded.target, nonce = excluded.nonce, ciphertext = excluded.ciphertext, updated_at = excluded.updated_at",
        params![id.to_string(), owner.to_string(), sealed.name.as_str(), to_json(&sealed.target)?, sealed.nonce, sealed.ciphertext, now.unix_millis()],
    )?;
    bump_secrets_generation(tx, owner)
}

/// Where the owner's other secrets go, to refuse a second one for the same place.
pub fn other_secret_targets(
    tx: &Connection,
    owner: PrincipalId,
    name: &SecretName,
) -> Result<Vec<(SecretName, SecretTarget)>, DbError> {
    let mut statement =
        tx.prepare("SELECT name, target FROM secret WHERE owner_id = ?1 AND name != ?2")?;
    Ok(statement
        .query_map(params![owner.to_string(), name.as_str()], |row| {
            Ok((
                text(row, 0)?,
                opt_json(row, 1)?
                    .ok_or_else(|| conversion(1, std::io::Error::other("missing target")))?,
            ))
        })?
        .collect::<Result<_, _>>()?)
}

pub fn delete_secret(
    tx: &Connection,
    owner: PrincipalId,
    name: &SecretName,
) -> Result<bool, DbError> {
    let deleted = tx.execute(
        "DELETE FROM secret WHERE owner_id = ?1 AND name = ?2",
        params![owner.to_string(), name.as_str()],
    )?;
    if deleted > 0 {
        bump_secrets_generation(tx, owner)?;
    }
    Ok(deleted > 0)
}

pub fn secrets(tx: &Connection, owner: PrincipalId) -> Result<Vec<SecretView>, DbError> {
    let mut statement = tx.prepare(
        "SELECT id, name, target, updated_at FROM secret WHERE owner_id = ?1 ORDER BY name",
    )?;
    Ok(statement
        .query_map([owner.to_string()], |row| {
            Ok(SecretView {
                id: text(row, 0)?,
                name: text(row, 1)?,
                target: opt_json(row, 2)?
                    .ok_or_else(|| conversion(2, std::io::Error::other("missing target")))?,
                updated_at: timestamp(row, 3)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

pub fn sealed_secrets(tx: &Connection, owner: PrincipalId) -> Result<Vec<SealedSecret>, DbError> {
    let mut statement = tx.prepare(
        "SELECT name, target, nonce, ciphertext FROM secret WHERE owner_id = ?1 ORDER BY name",
    )?;
    Ok(statement
        .query_map([owner.to_string()], |row| {
            Ok(SealedSecret {
                name: text(row, 0)?,
                target: opt_json(row, 1)?
                    .ok_or_else(|| conversion(1, std::io::Error::other("missing target")))?,
                nonce: row.get(2)?,
                ciphertext: row.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

// ---- activity ----

pub fn add_activity(
    tx: &Connection,
    workspace: Option<WorkspaceId>,
    actor: Option<PrincipalId>,
    kind: &str,
    detail: &str,
    now: Timestamp,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO activity (workspace_id, actor_id, kind, detail, at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![workspace.map(|w| w.to_string()), actor.map(|a| a.to_string()), kind, detail, now.unix_millis()],
    )?;
    Ok(())
}

pub fn activity(
    tx: &Connection,
    workspace: WorkspaceId,
    limit: u32,
) -> Result<Vec<ActivityEntry>, DbError> {
    let mut statement =
        tx.prepare("SELECT kind, detail, at FROM activity WHERE workspace_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2")?;
    Ok(statement
        .query_map(params![workspace.to_string(), limit], |row| {
            Ok(ActivityEntry {
                kind: row.get(0)?,
                detail: row.get(1)?,
                at: timestamp(row, 2)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

// Real SQLite, in memory: the queries and schema constraints are what's
// under test, so nothing here is faked.
#[cfg(test)]
mod tests {
    use iglu_domain::attention::{AttentionState, Summary};
    use iglu_domain::auth::{Email, Issuer, Subject};
    use iglu_domain::terminal::SessionName;

    use super::*;

    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory SQLite opens");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("foreign keys switch on");
        migrate(&mut conn).expect("the migrations apply");
        conn
    }

    fn at(millis: i64) -> Timestamp {
        Timestamp::from_unix_millis(millis)
    }

    fn identity(subject: &str, email: &str) -> VerifiedIdentity {
        VerifiedIdentity {
            issuer: "https://id.example.org".parse::<Issuer>().expect("issuer"),
            subject: subject.parse::<Subject>().expect("subject"),
            email: Some(email.parse::<Email>().expect("email")),
            name: None,
        }
    }

    fn principal(conn: &Connection, subject: &str) -> PrincipalId {
        upsert_principal(
            conn,
            PrincipalId::from_uuid(Uuid::new_v4()),
            &identity(subject, "someone@example.org"),
            at(1),
        )
        .expect("principal")
        .id
    }

    fn revision(conn: &Connection, owner: PrincipalId) -> EnvRevisionId {
        let env = Uuid::new_v4();
        let name: EnvName = "default".parse().expect("env name");
        let source: EnvSource = "github:acme/env#default".parse().expect("source");
        create_environment(conn, env, owner, &name, &source, at(1)).expect("environment");
        ensure_builtin(
            conn,
            owner,
            env,
            ProjectId::from_uuid(Uuid::new_v4()),
            at(1),
        )
        .expect("built-in project");
        let id = EnvRevisionId::from_uuid(Uuid::new_v4());
        insert_revision(conn, id, env, at(1)).expect("revision");
        id
    }

    fn workspace_record(
        owner: PrincipalId,
        env: EnvRevisionId,
        project: ProjectId,
        name: &str,
    ) -> WorkspaceRecord {
        WorkspaceRecord {
            id: WorkspaceId::from_uuid(Uuid::new_v4()),
            owner,
            host: "host-1".parse().expect("host"),
            env_revision: env,
            name: name.parse().expect("workspace name"),
            project,
            checkout: Some(Checkout {
                repo: "https://github.com/acme/app.git".parse().expect("repo"),
                branch: name.parse().expect("branch"),
                base: None,
            }),
            desired: DesiredState::Running,
            revision: Revision::from_u64(1),
            observed: None,
            observed_at: None,
            memory: None,
            condition: None,
            created_at: at(1),
        }
    }

    fn new_workspace(conn: &Connection, owner: PrincipalId, name: &str) -> WorkspaceRecord {
        let env = revision(conn, owner);
        let project = projects(conn, owner).expect("projects")[0].id;
        let ws = workspace_record(owner, env, project, name);
        insert_workspace(conn, &ws, None, "hash").expect("workspace");
        ws
    }

    #[test]
    fn migrations_apply_once_and_refuse_a_newer_schema() {
        let mut conn = conn();
        migrate(&mut conn).expect("applying again is a no-op");
        conn.pragma_update(None, "user_version", 99)
            .expect("set version");
        assert!(matches!(
            migrate(&mut conn),
            Err(DbError::TooNew { found: 99 })
        ));
    }

    #[test]
    fn a_principal_is_keyed_by_issuer_and_subject() {
        let conn = conn();
        let first = upsert_principal(
            &conn,
            PrincipalId::from_uuid(Uuid::new_v4()),
            &identity("alice", "alice@example.org"),
            at(1),
        )
        .expect("insert");
        let again = upsert_principal(
            &conn,
            PrincipalId::from_uuid(Uuid::new_v4()),
            &identity("alice", "alice@new.example.org"),
            at(2),
        )
        .expect("update");
        assert_eq!(first.id, again.id);
        assert_eq!(
            again.email.as_ref().map(Email::as_str),
            Some("alice@new.example.org")
        );
        let bob = principal(&conn, "bob");
        assert_ne!(bob, first.id);
    }

    #[test]
    fn logins_and_cli_codes_work_once() {
        let conn = conn();
        let login = LoginRow {
            kind: SessionKind::Preview,
            nonce: "n".into(),
            pkce_verifier: "v".into(),
            return_to: "/".into(),
        };
        insert_login(&conn, "state", &login, at(1)).expect("login");
        let taken = take_login(&conn, "state").expect("take");
        assert_eq!(taken.map(|l| l.kind), Some(SessionKind::Preview));
        assert!(take_login(&conn, "state").expect("take").is_none());

        let owner = principal(&conn, "alice");
        insert_cli_code(&conn, "fresh", owner, "challenge", at(100)).expect("code");
        insert_cli_code(&conn, "stale", owner, "challenge", at(10)).expect("code");
        assert!(
            take_cli_code(&conn, "stale", at(50))
                .expect("take")
                .is_none()
        );
        let fresh = take_cli_code(&conn, "fresh", at(50)).expect("take");
        assert_eq!(fresh, Some((owner, "challenge".to_owned())));
        assert!(
            take_cli_code(&conn, "fresh", at(50))
                .expect("take")
                .is_none()
        );
    }

    #[test]
    fn a_name_is_taken_while_its_workspace_is_live() {
        let conn = conn();
        let owner = principal(&conn, "alice");
        let ws = new_workspace(&conn, owner, "demo");
        assert!(workspace_name_taken(&conn, owner, "demo").expect("query"));
        let duplicate = workspace_record(owner, ws.env_revision, ws.project, "demo");
        assert!(insert_workspace(&conn, &duplicate, None, "hash").is_err());
        let other = principal(&conn, "bob");
        assert!(!workspace_name_taken(&conn, other, "demo").expect("query"));

        mark_deleted(&conn, ws.id, at(5)).expect("delete");
        assert!(workspace(&conn, ws.id).expect("query").is_none());
        assert!(!workspace_name_taken(&conn, owner, "demo").expect("query"));
        insert_workspace(&conn, &duplicate, None, "hash").expect("the name is free again");
    }

    #[test]
    fn desired_state_changes_only_from_the_expected_revision() {
        let conn = conn();
        let owner = principal(&conn, "alice");
        let ws = new_workspace(&conn, owner, "demo");
        let stale = Revision::from_u64(7);
        assert_eq!(
            set_desired(&conn, ws.id, DesiredState::Frozen, stale).expect("query"),
            None
        );
        let next = set_desired(&conn, ws.id, DesiredState::Frozen, ws.revision)
            .expect("query")
            .expect("the revision matched");
        assert_eq!(next, ws.revision.next());
        let stored = workspace(&conn, ws.id).expect("query").expect("live");
        assert_eq!(stored.desired, DesiredState::Frozen);
    }

    #[test]
    fn route_names_are_never_reused() {
        let conn = conn();
        let owner = principal(&conn, "alice");
        let ws = new_workspace(&conn, owner, "demo");
        let name: RouteName = "brave-hopping".parse().expect("route name");
        let port: GuestPort = "3000".parse().expect("port");
        let created = match create_route(
            &conn,
            RouteId::from_uuid(Uuid::new_v4()),
            ws.id,
            owner,
            &name,
            port,
            at(1),
        )
        .expect("route")
        {
            NewRoute::Created(route) => route,
            NewRoute::Exists(_) | NewRoute::NameTaken => panic!("a fresh name is created"),
        };
        assert!(matches!(
            create_route(&conn, RouteId::from_uuid(Uuid::new_v4()), ws.id, owner, &name, port, at(2)),
            Ok(NewRoute::Exists(route)) if route.id == created.id
        ));
        assert!(delete_route(&conn, ws.id, created.id).expect("delete"));

        let other = workspace_record(owner, ws.env_revision, ws.project, "other");
        insert_workspace(&conn, &other, None, "hash").expect("workspace");
        assert!(matches!(
            create_route(
                &conn,
                RouteId::from_uuid(Uuid::new_v4()),
                other.id,
                owner,
                &name,
                port,
                at(4)
            ),
            Ok(NewRoute::NameTaken)
        ));
    }

    #[test]
    fn attention_sync_reports_only_changes() {
        let conn = conn();
        let owner = principal(&conn, "alice");
        let ws = new_workspace(&conn, owner, "demo");
        let thread = |key: &str, state, ms| SessionStatus {
            session: "claude".parse::<SessionName>().expect("session"),
            thread: key.parse().expect("thread"),
            title: Summary::sanitize("fix login"),
            state,
            summary: Summary::sanitize("approve Bash?"),
            updated_at: at(ms),
        };
        let both = vec![
            thread("one", AttentionState::Waiting, 10),
            thread("two", AttentionState::Working, 11),
        ];
        assert!(sync_attention(&conn, ws.id, &both).expect("sync"));
        assert!(!sync_attention(&conn, ws.id, &both).expect("sync"));
        assert_eq!(attention(&conn, ws.id).expect("read").len(), 2);
        let one = vec![thread("two", AttentionState::Done, 12)];
        assert!(sync_attention(&conn, ws.id, &one).expect("sync"));
        let left = attention(&conn, ws.id).expect("read");
        assert_eq!(
            left.iter()
                .map(|r| (r.status.thread.as_str(), r.status.state))
                .collect::<Vec<_>>(),
            [("two", AttentionState::Done)]
        );
        assert!(sync_attention(&conn, ws.id, &[]).expect("sync"));
    }

    #[test]
    fn existing_workspaces_move_into_projects() {
        let mut conn = Connection::open_in_memory().expect("in-memory SQLite opens");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("foreign keys switch on");
        migrate_until(&mut conn, 2).expect("the schema before projects");
        let alice = principal(&conn, "alice");
        let bob = principal(&conn, "bob");
        let env = Uuid::new_v4();
        let name: EnvName = "default".parse().expect("env name");
        let source: EnvSource = "github:acme/env#default".parse().expect("source");
        create_environment(&conn, env, alice, &name, &source, at(1)).expect("environment");
        let revision = EnvRevisionId::from_uuid(Uuid::new_v4());
        insert_revision(&conn, revision, env, at(1)).expect("revision");
        let old = |name: &str, repo: &str, deleted: Option<i64>| {
            let id = WorkspaceId::from_uuid(Uuid::new_v4());
            conn.execute(
                "INSERT INTO workspace (id, owner_id, host_id, env_revision_id, name, repo, branch, base,
                                        desired, revision, created_at, deleted_at)
                 VALUES (?1, ?2, 'host-1', ?3, ?4, ?5, ?4, 'main', 'running', 1, 1, ?6)",
                params![id.to_string(), alice.to_string(), revision.to_string(), name, repo, deleted],
            )
            .expect("an old workspace");
            id
        };
        let first = old("first", "https://github.com/acme/app.git", None);
        let second = old("second", "https://github.com/acme/app.git", None);
        let other = old("other", "git@github.com:acme/site.git", None);
        let gone = old("gone", "https://github.com/acme/old.git", Some(5));
        migrate(&mut conn).expect("the rest apply");

        let projects = projects(&conn, alice).expect("projects");
        let names: Vec<(&str, Origin)> = projects
            .iter()
            .map(|p| (p.name.as_str(), p.origin))
            .collect();
        assert_eq!(
            names,
            [
                ("app", Origin::Added),
                ("general", Origin::Builtin),
                ("site", Origin::Added)
            ]
        );
        let id_of = |name: &str| {
            projects
                .iter()
                .find(|p| p.name.as_str() == name)
                .expect("the project")
                .id
        };
        for (ws, project) in [(first, "app"), (second, "app"), (other, "site")] {
            let ws = workspace(&conn, ws).expect("query").expect("still live");
            assert_eq!(ws.project, id_of(project));
            let checkout = ws.checkout.expect("the checkout carried over");
            assert_eq!(
                (
                    checkout.branch.as_str(),
                    checkout.base.map(|b| b.as_str().to_owned())
                ),
                (ws.name.as_str(), Some("main".into()))
            );
        }
        let parked: String = conn
            .query_row(
                "SELECT project_id FROM workspace WHERE id = ?1",
                [gone.to_string()],
                |row| row.get(0),
            )
            .expect("the deleted workspace");
        assert_eq!(parked, id_of("general").to_string());
        assert!(
            super::projects(&conn, bob).expect("projects").is_empty(),
            "bob has no environment yet"
        );
    }

    #[test]
    fn only_added_projects_without_live_workspaces_can_be_removed() {
        let conn = conn();
        let owner = principal(&conn, "alice");
        let ws = new_workspace(&conn, owner, "demo");
        let env = projects(&conn, owner).expect("projects")[0].environment_id;
        assert_eq!(
            remove_project(&conn, owner, ws.project).expect("query"),
            RemoveOutcome::Builtin
        );
        let app = ProjectId::from_uuid(Uuid::new_v4());
        let new = NewProject {
            id: app,
            owner,
            name: None,
            origin: Origin::Added,
            repo: Some("https://github.com/acme/app.git".parse().expect("repo")),
            environment_id: env,
            created_at: at(2),
        };
        assert!(matches!(
            add_project(&conn, &new).expect("added"),
            AddOutcome::Added
        ));
        let again = NewProject {
            id: ProjectId::from_uuid(Uuid::new_v4()),
            name: Some("app".parse().expect("name")),
            ..new
        };
        assert!(matches!(
            add_project(&conn, &again).expect("query"),
            AddOutcome::NameTaken
        ));
        let inside = WorkspaceRecord {
            project: app,
            ..workspace_record(owner, ws.env_revision, app, "inside")
        };
        insert_workspace(&conn, &inside, None, "hash").expect("a workspace in it");
        assert_eq!(
            remove_project(&conn, owner, app).expect("query"),
            RemoveOutcome::InUse
        );
        conn.execute(
            "UPDATE workspace SET deleted_at = 9 WHERE id = ?1",
            [inside.id.to_string()],
        )
        .expect("deleted");
        assert_eq!(
            remove_project(&conn, owner, app).expect("query"),
            RemoveOutcome::Done
        );
        assert_eq!(
            remove_project(&conn, owner, app).expect("query"),
            RemoveOutcome::Missing
        );
    }

    #[test]
    fn a_change_applies_only_to_the_revision_it_saw() {
        let conn = conn();
        let owner = principal(&conn, "alice");
        let ws = new_workspace(&conn, owner, "demo");
        let general = project(&conn, owner, ws.project)
            .expect("query")
            .expect("the project");
        let change = ProjectChange {
            name: "notes".parse().expect("name"),
            repo: None,
            environment_id: general.environment_id,
            opening: Opening::shell(),
            agent: None,
            ports: PreviewPorts::default(),
            idle: IdleRule::Never,
        };
        assert_eq!(
            change_project(&conn, owner, ws.project, general.revision, &change).expect("query"),
            ChangeOutcome::Done
        );
        assert_eq!(
            change_project(&conn, owner, ws.project, general.revision, &change).expect("query"),
            ChangeOutcome::Stale
        );
        let renamed = project(&conn, owner, ws.project)
            .expect("query")
            .expect("the project");
        assert_eq!(
            (renamed.name.as_str(), renamed.revision),
            ("notes", general.revision.next())
        );
    }

    #[test]
    fn only_columns_that_opened_forget_their_prompts() {
        let conn = conn();
        let owner = principal(&conn, "alice");
        let ws = new_workspace(&conn, owner, "demo");
        let column = |name: &str| ColumnRecord {
            spec: ColumnSpec {
                name: name.parse().expect("a session"),
                kind: iglu_domain::column::ColumnKind::Shell,
                width: iglu_domain::column::ColumnWidth::Half,
            },
            prompt: Some("fix the login bug".parse().expect("a prompt")),
        };
        replace_columns(&conn, ws.id, &[column("opened"), column("failed")]).expect("columns");
        clear_prompts(&conn, ws.id, &["opened".parse().expect("a session")]).expect("cleared");
        let prompts: Vec<(String, bool)> = columns(&conn, ws.id)
            .expect("columns")
            .into_iter()
            .map(|c| (c.spec.name.as_str().to_owned(), c.prompt.is_some()))
            .collect();
        assert_eq!(prompts, [("opened".into(), false), ("failed".into(), true)]);
    }

    #[test]
    fn rows_written_before_columns_and_agents_still_read() {
        let mut conn = Connection::open_in_memory().expect("in-memory SQLite opens");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("foreign keys switch on");
        migrate_until(&mut conn, 1).expect("the 0.2 schema");
        let owner = principal(&conn, "alice");
        let env = Uuid::new_v4();
        let name: EnvName = "default".parse().expect("env name");
        let source: EnvSource = "github:acme/env#default".parse().expect("source");
        create_environment(&conn, env, owner, &name, &source, at(1)).expect("environment");
        let revision = EnvRevisionId::from_uuid(Uuid::new_v4());
        insert_revision(&conn, revision, env, at(1)).expect("revision");
        // As 0.2 wrote them: no agents in the image, no columns in a running instance.
        conn.execute(
            "UPDATE env_revision SET status = 'ready', built = ?1",
            [r#"{"fingerprint":"0000000000000000000000000000000000000000000000000000000000000000","arch":"x86_64","user":{"name":"dev","uid":1000,"gid":100,"home":"/home/dev"},"store_path":"/nix/store/x"}"#],
        )
        .expect("an old image");
        let id = WorkspaceId::from_uuid(Uuid::new_v4());
        conn.execute(
            "INSERT INTO workspace (id, owner_id, host_id, env_revision_id, name, repo, branch, desired,
                                    revision, observed, observed_at, created_at)
             VALUES (?1, ?2, 'host-1', ?3, 'old', 'https://github.com/acme/app.git', 'old', 'running', 1, ?4, 5, 1)",
            params![
                id.to_string(),
                owner.to_string(),
                revision.to_string(),
                r#"{"kind":"present","runtime":{"status":"running","readiness":"network","secrets":1},"provisioning":"complete"}"#,
            ],
        )
        .expect("an old workspace");
        migrate(&mut conn).expect("the rest apply");
        let live = live_workspaces(&conn).expect("old rows read");
        assert_eq!(live.len(), 1);
        assert!(
            live[0].observed.is_none(),
            "observed again on the next tick"
        );
        let image = revision_image(&conn, revision)
            .expect("an old image reads")
            .expect("ready");
        assert!(image.agents.is_empty());
    }

    #[test]
    fn renaming_keeps_names_unique_among_live_workspaces() {
        let conn = conn();
        let owner = principal(&conn, "alice");
        let env = revision(&conn, owner);
        let project = projects(&conn, owner).expect("projects")[0].id;
        let first = workspace_record(owner, env, project, "first");
        let second = workspace_record(owner, env, project, "second");
        insert_workspace(&conn, &first, None, "a").expect("first");
        insert_workspace(&conn, &second, None, "b").expect("second");

        let taken: WorkspaceName = "first".parse().expect("name");
        assert_eq!(
            rename_workspace(&conn, second.id, owner, &taken).expect("rename"),
            RenameOutcome::NameTaken
        );
        let same = rename_workspace(&conn, first.id, owner, &taken).expect("rename to itself");
        assert_eq!(same, RenameOutcome::Done);

        let fresh: WorkspaceName = "palette".parse().expect("name");
        assert_eq!(
            rename_workspace(&conn, second.id, owner, &fresh).expect("rename"),
            RenameOutcome::Done
        );
        let renamed = workspace(&conn, second.id).expect("read").expect("live");
        assert_eq!(renamed.name, fresh);

        mark_deleted(&conn, first.id, at(2)).expect("delete");
        assert_eq!(
            rename_workspace(&conn, second.id, owner, &taken)
                .expect("a deleted workspace's name is free"),
            RenameOutcome::Done
        );
        assert_eq!(
            rename_workspace(&conn, first.id, owner, &fresh).expect("rename"),
            RenameOutcome::Missing
        );

        let bob = principal(&conn, "bob");
        let bobs = revision(&conn, bob);
        let theirs = workspace_record(
            bob,
            bobs,
            projects(&conn, bob).expect("projects")[0].id,
            "theirs",
        );
        insert_workspace(&conn, &theirs, None, "c").expect("bob's");
        assert_eq!(
            rename_workspace(&conn, theirs.id, bob, &taken).expect("names are per owner"),
            RenameOutcome::Done
        );
    }
}
