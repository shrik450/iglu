//! Ports something in a workspace is listening on, as candidates for a
//! preview. Guests report them; whether one can be published is decided here.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::parse::is_printable;
use crate::port::GuestPort;
use crate::terminal::SessionName;

/// A process's short name, as the guest reports it. Untrusted: printable and
/// bounded, so it can't smuggle escapes into a terminal or the console.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct ProcessName(String);

/// Off the wire, only a name that's already clean: deserializing parses.
impl TryFrom<String> for ProcessName {
    type Error = ParseError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let clean = Self::sanitize(&s);
        if clean.0 == s {
            Ok(clean)
        } else {
            Err(ParseError::new(
                "process name",
                "must be printable and at most 32 characters",
            ))
        }
    }
}

impl From<ProcessName> for String {
    fn from(name: ProcessName) -> Self {
        name.0
    }
}

impl ProcessName {
    pub const MAX_CHARS: usize = 32;

    #[must_use]
    pub fn sanitize(s: &str) -> Self {
        Self(
            s.trim()
                .chars()
                .filter(|c| is_printable(*c))
                .take(Self::MAX_CHARS)
                .collect(),
        )
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Something listening on a port in a workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Listener {
    pub port: GuestPort,
    pub process: ProcessName,
    /// The column it was started from, when it can be told.
    pub column: Option<SessionName>,
    /// Whether a preview can reach it: previews connect to `127.0.0.1`.
    pub reachable: bool,
}

/// Whether a server bound to `address` answers on `127.0.0.1`: any IPv4
/// loopback or wildcard address, or the IPv6 wildcard, which Linux shares
/// with IPv4 unless told not to. `::1` alone and outside addresses don't.
#[must_use]
pub fn reachable(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_unspecified(),
        IpAddr::V6(v6) => {
            v6.is_unspecified()
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| v4.is_loopback() || v4.is_unspecified())
        }
    }
}

/// One listener per port: a server that binds both IPv4 and IPv6 shows once,
/// reachable if either is, with the column of whichever entry knows it.
/// Ordered by port, at most `limit`.
#[must_use]
pub fn merge(listeners: Vec<Listener>, limit: usize) -> Vec<Listener> {
    let mut merged: std::collections::BTreeMap<GuestPort, Listener> =
        std::collections::BTreeMap::new();
    for listener in listeners {
        match merged.get_mut(&listener.port) {
            Some(known) => {
                known.reachable |= listener.reachable;
                if known.column.is_none() {
                    known.column = listener.column;
                }
            }
            None => {
                merged.insert(listener.port, listener);
            }
        }
    }
    merged.into_values().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("an address")
    }

    #[test]
    fn loopback_and_wildcards_are_reachable() {
        for address in [
            "127.0.0.1",
            "127.0.1.1",
            "0.0.0.0",
            "::",
            "::ffff:127.0.0.1",
        ] {
            assert!(reachable(ip(address)), "{address}");
        }
        for address in ["::1", "10.0.0.5", "fe80::1"] {
            assert!(!reachable(ip(address)), "{address}");
        }
    }

    #[test]
    fn a_port_shows_once() {
        let port = |p: u16| GuestPort::try_from(p).expect("a port");
        let listener = |p, reachable, column: Option<&str>| Listener {
            port: port(p),
            process: ProcessName::sanitize("node"),
            column: column.map(|c| c.parse().expect("a session")),
            reachable,
        };
        let merged = merge(
            vec![
                listener(5173, false, None),
                listener(3000, true, None),
                listener(5173, true, Some("server")),
            ],
            10,
        );
        assert_eq!(
            merged
                .iter()
                .map(|l| (l.port.get(), l.reachable))
                .collect::<Vec<_>>(),
            [(3000, true), (5173, true)]
        );
        assert_eq!(
            merged[1].column.as_ref().map(SessionName::as_str),
            Some("server")
        );
        assert_eq!(merge(merged, 1).len(), 1);
    }

    #[test]
    fn only_clean_names_come_off_the_wire() {
        assert!(serde_json::from_str::<ProcessName>(r#""vite""#).is_ok());
        assert!(serde_json::from_str::<ProcessName>(r#""vite[2J""#).is_err());
        assert!(serde_json::from_str::<ProcessName>(&format!("\"{}\"", "x".repeat(33))).is_err());
    }

    #[test]
    fn process_names_are_sanitized() {
        assert_eq!(
            ProcessName::sanitize(" vite\u{1b}[31m\n").as_str(),
            "vite[31m"
        );
        assert_eq!(
            ProcessName::sanitize(&"x".repeat(99)).as_str().len(),
            ProcessName::MAX_CHARS
        );
    }
}
