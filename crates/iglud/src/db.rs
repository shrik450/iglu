//! SQLite storage. Every row is parsed into domain types on the way out, so
//! nothing past this module handles raw column text.
//!
//! One connection, used from blocking threads: iglud is a single process and
//! SQLite serializes writes anyway.

use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use iglu_domain::attention::{Seen, SessionStatus};
use iglu_domain::auth::{PrincipalStatus, VerifiedIdentity};
use iglu_domain::capacity::Bytes;
use iglu_domain::env::{BuiltImage, EnvName, EnvSource};
use iglu_domain::id::{EnvRevisionId, PrincipalId, RouteId, SecretId, WorkspaceId};
use iglu_domain::label::RouteName;
use iglu_domain::lifecycle::{DesiredState, Instance, Revision, SecretsGeneration};
use iglu_domain::port::GuestPort;
use iglu_domain::secret::{SecretName, SecretTarget};
use iglu_domain::time::Timestamp;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::model::{
    AttentionRecord, Condition, EnvironmentRecord, PrincipalRecord, RevisionRecord, RevisionStatus,
    RouteRecord, SecretSummary, WorkspaceRecord,
};

/// The schema's history, oldest first. `user_version` records how many have
/// been applied. Never edit one that has shipped; add another.
const MIGRATIONS: &[&str] = &[include_str!("migrations/0001_initial.sql")];

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
    let raw: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let applied = usize::try_from(raw).unwrap_or(usize::MAX);
    if applied > MIGRATIONS.len() {
        return Err(DbError::TooNew { found: applied });
    }
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(migration)?;
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
    let status: String = row.get(3)?;
    Ok(PrincipalRecord {
        id: text(row, 0)?,
        email: opt_text(row, 1)?,
        name: opt_text(row, 2)?,
        status: if status == "disabled" {
            PrincipalStatus::Disabled
        } else {
            PrincipalStatus::Active
        },
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
                let kind: String = row.get(0)?;
                Ok(LoginRow {
                    kind: if kind == "preview" {
                        SessionKind::Preview
                    } else {
                        SessionKind::Console
                    },
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
) -> Result<Vec<(EnvironmentRecord, Option<RevisionRecord>)>, DbError> {
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

fn revision_row(row: &Row<'_>) -> rusqlite::Result<RevisionRecord> {
    let status: String = row.get(1)?;
    let status = match status.as_str() {
        "ready" => RevisionStatus::Ready {
            image: opt_json::<BuiltImage>(row, 2)?
                .ok_or_else(|| conversion(2, std::io::Error::other("missing image")))?,
        },
        "failed" => RevisionStatus::Failed {
            log_tail: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        },
        _ => RevisionStatus::Building,
    };
    Ok(RevisionRecord {
        id: text(row, 0)?,
        status,
        created_at: timestamp(row, 4)?,
    })
}

fn latest_revision(tx: &Connection, env: Uuid) -> Result<Option<RevisionRecord>, DbError> {
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

const WORKSPACE_COLUMNS: &str =
    "id, owner_id, host_id, env_revision_id, name, repo, branch, base, desired, revision,
    observed, observed_at, memory, condition, created_at";

fn workspace_row(row: &Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    let memory: Option<i64> = row.get(12)?;
    Ok(WorkspaceRecord {
        id: text(row, 0)?,
        owner: text(row, 1)?,
        host: text(row, 2)?,
        env_revision: text(row, 3)?,
        name: text(row, 4)?,
        repo: text(row, 5)?,
        branch: text(row, 6)?,
        base: opt_text(row, 7)?,
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
        "INSERT INTO workspace (id, owner_id, host_id, env_revision_id, name, repo, branch, base, desired, revision,
                                create_key, create_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            ws.id.to_string(),
            ws.owner.to_string(),
            ws.host.as_str(),
            ws.env_revision.to_string(),
            ws.name.as_str(),
            ws.repo.as_str(),
            ws.branch.as_str(),
            ws.base.as_ref().map(iglu_domain::repo::BranchName::as_str),
            ws.desired.as_str(),
            i64_of(ws.revision.get()),
            create_key,
            create_hash,
            ws.created_at.unix_millis(),
        ],
    )?;
    Ok(())
}

pub fn workspace_by_create_key(
    tx: &Connection,
    owner: PrincipalId,
    key: &str,
) -> Result<Option<(WorkspaceRecord, String)>, DbError> {
    Ok(tx
        .query_row(
            &format!("SELECT {WORKSPACE_COLUMNS}, create_hash FROM workspace WHERE owner_id = ?1 AND create_key = ?2"),
            params![owner.to_string(), key],
            |row| Ok((workspace_row(row)?, row.get::<_, Option<String>>(15)?.unwrap_or_default())),
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

pub fn workspace(tx: &Connection, id: WorkspaceId) -> Result<Option<WorkspaceRecord>, DbError> {
    Ok(tx
        .query_row(
            &format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspace WHERE id = ?1 AND deleted_at IS NULL"
            ),
            [id.to_string()],
            workspace_row,
        )
        .optional()?)
}

pub fn workspaces(tx: &Connection, owner: PrincipalId) -> Result<Vec<WorkspaceRecord>, DbError> {
    let mut statement = tx.prepare(&format!(
        "SELECT {WORKSPACE_COLUMNS} FROM workspace WHERE owner_id = ?1 AND deleted_at IS NULL ORDER BY created_at"
    ))?;
    Ok(statement
        .query_map([owner.to_string()], workspace_row)?
        .collect::<Result<_, _>>()?)
}

pub fn live_workspaces(tx: &Connection) -> Result<Vec<WorkspaceRecord>, DbError> {
    let mut statement = tx.prepare(&format!(
        "SELECT {WORKSPACE_COLUMNS} FROM workspace WHERE deleted_at IS NULL ORDER BY created_at"
    ))?;
    Ok(statement
        .query_map([], workspace_row)?
        .collect::<Result<_, _>>()?)
}

/// Sets the desired state if the revision still matches `expected`.
/// Returns the new revision, or `None` on a revision conflict.
pub fn set_desired(
    tx: &Connection,
    id: WorkspaceId,
    desired: DesiredState,
    expected: Option<Revision>,
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
    if expected.is_some_and(|expected| expected != current) {
        return Ok(None);
    }
    let next = current.next();
    tx.execute(
        "UPDATE workspace SET desired = ?2, revision = ?3, condition = NULL WHERE id = ?1",
        params![id.to_string(), desired.as_str(), i64_of(next.get())],
    )?;
    Ok(Some(next))
}

pub fn record_observation(
    tx: &Connection,
    id: WorkspaceId,
    instance: &Instance,
    memory: Option<Bytes>,
    now: Timestamp,
) -> Result<(), DbError> {
    tx.execute(
        "UPDATE workspace SET observed = ?2, observed_at = ?3, memory = ?4 WHERE id = ?1",
        params![
            id.to_string(),
            to_json(instance)?,
            now.unix_millis(),
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

/// Marks a workspace deleted and retires its route names.
pub fn mark_deleted(tx: &Connection, id: WorkspaceId, now: Timestamp) -> Result<(), DbError> {
    tx.execute(
        "UPDATE route_name SET retired_at = ?2 WHERE name IN (SELECT name FROM route WHERE workspace_id = ?1)",
        params![id.to_string(), now.unix_millis()],
    )?;
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

/// Replaces a workspace's session statuses with what the guest reports now.
/// A status whose timestamp changed becomes unseen again.
pub fn sync_attention(
    tx: &Connection,
    id: WorkspaceId,
    statuses: &[SessionStatus],
) -> Result<bool, DbError> {
    let before: Vec<(String, i64)> = {
        let mut statement = tx.prepare(
            "SELECT session, updated_at FROM attention WHERE workspace_id = ?1 ORDER BY session",
        )?;
        statement
            .query_map([id.to_string()], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    let mut after: Vec<(String, i64)> = statements_key(statuses).into_iter().collect();
    after.sort();
    if before == after {
        return Ok(false);
    }
    tx.execute("DELETE FROM attention WHERE workspace_id = ?1 AND session NOT IN (SELECT value FROM json_each(?2))", params![
        id.to_string(),
        to_json(&statuses.iter().map(|s| s.session.as_str()).collect::<Vec<_>>())?
    ])?;
    for status in statuses {
        tx.execute(
            "INSERT INTO attention (workspace_id, session, state, summary, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (workspace_id, session) DO UPDATE SET
               state = excluded.state, summary = excluded.summary, updated_at = excluded.updated_at",
            params![
                id.to_string(),
                status.session.as_str(),
                status.state.as_str(),
                status.summary.as_str(),
                status.updated_at.unix_millis()
            ],
        )?;
    }
    Ok(true)
}

fn statements_key(statuses: &[SessionStatus]) -> Vec<(String, i64)> {
    statuses
        .iter()
        .map(|s| (s.session.to_string(), s.updated_at.unix_millis()))
        .collect()
}

pub fn attention(tx: &Connection, id: WorkspaceId) -> Result<Vec<AttentionRecord>, DbError> {
    let mut statement = tx.prepare(
        "SELECT session, state, summary, updated_at, seen_at FROM attention WHERE workspace_id = ?1 ORDER BY session",
    )?;
    Ok(statement
        .query_map([id.to_string()], |row| {
            let updated_at = timestamp(row, 3)?;
            let seen_at = timestamp(row, 4)?;
            Ok(AttentionRecord {
                status: SessionStatus {
                    session: text(row, 0)?,
                    state: text(row, 1)?,
                    summary: text(row, 2)?,
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
        "INSERT INTO route_name (name, owner_id, generation) VALUES (?1, ?2, 1) ON CONFLICT (name) DO NOTHING",
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

pub fn delete_route(
    tx: &Connection,
    workspace: WorkspaceId,
    id: RouteId,
    now: Timestamp,
) -> Result<bool, DbError> {
    let name: Option<String> = tx
        .query_row(
            "SELECT name FROM route WHERE id = ?1 AND workspace_id = ?2",
            params![id.to_string(), workspace.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(name) = name else { return Ok(false) };
    tx.execute("DELETE FROM route WHERE id = ?1", [id.to_string()])?;
    tx.execute(
        "UPDATE route_name SET retired_at = ?2 WHERE name = ?1",
        params![name, now.unix_millis()],
    )?;
    Ok(true)
}

pub fn flag_service_worker(tx: &Connection, name: &RouteName) -> Result<(), DbError> {
    tx.execute(
        "UPDATE route_name SET sw_seen = 1 WHERE name = ?1",
        [name.as_str()],
    )?;
    Ok(())
}

// ---- secrets ----

pub struct SealedSecret {
    pub target: SecretTarget,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

pub fn put_secret(
    tx: &Connection,
    id: SecretId,
    owner: PrincipalId,
    name: &SecretName,
    sealed: &SealedSecret,
    now: Timestamp,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO secret (id, owner_id, name, target, nonce, ciphertext, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (owner_id, name) DO UPDATE SET
           target = excluded.target, nonce = excluded.nonce, ciphertext = excluded.ciphertext, updated_at = excluded.updated_at",
        params![id.to_string(), owner.to_string(), name.as_str(), to_json(&sealed.target)?, sealed.nonce, sealed.ciphertext, now.unix_millis()],
    )?;
    bump_secrets_generation(tx, owner)
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

pub fn secrets(tx: &Connection, owner: PrincipalId) -> Result<Vec<SecretSummary>, DbError> {
    let mut statement = tx.prepare(
        "SELECT id, name, target, updated_at FROM secret WHERE owner_id = ?1 ORDER BY name",
    )?;
    Ok(statement
        .query_map([owner.to_string()], |row| {
            Ok(SecretSummary {
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
        "SELECT target, nonce, ciphertext FROM secret WHERE owner_id = ?1 ORDER BY name",
    )?;
    Ok(statement
        .query_map([owner.to_string()], |row| {
            Ok(SealedSecret {
                target: opt_json(row, 0)?
                    .ok_or_else(|| conversion(0, std::io::Error::other("missing target")))?,
                nonce: row.get(1)?,
                ciphertext: row.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

// ---- activity ----

#[derive(Clone, Debug, Serialize)]
pub struct ActivityView {
    pub kind: String,
    pub detail: String,
    pub at: Timestamp,
}

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
) -> Result<Vec<ActivityView>, DbError> {
    let mut statement =
        tx.prepare("SELECT kind, detail, at FROM activity WHERE workspace_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2")?;
    Ok(statement
        .query_map(params![workspace.to_string(), limit], |row| {
            Ok(ActivityView {
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
        let id = EnvRevisionId::from_uuid(Uuid::new_v4());
        insert_revision(conn, id, env, at(1)).expect("revision");
        id
    }

    fn workspace_record(owner: PrincipalId, env: EnvRevisionId, name: &str) -> WorkspaceRecord {
        WorkspaceRecord {
            id: WorkspaceId::from_uuid(Uuid::new_v4()),
            owner,
            host: "host-1".parse().expect("host"),
            env_revision: env,
            name: name.parse().expect("workspace name"),
            repo: "https://github.com/acme/app.git".parse().expect("repo"),
            branch: name.parse().expect("branch"),
            base: None,
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
        let ws = workspace_record(owner, revision(conn, owner), name);
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
        let duplicate = workspace_record(owner, ws.env_revision, "demo");
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
        let stale = Some(Revision::from_u64(7));
        assert_eq!(
            set_desired(&conn, ws.id, DesiredState::Frozen, stale).expect("query"),
            None
        );
        let next = set_desired(&conn, ws.id, DesiredState::Frozen, Some(ws.revision))
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
        assert!(delete_route(&conn, ws.id, created.id, at(3)).expect("delete"));

        let other = workspace_record(owner, ws.env_revision, "other");
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
        let statuses = vec![SessionStatus {
            session: "t1".parse::<SessionName>().expect("session"),
            state: AttentionState::Waiting,
            summary: Summary::sanitize("approve Bash?"),
            updated_at: at(10),
        }];
        assert!(sync_attention(&conn, ws.id, &statuses).expect("sync"));
        assert!(!sync_attention(&conn, ws.id, &statuses).expect("sync"));
        assert!(sync_attention(&conn, ws.id, &[]).expect("sync"));
    }
}
