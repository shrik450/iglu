//! `iglu-devhost --config <path>`: hostd over the local runtime, serving
//! iglud on loopback. `just dev` runs it.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use iglu_devhost::config::Config;
use iglu_devhost::local::LocalRuntime;
use iglu_hostd::auth::Verifier;
use iglu_hostd::host::Host;
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
        .context("usage: iglu-devhost --config <path>")?;
    let config: Config = iglu_hostd::config::load(&config_path)?;

    let runtime = LocalRuntime::new(&config.runtime, config.timeouts).await?;
    let app = Arc::new(App {
        host_id: config.host_id,
        arch: iglu_hostd::host_arch().context("unsupported host architecture")?,
        host: Host::new(runtime, config.timeouts),
        verifier: Verifier::from_config(config.auth)?,
    });
    server::serve_loopback(app, config.listen).await?;
    Ok(())
}
