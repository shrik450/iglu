//! `iglu-hostd`: the execution host's only privileged service, running
//! workspaces on Incus.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use iglu_hostd::auth::Verifier;
use iglu_hostd::config::Config;
use iglu_hostd::host::Host;
use iglu_hostd::incus::IncusRuntime;
use iglu_hostd::server::{self, App};
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
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        &config.tls.certificate,
        &config.tls.key,
    )
    .await
    .context("loading the TLS certificate")?;
    let app = Arc::new(App {
        host_id: config.host_id,
        arch: iglu_hostd::host_arch().context("unsupported host architecture")?,
        host: Host::new(runtime, config.timeouts),
        verifier: Verifier::from_config(config.auth)?,
    });
    server::serve_tls(app, config.listen, tls).await?;
    Ok(())
}
