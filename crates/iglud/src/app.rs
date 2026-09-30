//! Shared state, the API error type, and resolving who is calling.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use iglu_domain::auth::{Action, Decision, Resource, authorize};
use iglu_domain::id::PrincipalId;
use iglu_domain::label::HostId;
use iglu_domain::preview::{
    ConsoleOrigin, Credential, CsrfToken, FetchMetadata, MethodClass, Transport, console_access,
};
use iglu_domain::time::{Millis, Timestamp};
use serde_json::json;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::crypto::{self, Sealer};
use crate::db::{self, Db, DbError, SessionKind, SessionRow};
use crate::hosts::HostClient;
use crate::model::PrincipalRecord;
use crate::oidc::RelyingParty;

pub const CONSOLE_COOKIE: &str = "__Host-iglu";
pub const PREVIEW_COOKIE: &str = "__Secure-iglu-preview";
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
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("internal error")]
    Internal,
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

impl From<iglu_domain::ParseError> for ApiError {
    fn from(error: iglu_domain::ParseError) -> Self {
        Self::BadRequest(error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        (
            status,
            Json(json!({ "error": code, "message": self.to_string() })),
        )
            .into_response()
    }
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
#[derive(Clone, Debug)]
pub struct Caller {
    pub principal: PrincipalRecord,
    pub session_hash: String,
    pub kind: SessionKind,
}

impl Caller {
    pub fn authorize(&self, action: Action, owner: PrincipalId) -> Result<(), ApiError> {
        match authorize(self.principal.principal(), action, Resource { owner }) {
            Decision::Allow => Ok(()),
            // Other people's resources look missing, not forbidden.
            Decision::Deny(iglu_domain::auth::DenyReason::NotOwner) => Err(ApiError::NotFound),
            Decision::Deny(reason) => Err(ApiError::Forbidden(reason.to_string())),
        }
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
            kind,
        })
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
