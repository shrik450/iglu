//! iglud: the control plane. Serves the console, the API and the preview
//! gateway, and runs the reconciler that keeps hosts in line with intent.

mod api;
mod app;
mod config;
mod crypto;
mod db;
mod gateway;
mod hosts;
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
use tower_http::services::{ServeDir, ServeFile};
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
    match gateway::preview_label(&app, &host).map(str::to_owned) {
        Some(label) => gateway::handle(app, label, request).await,
        None => {
            let mut response = next.run(request).await;
            let headers = response.headers_mut();
            let set =
                |headers: &mut axum::http::HeaderMap, name: &'static str, value: &'static str| {
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
    let db = db::Db::open(&config.database).context("opening the database")?;
    let sealer = crypto::Sealer::load(&config.secret_key_file).context("loading the secret key")?;

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(4000))
        .build()?;
    let tls = Arc::new(rustls::ClientConfig::with_platform_verifier()?);

    let console_callback = config.console_origin.join("/auth/callback")?;
    let console = oidc::RelyingParty::discover(
        http.clone(),
        &config.oidc.issuer,
        config.oidc.console.client_id.clone(),
        config.oidc.console.secret()?,
        console_callback,
    )
    .await
    .context("setting up console sign-in")?;
    let preview_callback = url::Url::parse(&format!(
        "https://{}.{}/callback",
        iglu_domain::label::PREVIEW_AUTH_LABEL,
        config.preview_domain
    ))?;
    let preview = oidc::RelyingParty::discover(
        http.clone(),
        &config.oidc.issuer,
        config.oidc.preview.client_id.clone(),
        config.oidc.preview.secret()?,
        preview_callback,
    )
    .await
    .context("setting up preview sign-in")?;
    let token_endpoint = console
        .token_endpoint()
        .cloned()
        .context("the IdP has no token endpoint")?;
    let tokens = Arc::new(oidc::WorkerTokens::new(
        http.clone(),
        token_endpoint,
        config.oidc.worker.client_id.clone(),
        config.oidc.worker.secret()?,
        config.oidc.worker.audience.clone(),
    ));
    let hosts = config
        .hosts
        .iter()
        .map(|host| {
            Arc::new(hosts::HostClient::new(
                host.id.clone(),
                host.url.clone(),
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
    });
    tokio::spawn(reconcile::run(app.clone()));

    let console_app = ServeDir::new(&assets).fallback(ServeFile::new(assets.join("index.html")));
    let router = Router::new()
        .merge(api::router())
        .merge(login::console_router())
        .fallback_service(console_app)
        .layer(middleware::from_fn_with_state(app.clone(), dispatch))
        .with_state(app);

    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, "iglud listening");
    axum::serve(listener, router).await?;
    Ok(())
}
