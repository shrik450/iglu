//! Opaque identifiers. An ID is a locator, never a capability.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ParseError;
use crate::parse::text_type;

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Wraps a UUID. The shell chooses fresh UUIDs; the core never generates them.
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.hyphenated().fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = ParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::try_parse(s)
                    .map(Self)
                    .map_err(|_| ParseError::new(stringify!($name), "not a UUID"))
            }
        }
    };
}

uuid_id!(
    /// A signed-in person or service, keyed by OIDC issuer and subject.
    PrincipalId
);
uuid_id!(
    /// One workspace: one task, one container, one lifetime.
    WorkspaceId
);
uuid_id!(
    /// One built revision of an environment.
    EnvRevisionId
);
uuid_id!(
    /// One published preview route.
    RouteId
);
uuid_id!(
    /// One stored secret.
    SecretId
);

impl WorkspaceId {
    /// The Incus instance that realizes this workspace. Deterministic, so a
    /// retried create finds the instance an earlier attempt made.
    #[must_use]
    pub const fn instance_name(self) -> InstanceName {
        InstanceName(self)
    }
}

/// The name of the Incus instance for a workspace: `iglu-<32 hex digits>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InstanceName(WorkspaceId);

impl InstanceName {
    const PREFIX: &'static str = "iglu-";

    #[must_use]
    pub const fn workspace(self) -> WorkspaceId {
        self.0
    }
}

impl fmt::Display for InstanceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", Self::PREFIX, self.0.as_uuid().simple())
    }
}

impl FromStr for InstanceName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = ParseError::new("instance name", "expected iglu-<32 lowercase hex digits>");
        let hex = s.strip_prefix(Self::PREFIX).ok_or(error.clone())?;
        if hex.len() != 32 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(error);
        }
        let uuid = Uuid::try_parse(hex).map_err(|_| error)?;
        Ok(Self(WorkspaceId::from_uuid(uuid)))
    }
}

text_type!(InstanceName);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_names_round_trip() {
        let id = WorkspaceId::from_uuid(Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef));
        let name = id.instance_name();
        assert_eq!(name.to_string(), "iglu-0123456789abcdef0123456789abcdef");
        assert_eq!(name.to_string().parse::<InstanceName>(), Ok(name));
        assert_eq!(name.workspace(), id);
    }

    #[test]
    fn foreign_instance_names_are_rejected() {
        for name in [
            "workspace",
            "iglu-",
            "iglu-0123456789ABCDEF0123456789abcdef",
            "iglu-0123456789abcdef0123456789abcde",
            "iglu-0123456789abcdef0123456789abcdef0",
            "other-0123456789abcdef0123456789abcdef",
        ] {
            assert!(name.parse::<InstanceName>().is_err(), "{name}");
        }
    }
}
