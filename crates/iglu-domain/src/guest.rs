//! The state the guest tools keep under the workspace user's runtime
//! directory (`$XDG_RUNTIME_DIR`), which hosts read. `iglu-guest` and
//! `iglu-status` write these files; hostd reads them, relative to wherever
//! its runtime puts that directory.
//!
//! The runtime directory is per boot, so everything here starts empty when
//! a workspace starts.

use crate::attention::{AttentionState, Summary};

/// Written last by `iglu-guest install-secrets`: the secrets delivery the
/// guest holds, as a decimal number.
pub const SECRETS_GENERATION: &str = "iglu/secrets/generation";

/// The environment variable that marks a process as one of a local
/// workspace's, on the dev stack's local runtime, where every workspace's
/// processes are the same user's. Its value is the instance's name.
pub const LOCAL_WORKSPACE: &str = "IGLU_DEVHOST_WORKSPACE";

/// Per-session attention status, maintained by `iglu-status`: sessions,
/// each with its threads, each a [`StatusEntry`].
pub const STATUS: &str = "iglu/status/sessions.json";

/// One thread's entry in the status file. Hosts read it as untrusted and
/// parse it again.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StatusEntry {
    pub state: AttentionState,
    pub summary: Summary,
    pub title: Summary,
    /// Unix milliseconds, by the guest's clock.
    pub at: i64,
}

/// What `iglu-guest git-state` prints for a checkout; `null` without one.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitReport {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changed: u32,
    pub untracked: u32,
    pub conflicted: u32,
}

/// One entry of what `iglu-guest listeners` prints.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ListenerReport {
    pub port: u16,
    pub address: std::net::IpAddr,
    pub process: String,
    /// The zmx session the process runs in, if it can be told.
    pub session: Option<String>,
}

/// Written by `iglu-guest open --boot` once a boot has opened the
/// workspace's columns.
pub const COLUMNS_OPENED: &str = "iglu/columns-opened";

/// The version of the interface between hosts and the guest tools: the
/// tools' command line and the files above. An image records the version
/// its tools speak, and a host only uses images that speak its own. Bump it
/// with any change either side would notice.
pub const INTERFACE: Interface = Interface(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Interface(u32);

impl Interface {
    #[must_use]
    pub const fn new(version: u32) -> Self {
        Self(version)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for Interface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// An image whose guest tools a host can't drive. The message is for the
/// environment's owner.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "the environment was built with a different iglu ({found}) than this host's (interface {INTERFACE}); \
     run `nix flake update iglu` in the environment's flake, push it, and rebuild the environment"
)]
pub struct Incompatible {
    /// What the image says, if it says anything.
    pub found: Found,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// Images from before interfaces were recorded.
    Unrecorded,
    Other(Interface),
}

impl std::fmt::Display for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unrecorded => f.write_str("no interface recorded"),
            Self::Other(interface) => write!(f, "interface {interface}"),
        }
    }
}

/// Whether a host can use an image whose tools speak `recorded`.
///
/// # Errors
///
/// When the image records another interface, or none.
pub fn compatible(recorded: Option<Interface>) -> Result<(), Incompatible> {
    match recorded {
        Some(interface) if interface == INTERFACE => Ok(()),
        Some(interface) => Err(Incompatible {
            found: Found::Other(interface),
        }),
        None => Err(Incompatible {
            found: Found::Unrecorded,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_images_speaking_this_hosts_interface_are_used() {
        assert_eq!(compatible(Some(INTERFACE)), Ok(()));
        let older = Interface(INTERFACE.get() - 1);
        assert_eq!(
            compatible(Some(older)),
            Err(Incompatible {
                found: Found::Other(older)
            })
        );
        let unrecorded = compatible(None).expect_err("an unrecorded image is refused");
        assert!(
            unrecorded.to_string().contains("nix flake update iglu"),
            "{unrecorded}"
        );
    }
}
