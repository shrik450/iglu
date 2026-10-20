//! How the CLI checks iglu's certificate: against the system's trust store,
//! and the CAs in `IGLU_EXTRA_CA_CERTIFICATES`, for a deployment whose CA
//! the system doesn't trust, as iglud's `extra_ca_certificates` is.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use rustls::ClientConfig;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use rustls_platform_verifier::Verifier;

/// A file of PEM certificates to trust as well as the system's.
pub const EXTRA_CA_ENV: &str = "IGLU_EXTRA_CA_CERTIFICATES";

pub fn config() -> anyhow::Result<ClientConfig> {
    let extra = match std::env::var_os(EXTRA_CA_ENV) {
        Some(path) => CertificateDer::pem_file_iter(&path)
            .and_then(Iterator::collect)
            .with_context(|| format!("reading {EXTRA_CA_ENV}, {}", Path::new(&path).display()))?,
        None => Vec::new(),
    };
    let provider = rustls::crypto::CryptoProvider::get_default()
        .context("no TLS provider is installed")?
        .clone();
    let verifier = Verifier::new_with_extra_roots(extra, provider.clone())?;
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}
