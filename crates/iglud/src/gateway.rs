//! The preview gateway: `https://<route>.<preview domain>` to a guest's
//! loopback port, for signed-in users only.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Redirect, Response};
use hyper::client::conn::http1::SendRequest;
use hyper_util::rt::TokioIo;
use iglu_domain::auth::{Action, Decision, DenyReason, Resource, authorize};
use iglu_domain::id::WorkspaceId;
use iglu_domain::label::{PREVIEW_AUTH_LABEL, RouteName};
use iglu_domain::lifecycle::Phase;
use iglu_domain::port::GuestPort;
use iglu_domain::preview::{
    MethodClass, PreviewAccess, RequestSource, Transport, preview_access, preview_may_set_cookie,
};

use crate::app::{App, PREVIEW_COOKIE, cookie, fetch_metadata, session_principal, transport};
use crate::db::{self, SessionKind};

type Upstream = (WorkspaceId, GuestPort);

/// Upstream connections kept per route, so page loads don't open a tunnel per asset.
#[derive(Default)]
pub struct Pool(Mutex<HashMap<Upstream, Vec<SendRequest<Body>>>>);

const POOL_LIMIT: usize = 16;

impl Pool {
    fn take(&self, key: Upstream) -> Option<SendRequest<Body>> {
        let mut pool = self
            .0
            .lock()
            .expect("the pool lock is never held across a panic");
        let senders = pool.get_mut(&key)?;
        senders.retain(|sender| !sender.is_closed());
        let index = senders.iter().position(SendRequest::is_ready)?;
        Some(senders.swap_remove(index))
    }

    fn put(&self, key: Upstream, sender: SendRequest<Body>) {
        let mut pool = self
            .0
            .lock()
            .expect("the pool lock is never held across a panic");
        let senders = pool.entry(key).or_default();
        if senders.len() < POOL_LIMIT {
            senders.push(sender);
        }
    }
}

/// The preview label of a host header value, if it's in the preview domain.
pub fn preview_label<'a>(app: &App, host: &'a str) -> Option<&'a str> {
    let host = host.rsplit_once(':').map_or(host, |(name, _)| name);
    host.strip_suffix(&app.config.preview_domain)?
        .strip_suffix('.')
        .filter(|label| !label.contains('.'))
}

fn source_label<'a>(app: &App, value: &'a str) -> Option<&'a str> {
    let rest = value.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    preview_label(app, host)
}

async fn source(
    app: &App,
    headers: &HeaderMap,
    own: &RouteName,
    workspace: WorkspaceId,
) -> RequestSource {
    let from = headers
        .get(header::ORIGIN)
        .or_else(|| headers.get(header::REFERER))
        .and_then(|v| v.to_str().ok())
        .filter(|v| *v != "null");
    let Some(from) = from else {
        return RequestSource::Unknown;
    };
    let Some(label) = source_label(app, from) else {
        return RequestSource::Elsewhere;
    };
    if label == own.as_str() {
        return RequestSource::SameOrigin;
    }
    let Ok(name) = label.parse::<RouteName>() else {
        return RequestSource::Elsewhere;
    };
    match app.db.call(move |tx| db::route_by_name(tx, &name)).await {
        Ok(Some(route)) if route.workspace == workspace => RequestSource::SameWorkspace,
        Ok(Some(_) | None) | Err(_) => RequestSource::Elsewhere,
    }
}

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

fn strip_hop_by_hop(headers: &mut HeaderMap, keep_upgrade: bool) {
    let listed: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|name| name.trim().parse().ok())
        .collect();
    for name in listed {
        if !(keep_upgrade && name == header::UPGRADE) {
            headers.remove(name);
        }
    }
    for name in HOP_BY_HOP {
        if keep_upgrade && (*name == "connection" || *name == "upgrade") {
            continue;
        }
        headers.remove(*name);
    }
}

/// Drops the app's cookies that would reach beyond its own preview.
fn filter_app_cookies(headers: &mut HeaderMap) {
    let allowed: Vec<HeaderValue> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter(|value| value.to_str().is_ok_and(preview_may_set_cookie))
        .cloned()
        .collect();
    headers.remove(header::SET_COOKIE);
    for value in allowed {
        headers.append(header::SET_COOKIE, value);
    }
}

/// Removes the gateway's own cookie; the app's cookies pass through.
fn strip_platform_cookie(headers: &mut HeaderMap) {
    let kept: Vec<String> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .filter(|pair| !pair.is_empty() && !pair.starts_with(&format!("{PREVIEW_COOKIE}=")))
        .map(str::to_owned)
        .collect();
    headers.remove(header::COOKIE);
    if !kept.is_empty()
        && let Ok(value) = HeaderValue::from_str(&kept.join("; "))
    {
        headers.insert(header::COOKIE, value);
    }
}

/// How long a preview request waits for a frozen workspace to thaw.
const THAW: Duration = Duration::from_secs(25);

/// A page that retries while the workspace comes up, or says why it won't.
fn thawing(phase: Phase) -> Response {
    let (text, retry) = match phase {
        Phase::Stopped | Phase::Stopping => (
            "This workspace is stopped. Start it from the console.",
            false,
        ),
        Phase::Deleting | Phase::Deleted => ("This workspace is being deleted.", false),
        Phase::Creating | Phase::Starting | Phase::Running | Phase::Freezing | Phase::Frozen => {
            ("Thawing…", true)
        }
    };
    let refresh = if retry {
        "<meta http-equiv=\"refresh\" content=\"3\">"
    } else {
        ""
    };
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        format!(
            "<!doctype html><meta charset=\"utf-8\">{refresh}<title>{text}</title><p>{text}</p>"
        ),
    )
        .into_response()
}

fn reject(status: StatusCode, message: &str) -> Response {
    (status, message.to_owned()).into_response()
}

pub async fn handle(app: Arc<App>, label: String, request: Request) -> Response {
    if label == PREVIEW_AUTH_LABEL {
        let path = request.uri().path().to_owned();
        let query = request.uri().query().map(str::to_owned);
        return crate::login::preview_auth(&app, &path, query.as_deref(), request.headers()).await;
    }
    let Ok(name) = label.parse::<RouteName>() else {
        return reject(StatusCode::NOT_FOUND, "no such preview");
    };
    let lookup = name.clone();
    let route = match app.db.call(move |tx| db::route_by_name(tx, &lookup)).await {
        Ok(Some(route)) => route,
        Ok(None) => return reject(StatusCode::NOT_FOUND, "no such preview"),
        Err(_) => return reject(StatusCode::SERVICE_UNAVAILABLE, "try again"),
    };
    let workspace_id = route.workspace;
    let Ok(Some(ws)) = app.db.call(move |tx| db::workspace(tx, workspace_id)).await else {
        return reject(StatusCode::NOT_FOUND, "no such preview");
    };

    let kind = transport(request.method(), request.headers());
    let session = match cookie(request.headers(), PREVIEW_COOKIE) {
        Some(token) => session_principal(&app, token, SessionKind::Preview)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let Some((principal, _)) = session else {
        return match kind {
            Transport::Http(MethodClass::Safe) => {
                let here = format!(
                    "https://{label}.{}{}",
                    app.config.preview_domain,
                    request.uri()
                );
                let encoded: String =
                    url::form_urlencoded::byte_serialize(here.as_bytes()).collect();
                Redirect::to(&format!(
                    "https://{PREVIEW_AUTH_LABEL}.{}/login?return={encoded}",
                    app.config.preview_domain
                ))
                .into_response()
            }
            Transport::Http(MethodClass::Unsafe) | Transport::WebSocket => {
                reject(StatusCode::UNAUTHORIZED, "sign in first")
            }
        };
    };
    match authorize(
        principal.principal(),
        Action::UsePreview,
        Resource { owner: ws.owner },
    ) {
        Decision::Allow => {}
        // Other people's previews look missing, not forbidden.
        Decision::Deny(DenyReason::NotOwner) => {
            return reject(StatusCode::NOT_FOUND, "no such preview");
        }
        Decision::Deny(reason) => return reject(StatusCode::FORBIDDEN, &reason.to_string()),
    }

    let from = source(&app, request.headers(), &name, ws.id).await;
    match preview_access(kind, fetch_metadata(request.headers()), from) {
        PreviewAccess::Forward => {}
        PreviewAccess::Reject => {
            return reject(StatusCode::FORBIDDEN, "cross-workspace request refused");
        }
    }
    let Some(host) = app.host(&ws.host) else {
        return reject(StatusCode::SERVICE_UNAVAILABLE, "host unavailable");
    };
    // A request is a use, and one to a frozen workspace thaws it.
    if !crate::idle::thaw(&app, &ws, THAW).await {
        return thawing(ws.phase());
    }
    match forward(&app, &host, ws.id, route.port, &name, request, kind).await {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!(route = %name, %error, "preview upstream failed");
            reject(
                StatusCode::BAD_GATEWAY,
                "the app didn't answer; is it listening on that port?",
            )
        }
    }
}

async fn connect(
    host: &crate::hosts::HostClient,
    workspace: WorkspaceId,
    port: GuestPort,
) -> anyhow::Result<SendRequest<Body>> {
    let stream = host.tunnel(workspace, port).await?;
    let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        if let Err(error) = connection.with_upgrades().await {
            tracing::debug!(%error, "preview connection closed");
        }
    });
    Ok(sender)
}

async fn forward(
    app: &App,
    host: &crate::hosts::HostClient,
    workspace: WorkspaceId,
    port: GuestPort,
    name: &RouteName,
    mut request: Request,
    kind: Transport,
) -> anyhow::Result<Response> {
    let websocket = matches!(kind, Transport::WebSocket);
    let own_origin = app.config.preview_origin(name.as_str());
    let public_host = format!("{name}.{}", app.config.preview_domain);
    let client_upgrade = websocket.then(|| hyper::upgrade::on(&mut request));

    let (mut parts, body) = request.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map_or("/", |pq| pq.as_str())
        .to_owned();
    parts.uri = Uri::try_from(path)?;
    let headers = &mut parts.headers;
    strip_hop_by_hop(headers, websocket);
    strip_platform_cookie(headers);
    for forwarded in [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-real-ip",
    ] {
        headers.remove(forwarded);
    }
    // Dev servers usually only trust localhost, so the app sees itself as
    // local; the gateway already enforced who may reach it.
    let local = format!("localhost:{port}");
    headers.insert(header::HOST, HeaderValue::from_str(&local)?);
    headers.insert("x-forwarded-host", HeaderValue::from_str(&public_host)?);
    headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
    if headers
        .get(header::ORIGIN)
        .is_some_and(|o| o.as_bytes() == own_origin.as_bytes())
    {
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_str(&format!("http://{local}"))?,
        );
    }
    let upstream_request = Request::from_parts(parts, body);

    let key = (workspace, port);
    let mut sender = match (websocket, app.pool.take(key)) {
        (false, Some(sender)) => sender,
        (true, _) | (false, None) => connect(host, workspace, port).await?,
    };
    let mut response = sender.send_request(upstream_request).await?;
    if !websocket {
        app.pool.put(key, sender);
    }

    if let (Some(client_upgrade), StatusCode::SWITCHING_PROTOCOLS) =
        (client_upgrade, response.status())
    {
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        tokio::spawn(async move {
            match (client_upgrade.await, upstream_upgrade.await) {
                (Ok(client), Ok(upstream)) => {
                    let _ = tokio::io::copy_bidirectional(
                        &mut TokioIo::new(client),
                        &mut TokioIo::new(upstream),
                    )
                    .await;
                }
                (Err(error), _) | (_, Err(error)) => {
                    tracing::debug!(%error, "preview upgrade failed");
                }
            }
        });
        let (mut parts, _) = response.into_parts();
        filter_app_cookies(&mut parts.headers);
        return Ok(Response::from_parts(parts, Body::empty()));
    }

    let (mut parts, body) = response.into_parts();
    strip_hop_by_hop(&mut parts.headers, false);
    filter_app_cookies(&mut parts.headers);
    Ok(Response::from_parts(parts, Body::new(body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hop_by_hop_headers_are_removed() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONNECTION,
            "keep-alive, x-secret".parse().expect("valid"),
        );
        headers.insert("x-secret", "1".parse().expect("valid"));
        headers.insert("keep-alive", "timeout=5".parse().expect("valid"));
        headers.insert("x-app", "ok".parse().expect("valid"));
        strip_hop_by_hop(&mut headers, false);
        assert!(headers.get("x-secret").is_none());
        assert!(headers.get("keep-alive").is_none());
        assert!(headers.get(header::CONNECTION).is_none());
        assert_eq!(
            headers.get("x-app").map(http::HeaderValue::as_bytes),
            Some(&b"ok"[..])
        );
    }

    #[test]
    fn the_platform_cookie_never_reaches_apps() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("app=1; {PREVIEW_COOKIE}=secret; other=2")
                .parse()
                .expect("valid"),
        );
        strip_platform_cookie(&mut headers);
        assert_eq!(
            headers.get(header::COOKIE).map(|v| v.to_str().ok()),
            Some(Some("app=1; other=2"))
        );
        let mut only = HeaderMap::new();
        only.insert(
            header::COOKIE,
            format!("{PREVIEW_COOKIE}=secret").parse().expect("valid"),
        );
        strip_platform_cookie(&mut only);
        assert!(only.get(header::COOKIE).is_none());
    }
}
