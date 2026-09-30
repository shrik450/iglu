//! Names that end up in DNS, URLs and Incus: lowercase DNS labels.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::parse::text_type;

/// One DNS label: 1–63 characters of `a-z`, `0-9` and `-`, not starting or
/// ending with `-`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DnsLabel(String);

impl DnsLabel {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn parse_as(s: &str, what: &'static str) -> Result<Self, ParseError> {
        if s.is_empty() || s.len() > 63 {
            return Err(ParseError::new(what, "must be 1 to 63 characters"));
        }
        if !s
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        {
            return Err(ParseError::new(what, "may only contain a-z, 0-9 and '-'"));
        }
        if s.starts_with('-') || s.ends_with('-') {
            return Err(ParseError::new(what, "may not start or end with '-'"));
        }
        Ok(Self(s.to_owned()))
    }

    fn starts_with_letter(&self) -> bool {
        self.0
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase())
    }
}

impl fmt::Display for DnsLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for DnsLabel {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_as(s, "DNS label")
    }
}

text_type!(DnsLabel);

macro_rules! label_type {
    ($(#[$meta:meta])* $name:ident, $what:literal, $check:expr) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(DnsLabel);

        impl $name {
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            pub fn label(&self) -> &DnsLabel {
                &self.0
            }
        }

        impl TryFrom<DnsLabel> for $name {
            type Error = ParseError;

            fn try_from(label: DnsLabel) -> Result<Self, Self::Error> {
                let check: fn(&DnsLabel) -> Result<(), &'static str> = $check;
                check(&label).map_err(|reason| ParseError::new($what, reason))?;
                Ok(Self(label))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = ParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                DnsLabel::parse_as(s, $what)?.try_into()
            }
        }

        text_type!($name);
    };
}

label_type!(
    /// Identifies an execution host.
    HostId,
    "host ID",
    |_| Ok(())
);

label_type!(
    /// A workspace's human-facing name. Starts with a letter so it reads as a
    /// name and is valid as a Git branch and a directory.
    WorkspaceName,
    "workspace name",
    |label| {
        if label.starts_with_letter() {
            Ok(())
        } else {
            Err("must start with a letter")
        }
    }
);

label_type!(
    /// The hostname label of a preview route: `<name>.<preview domain>`.
    RouteName,
    "route name",
    |label| {
        if RESERVED_ROUTE_LABELS.contains(&label.as_str()) {
            Err("is reserved for the preview gateway")
        } else if !label.starts_with_letter() {
            Err("must start with a letter")
        } else {
            Ok(())
        }
    }
);

/// Labels in the preview domain that belong to the gateway itself.
pub const RESERVED_ROUTE_LABELS: &[&str] = &["auth"];

/// The gateway's own sign-in host label in the preview domain.
pub const PREVIEW_AUTH_LABEL: &str = "auth";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_dns_rules() {
        for ok in ["a", "quiet-running", "a1", "0day", &"a".repeat(63)] {
            assert!(ok.parse::<DnsLabel>().is_ok(), "{ok}");
        }
        for bad in ["", "-a", "a-", "Upper", "a_b", "a.b", "é", &"a".repeat(64)] {
            assert!(bad.parse::<DnsLabel>().is_err(), "{bad}");
        }
    }

    #[test]
    fn workspace_and_route_names_start_with_a_letter() {
        assert!("0day".parse::<WorkspaceName>().is_err());
        assert!("0day".parse::<RouteName>().is_err());
        assert!("swift-dancing".parse::<RouteName>().is_ok());
    }

    #[test]
    fn the_gateway_label_is_not_a_route_name() {
        assert!(PREVIEW_AUTH_LABEL.parse::<RouteName>().is_err());
    }

    #[test]
    fn deserializing_parses() {
        let parsed: Result<RouteName, _> = serde_json::from_str("\"Bad Name\"");
        assert!(parsed.is_err());
        let parsed: RouteName = serde_json::from_str("\"calm-coding\"").expect("valid route name");
        assert_eq!(parsed.as_str(), "calm-coding");
    }
}
