//! Shared state, the API error type, and resolving who is calling.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use iglu_api::{ErrorBody, ErrorKind, Field};
use iglu_domain::auth::{self, Action, Decision, DenyReason, Grant, Resource, authorize};
use iglu_domain::id::{PrincipalId, WorkspaceId};
use iglu_domain::label::HostId;
use iglu_domain::preview::{
    ConsoleOrigin, Credential, CsrfToken, FetchMetadata, MethodClass, Transport, console_access,
};
use iglu_domain::secret::{SecretTarget, SecretValue};
use iglu_domain::time::{Millis, Timestamp};
use tokio::sync::broadcast;

use crate::config::Config;
use crate::crypto::{self, Binding, Sealer};
use crate::db::{self, Db, DbError, SessionKind, SessionRow};
use crate::hosts::HostClient;
use crate::model::{PrincipalRecord, WorkspaceRecord};
use crate::oidc::RelyingParty;

pub const CONSOLE_COOKIE: &str = "__Host-iglu";
pub use iglu_domain::preview::PREVIEW_COOKIE;
pub const CSRF_HEADER: &str = "x-csrf-token";

pub struct App {
    pub config: Config,
    pub db: Db,
    pub sealer: Sealer,
    pub console: RelyingParty,
    pub preview: RelyingParty,
    pub hosts: Vec<Arc<HostClient>>,
    /// Signals that something users can see changed.
    pub changes: broadcast::Sender<()>,
    /// When each host last answered.
    pub host_seen: Mutex<HashMap<HostId, Timestamp>>,
    pub reconciler: Arc<crate::reconcile::Reconciler>,
    pub pool: crate::gateway::Pool,
    pub usage: crate::idle::Usage,
    /// This run of iglud, which the console compares to notice an upgrade.
    pub boot: iglu_api::BootId,
    pub column_edits: ColumnEdits,
}

/// One change at a time to a workspace's columns. Adding one picks a free
/// name, opens its session on the host, then records it; two at once would
/// pick the same name. Layout changes are one transaction and don't need it.
#[derive(Default)]
pub struct ColumnEdits(Mutex<HashMap<WorkspaceId, Arc<tokio::sync::Mutex<()>>>>);

impl ColumnEdits {
    /// Waits for the workspace's columns, held until the guard drops.
    pub async fn lock(&self, id: WorkspaceId) -> tokio::sync::OwnedMutexGuard<()> {
        let edits = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(id)
            .or_default()
            .clone();
        edits.lock_owned().await
    }
}

impl App {
    pub fn host(&self, id: &HostId) -> Option<Arc<HostClient>> {
        self.hosts.iter().find(|host| host.id == *id).cloned()
    }

    /// New workspaces go to the first host; placement comes with a second host.
    pub fn default_host(&self) -> Option<Arc<HostClient>> {
        self.hosts.first().cloned()
    }

    pub fn changed(&self) {
        let _ = self.changes.send(());
    }

    /// Asks the reconciler to look now rather than at its next tick.
    pub fn kick(&self) {
        self.reconciler.kick.notify_one();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenSecretsError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Crypto(#[from] crypto::CryptoError),
    #[error("a stored secret is unreadable")]
    Unreadable,
}

/// An owner's stored secrets, decrypted.
pub async fn open_secrets(
    app: &App,
    owner: PrincipalId,
) -> Result<Vec<(SecretTarget, SecretValue)>, OpenSecretsError> {
    let sealed = app.db.call(move |tx| db::sealed_secrets(tx, owner)).await?;
    let mut opened = Vec::with_capacity(sealed.len());
    for secret in sealed {
        let binding = Binding {
            owner,
            name: &secret.name,
            target: &secret.target,
        };
        let plaintext = app
            .sealer
            .open(&secret.nonce, &secret.ciphertext, &binding)?;
        let value = String::from_utf8(plaintext)
            .ok()
            .and_then(|text| SecretValue::try_from(text).ok())
            .ok_or(OpenSecretsError::Unreadable)?;
        opened.push((secret.target, value));
    }
    Ok(opened)
}

pub fn now() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0);
    Timestamp::from_unix_millis(millis)
}

/// Errors every API handler can return. Messages are safe to show.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("sign in first")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("not found")]
    NotFound,
    #[error("this path doesn't take that method")]
    MethodNotAllowed,
    #[error("{0}")]
    Conflict(Problem),
    #[error("{0}")]
    BadRequest(Problem),
    #[error("the request body is too large")]
    TooLarge,
    #[error("{0}")]
    UnsupportedMediaType(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("internal error")]
    Internal,
}

/// What's wrong with a request, and the input it's about when there is one.
#[derive(Debug, PartialEq, Eq)]
pub struct Problem {
    pub message: String,
    pub field: Option<Field>,
}

impl Problem {
    pub fn at(field: &str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            field: Some(Field::new(field)),
        }
    }
}

impl From<String> for Problem {
    fn from(message: String) -> Self {
        Self {
            message,
            field: None,
        }
    }
}

impl From<&str> for Problem {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl ApiError {
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::Unauthorized => ErrorKind::Unauthorized,
            Self::Forbidden(_) => ErrorKind::Forbidden,
            Self::NotFound => ErrorKind::NotFound,
            Self::MethodNotAllowed => ErrorKind::MethodNotAllowed,
            Self::Conflict(_) => ErrorKind::Conflict,
            Self::BadRequest(_) => ErrorKind::BadRequest,
            Self::TooLarge => ErrorKind::TooLarge,
            Self::UnsupportedMediaType(_) => ErrorKind::UnsupportedMediaType,
            Self::Unavailable(_) => ErrorKind::Unavailable,
            Self::Internal => ErrorKind::Internal,
        }
    }

    pub const fn status(&self) -> StatusCode {
        match self.kind() {
            ErrorKind::BadRequest => StatusCode::BAD_REQUEST,
            ErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorKind::Forbidden => StatusCode::FORBIDDEN,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            ErrorKind::Conflict => StatusCode::CONFLICT,
            ErrorKind::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ErrorKind::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn body(self) -> ErrorBody {
        let error = self.kind();
        let message = self.to_string();
        let field = match self {
            Self::Conflict(problem) | Self::BadRequest(problem) => problem.field,
            Self::Unauthorized
            | Self::Forbidden(_)
            | Self::NotFound
            | Self::MethodNotAllowed
            | Self::TooLarge
            | Self::UnsupportedMediaType(_)
            | Self::Unavailable(_)
            | Self::Internal => None,
        };
        ErrorBody {
            error,
            message,
            field,
        }
    }
}

impl From<DbError> for ApiError {
    fn from(error: DbError) -> Self {
        tracing::error!(%error, "database error");
        Self::Internal
    }
}

impl From<crypto::CryptoError> for ApiError {
    fn from(error: crypto::CryptoError) -> Self {
        tracing::error!(%error, "crypto error");
        Self::Internal
    }
}

impl From<OpenSecretsError> for ApiError {
    fn from(error: OpenSecretsError) -> Self {
        tracing::error!(%error, "couldn't open stored secrets");
        Self::Internal
    }
}

impl From<iglu_domain::ParseError> for ApiError {
    fn from(error: iglu_domain::ParseError) -> Self {
        Self::BadRequest(error.to_string().into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status(), Json(self.body())).into_response()
    }
}

/// Answers API paths that don't exist, so they get an API error rather than
/// the console's page.
pub async fn no_such_path() -> ApiError {
    ApiError::NotFound
}

/// Answers what a workspace can't ask for at all: its owner's account, and
/// everything else that isn't a workspace's to do.
pub async fn not_for_workspaces() -> ApiError {
    ApiError::Forbidden(
        "a workspace can't do this from inside; its owner can, in the console or with their own `iglu`".into(),
    )
}

/// Answers API paths that exist, but not with the request's method. axum
/// adds the `Allow` header.
pub async fn no_such_method() -> ApiError {
    ApiError::MethodNotAllowed
}

/// Reads one cookie from the request.
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

pub fn fetch_metadata(headers: &HeaderMap) -> FetchMetadata {
    fn parse<T: std::str::FromStr>(headers: &HeaderMap, name: &str) -> Option<T> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
    }
    FetchMetadata {
        site: parse(headers, "sec-fetch-site"),
        mode: parse(headers, "sec-fetch-mode"),
    }
}

pub fn transport(method: &Method, headers: &HeaderMap) -> Transport {
    let websocket = headers
        .get(header::UPGRADE)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"));
    if websocket {
        Transport::WebSocket
    } else if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        Transport::Http(MethodClass::Safe)
    } else {
        Transport::Http(MethodClass::Unsafe)
    }
}

/// Whether a stored session is still usable: within its absolute lifetime
/// and, for browser sessions, recently active.
pub fn session_valid(row: &SessionRow, kind: SessionKind, now: Timestamp, idle: Millis) -> bool {
    let within_lifetime = now < row.expires_at;
    let recently_active = match kind {
        SessionKind::Console | SessionKind::Preview => now.since(row.last_seen_at) < idle,
        SessionKind::Api => true,
    };
    within_lifetime && recently_active
}

/// The signed-in caller of a console or API request.
#[derive(Clone)]
pub struct Caller {
    pub principal: PrincipalRecord,
    pub session_hash: String,
    /// The session's CSRF token, which the console sends back on writes.
    pub csrf_token: String,
    pub kind: SessionKind,
}

impl Caller {
    pub fn authorize(&self, action: Action, owner: PrincipalId) -> Result<(), ApiError> {
        let actor = auth::Actor::Person(self.principal.principal());
        decided(authorize(actor, action, Resource::owned_by(owner)), false)
    }
}

/// An answer for a caller: what they can't see looks missing, not forbidden.
fn decided(decision: Decision, visible: bool) -> Result<(), ApiError> {
    match decision {
        Decision::Allow => Ok(()),
        Decision::Deny(DenyReason::NotOwner) => Err(ApiError::NotFound),
        Decision::Deny(DenyReason::NotGranted) if !visible => Err(ApiError::NotFound),
        Decision::Deny(reason) => Err(ApiError::Forbidden(reason.to_string())),
    }
}

/// Marks a request that came from inside a workspace, through its channel.
/// Only the guest ingress adds it, after checking the channel is that
/// workspace's; no header can.
#[derive(Clone, Copy, Debug)]
pub struct FromWorkspace(pub WorkspaceId);

/// A workspace asking through its channel, for its owner as far as its
/// grants go.
#[derive(Clone)]
pub struct WorkspaceCaller {
    pub workspace: WorkspaceRecord,
    pub owner: PrincipalRecord,
    pub grants: Vec<Grant>,
}

/// Whoever is asking, where a person or a workspace may.
#[derive(Clone)]
pub enum Actor {
    Person(Caller),
    Workspace(Box<WorkspaceCaller>),
}

impl Actor {
    /// The person it acts for: whose workspaces it can see at all.
    pub const fn owner(&self) -> &PrincipalRecord {
        match self {
            Self::Person(caller) => &caller.principal,
            Self::Workspace(caller) => &caller.owner,
        }
    }

    pub fn by(&self) -> db::By {
        db::By {
            person: self.owner().id,
            via: match self {
                Self::Person(_) => None,
                Self::Workspace(caller) => Some(caller.workspace.id),
            },
        }
    }

    const fn domain(&self) -> auth::Actor<'_> {
        match self {
            Self::Person(caller) => auth::Actor::Person(caller.principal.principal()),
            Self::Workspace(caller) => auth::Actor::Workspace {
                owner: caller.owner.principal(),
                grants: caller.grants.as_slice(),
            },
        }
    }

    pub fn may(&self, action: Action, ws: &WorkspaceRecord) -> bool {
        authorize(self.domain(), action, Resource::workspace(ws.id, ws.owner)) == Decision::Allow
    }

    /// Whether it may do `action` to the workspace. A workspace it can't
    /// see looks missing.
    pub fn authorize(&self, action: Action, ws: &WorkspaceRecord) -> Result<(), ApiError> {
        decided(
            authorize(self.domain(), action, Resource::workspace(ws.id, ws.owner)),
            self.may(Action::ViewWorkspace, ws),
        )
    }
}

/// Looks up a session token of `kind` and checks it's still valid.
pub async fn session_principal(
    app: &App,
    token: &str,
    kind: SessionKind,
) -> Result<Option<(PrincipalRecord, SessionRow)>, ApiError> {
    let hash = crypto::hash(token);
    let idle = Millis::from_secs(app.config.sessions.idle_minutes.saturating_mul(60));
    let now = now();
    let found = app
        .db
        .call(move |tx| {
            let Some(row) = db::session(tx, &hash, kind)? else {
                return Ok(None);
            };
            if !session_valid(&row, kind, now, idle) {
                db::delete_session(tx, &hash)?;
                return Ok(None);
            }
            db::touch_session(tx, &hash, now)?;
            Ok(db::principal(tx, row.principal)?.map(|principal| (principal, row)))
        })
        .await?;
    Ok(found)
}

impl FromRequestParts<Arc<App>> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        app: &Arc<App>,
    ) -> Result<Self, Self::Rejection> {
        // What only a person may do, a workspace can't, whatever it sends.
        if parts.extensions.get::<FromWorkspace>().is_some() {
            return Err(ApiError::Forbidden("only a person can do this".into()));
        }
        let bearer = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        let (token, kind, credential) = match (bearer, cookie(&parts.headers, CONSOLE_COOKIE)) {
            (Some(token), _) => (token.to_owned(), SessionKind::Api, Credential::Bearer),
            (None, Some(token)) => (token.to_owned(), SessionKind::Console, Credential::Cookie),
            (None, None) => return Err(ApiError::Unauthorized),
        };
        let (principal, row) = session_principal(app, &token, kind)
            .await?
            .ok_or(ApiError::Unauthorized)?;

        let origin = match parts
            .headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
        {
            None => ConsoleOrigin::Absent,
            Some(origin) if origin == app.config.console_origin_string() => ConsoleOrigin::Exact,
            Some(_) => ConsoleOrigin::Other,
        };
        let csrf = match parts.headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()) {
            Some(sent) if crypto::constant_time_eq(sent.as_bytes(), row.csrf.as_bytes()) => {
                CsrfToken::Valid
            }
            Some(_) | None => CsrfToken::MissingOrWrong,
        };
        console_access(
            credential,
            transport(&parts.method, &parts.headers),
            fetch_metadata(&parts.headers),
            origin,
            csrf,
        )
        .map_err(|rejection| ApiError::Forbidden(rejection.to_string()))?;
        Ok(Self {
            principal,
            session_hash: crypto::hash(&token),
            csrf_token: row.csrf,
            kind,
        })
    }
}

impl FromRequestParts<Arc<App>> for Actor {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        app: &Arc<App>,
    ) -> Result<Self, Self::Rejection> {
        let Some(&FromWorkspace(id)) = parts.extensions.get::<FromWorkspace>() else {
            return Caller::from_request_parts(parts, app)
                .await
                .map(Self::Person);
        };
        // Loaded for each request, so a change to its grants applies to the
        // next one.
        let found = app
            .db
            .call(move |tx| {
                let Some(workspace) = db::workspace(tx, id)? else {
                    return Ok(None);
                };
                let Some(owner) = db::principal(tx, workspace.owner)? else {
                    return Ok(None);
                };
                Ok(Some(WorkspaceCaller {
                    grants: db::grants(tx, id)?,
                    workspace,
                    owner,
                }))
            })
            .await?;
        match found {
            Some(caller)
                if caller.workspace.desired != iglu_domain::lifecycle::DesiredState::Deleted =>
            {
                Ok(Self::Workspace(Box::new(caller)))
            }
            _ => Err(ApiError::Unauthorized),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn row(seen: i64, expires: i64) -> SessionRow {
        SessionRow {
            principal: PrincipalId::from_uuid(Uuid::nil()),
            csrf: String::new(),
            last_seen_at: Timestamp::from_unix_millis(seen),
            expires_at: Timestamp::from_unix_millis(expires),
        }
    }

    #[test]
    fn browser_sessions_expire_when_idle_or_old() {
        let idle = Millis::from_secs(60);
        let now = Timestamp::from_unix_millis(100_000);
        assert!(session_valid(
            &row(90_000, 200_000),
            SessionKind::Console,
            now,
            idle
        ));
        assert!(!session_valid(
            &row(10_000, 200_000),
            SessionKind::Console,
            now,
            idle
        ));
        assert!(!session_valid(
            &row(99_000, 100_000),
            SessionKind::Console,
            now,
            idle
        ));
    }

    #[test]
    fn api_tokens_only_expire_absolutely() {
        let idle = Millis::from_secs(60);
        let now = Timestamp::from_unix_millis(100_000);
        assert!(session_valid(&row(0, 200_000), SessionKind::Api, now, idle));
        assert!(!session_valid(&row(0, 50_000), SessionKind::Api, now, idle));
    }

    #[test]
    fn cookies_are_found_by_name() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "a=1; __Host-iglu=tok; b=2".parse().expect("valid header"),
        );
        assert_eq!(cookie(&headers, CONSOLE_COOKIE), Some("tok"));
        assert_eq!(cookie(&headers, "missing"), None);
    }
}
