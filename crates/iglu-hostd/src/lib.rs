//! hostd: the execution host's agent. It exposes a fixed set of typed
//! commands to iglud and carries them out through a [`runtime::Runtime`].
//! It keeps no state of its own: the runtime is the record of what exists.
//!
//! The `iglu-hostd` binary runs workspaces on Incus. Other runtimes, such
//! as the local one for development, reuse everything here but the runtime.

pub mod auth;
pub mod config;
#[cfg(feature = "conformance")]
pub mod conformance;
pub mod guest;
pub mod host;
pub mod incus;
mod rules;
pub mod runtime;
pub mod server;
mod terminal;

use iglu_domain::ParseError;
use iglu_domain::env::Arch;

/// The architecture this host runs, and so the one its images must be for.
///
/// # Errors
///
/// When hostd runs on an architecture iglu doesn't support.
pub fn host_arch() -> Result<Arch, ParseError> {
    std::env::consts::ARCH.parse()
}
