//! Sign-in for the console, the preview gateway and the CLI.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use iglu_api::{CliToken, CliTokenRequest};
use iglu_domain::id::PrincipalId;
use iglu_domain::preview::FetchSite;
use iglu_domain::signin::{ReturnPath, same_browser};
use iglu_domain::time::Millis;
use serde::Deserialize;
use uuid::Uuid;

use crate::app::{
    ApiError, App, CONSOLE_COOKIE, PREVIEW_COOKIE, cookie, fetch_metadata, now, session_principal,
};
use crate::crypto;
use crate::db::{self, LoginRow, SessionKind};
use crate::extract::{Body, Form, Query};
use crate::oidc::{OidcError, RelyingParty};

/// The console's sign-in pages. The CLI's token exchange is part of the API.
pub fn console_router() -> Router<Arc<App>> {
    Router::new()
        .route("/auth/login", get(console_login))
        .route("/auth/callback", get(console_callback))
        .route("/auth/logout", post(logout))
        .route("/auth/cli", get(cli_confirm).post(cli_approve))
        .method_not_allowed_fallback(async || PageError(ApiError::MethodNotAllowed))
}

/// A refusal on a page someone opened in their browser: a page saying why,
/// rather than the API's JSON.
pub struct PageError(ApiError);

impl From<ApiError> for PageError {
    fn from(error: ApiError) -> Self {
        Self(error)
    }
}

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        error_page(&self.0, "/")
    }
}

/// A page saying why a sign-in step was refused, linking back to `home`.
fn error_page(error: &ApiError, home: &str) -> Response {
    let page = format!(
        r#"<!doctype html><meta charset="utf-8"><title>iglu</title>
<main class="dialog"><h1>That didn't work</h1><p>{message}</p><p><a href="{home}">Back to iglu</a></p></main>"#,
        message = escape(&sentence(&error.to_string())),
        home = escape(home),
    );
    (error.status(), Html(page)).into_response()
}

/// An API message, which is a lowercase phrase, as a sentence.
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    let mut text: String = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !text.ends_with(['.', '!', '?']) {
        text.push('.');
    }
    text
}

#[derive(Deserialize)]
struct ReturnTo {
    #[serde(default)]
    r#return: Option<String>,
}

/// An unreachable provider is worth retrying; anything else is a refusal.
#[expect(
    clippy::needless_pass_by_value,
    reason = "an adapter for map_err, which passes errors by value"
)]
fn sign_in_error(error: OidcError) -> ApiError {
    tracing::warn!(%error, "sign-in failed");
    match error {
        OidcError::Discovery(_) => {
            ApiError::Unavailable("the identity provider isn't reachable; try again shortly".into())
        }
        OidcError::Config(_)
        | OidcError::Exchange(_)
        | OidcError::Token(_)
        | OidcError::Claims(_) => ApiError::Forbidden("sign-in failed".into()),
    }
}

async fn begin(
    app: &App,
    rp: &RelyingParty,
    kind: SessionKind,
    return_to: String,
) -> Result<Response, ApiError> {
    let pending = rp.begin().await.map_err(sign_in_error)?;
    let login = LoginRow {
        kind,
        nonce: pending.nonce,
        pkce_verifier: pending.pkce_verifier,
        return_to,
    };
    let state = pending.state.clone();
    let stored = state.clone();
    app.db
        .call(move |tx| db::insert_login(tx, &stored, &login, now()))
        .await?;
    let cookie =
        format!("{SIGNIN_COOKIE}={state}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=600");
    Ok(with_cookie(
        Redirect::to(pending.url.as_str()).into_response(),
        &cookie,
    ))
}

async fn console_login(
    State(app): State<Arc<App>>,
    query: Result<Query<ReturnTo>, ApiError>,
) -> Result<Response, PageError> {
    let Query(query) = query?;
    Ok(begin(
        &app,
        &app.console,
        SessionKind::Console,
        query
            .r#return
            .and_then(|path| path.parse().ok())
            .unwrap_or_else(ReturnPath::root)
            .to_string(),
    )
    .await?)
}

#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Completes a sign-in started in the browser holding `signin_cookie`, and
/// returns the principal and where to go next.
async fn complete(
    app: &App,
    rp: &RelyingParty,
    kind: SessionKind,
    callback: Callback,
    signin_cookie: Option<&str>,
) -> Result<(PrincipalId, String), ApiError> {
    if let Some(error) = callback.error {
        return Err(ApiError::Forbidden(format!(
            "the identity provider refused: {error}"
        )));
    }
    let (code, state) = callback
        .code
        .zip(callback.state)
        .ok_or_else(|| ApiError::BadRequest("missing code".into()))?;
    if !same_browser(signin_cookie, &state) {
        return Err(ApiError::BadRequest(
            "finish signing in in the browser that started it".into(),
        ));
    }
    let login = app
        .db
        .call(move |tx| db::take_login(tx, &state))
        .await?
        .filter(|login| login.kind == kind)
        .ok_or_else(|| ApiError::BadRequest("this sign-in expired; try again".into()))?;
    let identity = rp
        .finish(code, login.nonce, login.pkce_verifier)
        .await
        .map_err(sign_in_error)?;
    if !app.config.sign_in.admits(&identity) {
        tracing::info!(subject = %identity.subject, "sign-in refused by policy");
        return Err(ApiError::Forbidden(
            "this account isn't allowed to use iglu".into(),
        ));
    }
    let id = PrincipalId::from_uuid(Uuid::new_v4());
    let principal = app
        .db
        .call(move |tx| db::upsert_principal(tx, id, &identity, now()))
        .await?;
    Ok((principal.id, login.return_to))
}

async fn new_session(
    app: &App,
    principal: PrincipalId,
    kind: SessionKind,
    lifetime: Millis,
) -> Result<String, ApiError> {
    let token = crypto::token()?;
    let csrf = crypto::token()?;
    let hash = crypto::hash(&token);
    let issued = now();
    app.db
        .call(move |tx| {
            db::create_session(
                tx,
                &hash,
                kind,
                principal,
                &csrf,
                issued,
                issued.plus(lifetime),
            )
        })
        .await?;
    Ok(token)
}

fn browser_lifetime(app: &App) -> Millis {
    Millis::from_secs(app.config.sessions.absolute_hours.saturating_mul(3600))
}

/// Holds a sign-in's `state` in the browser that started it.
const SIGNIN_COOKIE: &str = "__Host-iglu-signin";
const CLEAR_SIGNIN: &str = "__Host-iglu-signin=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0";

fn with_cookie(mut response: Response, cookie: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(cookie) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

async fn console_callback(
    State(app): State<Arc<App>>,
    callback: Result<Query<Callback>, ApiError>,
    headers: HeaderMap,
) -> Result<Response, PageError> {
    let Query(callback) = callback?;
    let (principal, return_to) = complete(
        &app,
        &app.console,
        SessionKind::Console,
        callback,
        cookie(&headers, SIGNIN_COOKIE),
    )
    .await?;
    let token = new_session(
        &app,
        principal,
        SessionKind::Console,
        browser_lifetime(&app),
    )
    .await?;
    let cookie = format!("{CONSOLE_COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Lax");
    let response = with_cookie(Redirect::to(&return_to).into_response(), &cookie);
    Ok(with_cookie(response, CLEAR_SIGNIN))
}

async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> Result<Response, ApiError> {
    if fetch_metadata(&headers).site != Some(FetchSite::SameOrigin) {
        return Err(ApiError::Forbidden("cross-origin logout".into()));
    }
    if let Some(token) = cookie(&headers, CONSOLE_COOKIE) {
        let hash = crypto::hash(token);
        app.db.call(move |tx| db::delete_session(tx, &hash)).await?;
    }
    let cookie = format!("{CONSOLE_COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0");
    Ok(with_cookie(StatusCode::NO_CONTENT.into_response(), &cookie))
}

// ---- CLI sign-in: a loopback redirect with PKCE ----

#[derive(Deserialize)]
struct CliRequest {
    port: u16,
    challenge: String,
    state: String,
}

impl CliRequest {
    fn check(&self) -> Result<(), ApiError> {
        let url_safe = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        };
        if self.port < 1024 || !url_safe(&self.challenge) || !url_safe(&self.state) {
            return Err(ApiError::BadRequest("malformed CLI sign-in request".into()));
        }
        Ok(())
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn cli_confirm(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    request: Result<Query<CliRequest>, ApiError>,
) -> Result<Response, PageError> {
    let Query(request) = request?;
    request.check()?;
    let session = match cookie(&headers, CONSOLE_COOKIE) {
        Some(token) => session_principal(&app, token, SessionKind::Console).await?,
        None => None,
    };
    let Some((principal, row)) = session else {
        let here = format!(
            "/auth/cli?port={}&challenge={}&state={}",
            request.port, request.challenge, request.state
        );
        return Ok(Redirect::to(&format!(
            "/auth/login?return={}",
            url::form_urlencoded::byte_serialize(here.as_bytes()).collect::<String>()
        ))
        .into_response());
    };
    let who = principal
        .email
        .as_ref()
        .map_or_else(|| principal.id.to_string(), ToString::to_string);
    let page = format!(
        r#"<!doctype html><meta charset="utf-8"><title>Sign in the iglu CLI</title>
<link rel="stylesheet" href="/app.css">
<main class="dialog"><h1>Sign in the iglu CLI?</h1>
<p>A program on this computer asked to act as <strong>{who}</strong>. Only continue if you just ran <code>iglu login</code>.</p>
<form method="post" action="/auth/cli">
<input type="hidden" name="port" value="{port}"><input type="hidden" name="challenge" value="{challenge}">
<input type="hidden" name="state" value="{state}"><input type="hidden" name="csrf" value="{csrf}">
<button type="submit">Sign in the CLI</button></form></main>"#,
        who = escape(&who),
        port = request.port,
        challenge = escape(&request.challenge),
        state = escape(&request.state),
        csrf = escape(&row.csrf),
    );
    Ok(Html(page).into_response())
}

#[derive(Deserialize)]
struct CliApproval {
    port: u16,
    challenge: String,
    state: String,
    csrf: String,
}

async fn cli_approve(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    form: Result<Form<CliApproval>, ApiError>,
) -> Result<Response, PageError> {
    let Form(form) = form?;
    let request = CliRequest {
        port: form.port,
        challenge: form.challenge,
        state: form.state,
    };
    request.check()?;
    if fetch_metadata(&headers).site != Some(FetchSite::SameOrigin) {
        return Err(ApiError::Forbidden("cross-origin request".into()).into());
    }
    let token = cookie(&headers, CONSOLE_COOKIE).ok_or(ApiError::Unauthorized)?;
    let (principal, row) = session_principal(&app, token, SessionKind::Console)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if !crypto::constant_time_eq(form.csrf.as_bytes(), row.csrf.as_bytes()) {
        return Err(ApiError::Forbidden("missing or wrong CSRF token".into()).into());
    }
    let code = crypto::token().map_err(ApiError::from)?;
    let hash = crypto::hash(&code);
    let challenge = request.challenge.clone();
    let id = principal.id;
    app.db
        .call(move |tx| {
            db::insert_cli_code(
                tx,
                &hash,
                id,
                &challenge,
                now().plus(Millis::from_secs(120)),
            )
        })
        .await
        .map_err(ApiError::from)?;
    Ok(Redirect::to(&format!(
        "http://127.0.0.1:{}/callback?code={code}&state={}",
        request.port, request.state
    ))
    .into_response())
}

pub async fn cli_token(
    State(app): State<Arc<App>>,
    Body(exchange): Body<CliTokenRequest>,
) -> Result<Json<CliToken>, ApiError> {
    let hash = crypto::hash(&exchange.code);
    let (principal, challenge) = app
        .db
        .call(move |tx| db::take_cli_code(tx, &hash, now()))
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if !crypto::pkce_matches(&exchange.verifier, &challenge) {
        return Err(ApiError::Unauthorized);
    }
    let lifetime = Millis::from_secs(app.config.sessions.cli_days.saturating_mul(86_400));
    let token = new_session(&app, principal, SessionKind::Api, lifetime).await?;
    Ok(Json(CliToken { token }))
}

// ---- the preview gateway's own sign-in, on auth.<preview domain> ----

/// Accepts only `https://<label>.<preview domain>/...` as a return address.
fn preview_return(app: &App, value: Option<String>) -> Option<String> {
    let url = url::Url::parse(&value?).ok()?;
    let host = url.host_str()?;
    let label = host.strip_suffix(&format!(".{}", app.config.preview_domain))?;
    (url.scheme() == "https" && !label.contains('.')).then(|| url.to_string())
}

pub async fn preview_auth(
    app: &Arc<App>,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
) -> Response {
    let params: Vec<(String, String)> =
        url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let result = match path {
        "/login" => match preview_return(app, get("return")) {
            Some(return_to) => begin(app, &app.preview, SessionKind::Preview, return_to).await,
            None => Err(ApiError::BadRequest("bad return address".into())),
        },
        "/callback" => {
            let callback = Callback {
                code: get("code"),
                state: get("state"),
                error: get("error"),
            };
            let signin = cookie(headers, SIGNIN_COOKIE);
            match complete(app, &app.preview, SessionKind::Preview, callback, signin).await {
                Ok((principal, return_to)) => {
                    match new_session(app, principal, SessionKind::Preview, browser_lifetime(app))
                        .await
                    {
                        Ok(token) => {
                            let cookie = format!(
                                "{PREVIEW_COOKIE}={token}; Domain={}; Path=/; Secure; HttpOnly; SameSite=Lax",
                                app.config.preview_domain
                            );
                            let response =
                                with_cookie(Redirect::to(&return_to).into_response(), &cookie);
                            Ok(with_cookie(response, CLEAR_SIGNIN))
                        }
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            }
        }
        _ => Err(ApiError::NotFound),
    };
    result.unwrap_or_else(|error| error_page(&error, &app.config.console_origin_string()))
}
