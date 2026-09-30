//! hostd: the execution host's only privileged service.
//!
//! It exposes a fixed set of typed commands to iglud and performs them
//! against the local Incus daemon. It keeps no state of its own: Incus is
//! the record of what exists.

mod auth;
mod build;
mod commands;
mod config;
mod exec;
mod guestfs;
mod incus;
mod observe;
mod policy;
mod reclaim;
mod server;
mod terminal;
mod tunnel;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use iglu_domain::env::Arch;
use tracing_subscriber::EnvFilter;

fn host_arch() -> anyhow::Result<Arch> {
    std::env::consts::ARCH
        .parse()
        .map_err(|e| anyhow::anyhow!("unsupported host architecture: {e}"))
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
        .context("usage: hostd --config <path>")?;
    let config = config::Config::load(&config_path)?;

    let incus = incus::Incus::new(config.incus.socket.clone(), config.incus.project.clone());
    policy::ensure(
        &incus,
        &config.incus.network,
        &config.incus.acl,
        config.timeouts.operation(),
    )
    .await
    .context("applying the workspace egress policy")?;
    tokio::fs::create_dir_all(config.runtime_dir.join("proxy")).await?;

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let verifier = auth::Verifier::new(
        config.auth.issuer.clone(),
        config.auth.audience.clone(),
        config.auth.subjects.clone(),
        http,
    );
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        &config.tls.certificate,
        &config.tls.key,
    )
    .await
    .context("loading the TLS certificate")?;
    let listen = config.listen;
    let app = Arc::new(server::App {
        config,
        incus,
        boots: observe::BootCache::default(),
        verifier,
        arch: host_arch()?,
    });

    tracing::info!(%listen, "hostd listening");
    axum_server::bind_rustls(listen, tls)
        .serve(server::router(app).into_make_service())
        .await?;
    Ok(())
}
