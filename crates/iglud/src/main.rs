//! iglud: the control plane. Serves the console, the API and the preview
//! gateway, and runs the reconciler that keeps hosts in line with intent.

mod api;
mod app;
mod backup;
mod browser;
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
#[cfg(test)]
mod sim;
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
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_platform_verifier::Verifier;
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

/// Lets browsers keep an asset without asking again: a new build names a
/// changed asset differently.
async fn keep_for_good(mut response: Response) -> Response {
    if response.status().is_success() {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    response
}

/// Every route iglud serves. The console's assets are public, and named for
/// what's in them, so browsers may keep them for good; its page, for every
/// other path, asks for a session first and is checked on every load.
fn router(app: Arc<App>, assets: &std::path::Path) -> Router {
    let kept = Router::new()
        .nest_service("/assets", ServeDir::new(assets.join("assets")))
        .layer(middleware::map_response(keep_for_good));
    Router::new()
        .merge(api::router())
        .merge(login::console_router())
        .merge(kept)
        .fallback(login::console_page)
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

    let verifier = verifier(&config)?;
    let http = reqwest::Client::builder()
        .use_preconfigured_tls(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(verifier.clone())
                .with_no_client_auth(),
        )
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(4000))
        .build()?;

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
            "{}/callback",
            config.preview_origin(iglu_domain::label::PREVIEW_AUTH_LABEL)
        ))?,
    );
    // Discover the provider now, so the first sign-in doesn't wait for it.
    // Failing here isn't fatal: sign-in and host calls retry on demand.
    tokio::spawn(async move {
        if let Err(error) = provider.metadata().await {
            tracing::warn!(%error, "the identity provider isn't reachable yet");
        }
    });
    let hosts = host_clients(&config, verifier)?;

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

/// How iglud checks the servers it calls: against the system's trust
/// store, and the configured extra CAs.
fn verifier(config: &config::Config) -> anyhow::Result<Arc<Verifier>> {
    let extra = match &config.extra_ca_certificates {
        Some(path) => CertificateDer::pem_file_iter(path)
            .and_then(Iterator::collect)
            .with_context(|| format!("reading {}", path.display()))?,
        None => Vec::new(),
    };
    let provider = rustls::crypto::CryptoProvider::get_default()
        .context("no TLS provider is installed")?
        .clone();
    Ok(Arc::new(Verifier::new_with_extra_roots(extra, provider)?))
}

/// A client for each host. Hosts' certificates are checked by `verifier`,
/// and they check iglud's own.
fn host_clients(
    config: &config::Config,
    verifier: Arc<Verifier>,
) -> anyhow::Result<Vec<Arc<hosts::HostClient>>> {
    let tls = Arc::new(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_client_auth_cert(
                CertificateDer::pem_file_iter(&config.host_client.certificate)
                    .and_then(Iterator::collect)
                    .context("reading the host client certificate")?,
                PrivateKeyDer::from_pem_file(&config.host_client.key)
                    .context("reading the host client key")?,
            )?,
    );
    let http = reqwest::Client::builder()
        .use_preconfigured_tls((*tls).clone())
        .timeout(std::time::Duration::from_secs(4000))
        .build()?;
    Ok(config
        .hosts
        .iter()
        .map(|host| {
            Arc::new(hosts::HostClient::new(
                host.id.clone(),
                host.url.as_url().clone(),
                http.clone(),
                tls.clone(),
            ))
        })
        .collect())
}
