//! iglud: the control plane. Serves the console, the API and the preview
//! gateway, and runs the reconciler that keeps hosts in line with intent.

mod api;
mod app;
mod backup;
mod config;
mod crypto;
mod db;
mod extract;
mod gateway;
mod guest;
mod hosts;
mod idle;
mod login;
mod model;
mod oidc;
mod reconcile;
mod terminal;
mod views;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use rustls_platform_verifier::ConfigVerifierExt;
use tower_http::services::ServeDir;
use tracing_subscriber::EnvFilter;

use crate::app::App;

/// Serves preview hosts through the gateway; everything else is the console.
async fn dispatch(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_default();
    if let Some(label) = gateway::preview_label(&app, &host).map(str::to_owned) {
        gateway::handle(app, label, request).await
    } else {
        let mut response = next.run(request).await;
        let headers = response.headers_mut();
        let set = |headers: &mut axum::http::HeaderMap, name: &'static str, value: &'static str| {
            headers.insert(name, HeaderValue::from_static(value));
        };
        set(
            headers,
            "content-security-policy",
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self' http://127.0.0.1:*",
        );
        set(headers, "x-frame-options", "DENY");
        set(headers, "x-content-type-options", "nosniff");
        set(headers, "referrer-policy", "same-origin");
        set(headers, "strict-transport-security", "max-age=31536000");
        response
    }
}

/// Every route iglud serves. The console's files are public; its page, for
/// every path the console routes, asks for a session first.
fn router(app: Arc<App>, assets: &std::path::Path) -> Router {
    let console_app = ServeDir::new(assets)
        .append_index_html_on_directories(false)
        .fallback(axum::routing::get(login::console_page).with_state(app.clone()));
    Router::new()
        .merge(api::router())
        .merge(login::console_router())
        .fallback_service(console_app)
        .layer(middleware::from_fn_with_state(app.clone(), dispatch))
        .with_state(app)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("a TLS provider was already installed"))?;

    let config_path: PathBuf = std::env::args_os()
        .skip_while(|arg| arg != "--config")
        .nth(1)
        .map(PathBuf::from)
        .context("usage: iglud --config <path>")?;
    let config = config::Config::load(&config_path)?;
    // Copy the database before opening it, so an upgrade's migrations can be undone.
    if let Some(policy) = &config.backups
        && config.database.exists()
    {
        match backup::back_up(&config.database, policy, app::now()) {
            Ok(path) => tracing::info!(path = %path.display(), "backed up the database"),
            Err(error) => tracing::warn!(%error, "couldn't back up the database"),
        }
    }
    let db = db::Db::open(&config.database).context("opening the database")?;
    let sealer = crypto::Sealer::load(&config.secret_key_file).context("loading the secret key")?;

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(4000))
        .build()?;
    let tls = Arc::new(rustls::ClientConfig::with_platform_verifier()?);

    let provider = Arc::new(oidc::Provider::new(http.clone(), &config.oidc.issuer)?);
    let console = oidc::RelyingParty::new(
        provider.clone(),
        config.oidc.console.client_id.clone(),
        config.oidc.console.secret()?,
        config.console_origin.join("/auth/callback")?,
    );
    let preview = oidc::RelyingParty::new(
        provider.clone(),
        config.oidc.preview.client_id.clone(),
        config.oidc.preview.secret()?,
        url::Url::parse(&format!(
            "https://{}.{}/callback",
            iglu_domain::label::PREVIEW_AUTH_LABEL,
            config.preview_domain
        ))?,
    );
    let tokens = Arc::new(oidc::WorkerTokens::new(
        provider.clone(),
        config.oidc.worker.client_id.clone(),
        config.oidc.worker.secret()?,
        config.oidc.worker.resource.clone(),
    ));
    // Discover the provider now, so the first sign-in doesn't wait for it.
    // Failing here isn't fatal: sign-in and host calls retry on demand.
    tokio::spawn(async move {
        if let Err(error) = provider.metadata().await {
            tracing::warn!(%error, "the identity provider isn't reachable yet");
        }
    });
    let hosts = config
        .hosts
        .iter()
        .map(|host| {
            Arc::new(hosts::HostClient::new(
                host.id.clone(),
                host.url.as_url().clone(),
                http.clone(),
                tls.clone(),
                tokens.clone(),
            ))
        })
        .collect();

    let assets = config.console_assets.clone();
    let listen = config.listen;
    let (changes, _) = tokio::sync::broadcast::channel(64);
    let app = Arc::new(App {
        config,
        db,
        sealer,
        console,
        preview,
        hosts,
        changes,
        host_seen: Mutex::default(),
        reconciler: Arc::default(),
        pool: gateway::Pool::default(),
        usage: idle::Usage::new(app::now()),
        boot: iglu_api::BootId::from_uuid(uuid::Uuid::new_v4()),
        column_edits: app::ColumnEdits::default(),
        uploads: tokio::sync::Semaphore::new(4),
    });
    tokio::spawn(reconcile::run(app.clone()));
    guest::serve(&app);
    if let Some(policy) = app.config.backups.clone() {
        tokio::spawn(backup::run(app.config.database.clone(), policy));
    }

    let router = router(app, &assets);

    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, "iglud listening");
    axum::serve(listener, router).await?;
    Ok(())
}
