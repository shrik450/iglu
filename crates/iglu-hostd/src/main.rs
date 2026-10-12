//! `iglu-hostd`: the execution host's only privileged service, running
//! workspaces on Incus.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum_server::tls_rustls::RustlsConfig;
use iglu_hostd::config::Config;
use iglu_hostd::host::Host;
use iglu_hostd::incus::IncusRuntime;
use iglu_hostd::server::{self, App};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use tracing_subscriber::EnvFilter;

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
        .context("usage: iglu-hostd --config <path>")?;
    let config: Config = iglu_hostd::config::load(&config_path)?;

    let runtime = IncusRuntime::new(
        config.incus,
        config.build,
        &config.runtime_dir,
        config.timeouts,
    )
    .await
    .context("applying the workspace egress policy")?;
    let tls = tls(&config.tls).context("loading the TLS certificates")?;
    let app = Arc::new(App {
        host_id: config.host_id,
        arch: iglu_hostd::host_arch().context("unsupported host architecture")?,
        host: Host::new(runtime, config.timeouts),
    });
    server::serve_tls(app, config.listen, tls).await?;
    Ok(())
}

/// The host's certificate, and only clients with a certificate from
/// `tls.clients`: iglud's.
fn tls(tls: &iglu_hostd::config::Tls) -> anyhow::Result<RustlsConfig> {
    let mut clients = rustls::RootCertStore::empty();
    for ca in CertificateDer::pem_file_iter(&tls.clients)? {
        clients.add(ca?)?;
    }
    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(WebPkiClientVerifier::builder(Arc::new(clients)).build()?)
        .with_single_cert(
            CertificateDer::pem_file_iter(&tls.certificate)?.collect::<Result<_, _>>()?,
            PrivateKeyDer::from_pem_file(&tls.key)?,
        )?;
    Ok(RustlsConfig::from_config(Arc::new(config)))
}
