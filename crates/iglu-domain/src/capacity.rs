//! Whether the host has room to run one more workspace.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A number of bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "number"))]
#[serde(transparent)]
pub struct Bytes(u64);

impl Bytes {
    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn mib(mib: u64) -> Self {
        Self(mib.saturating_mul(1024 * 1024))
    }

    #[must_use]
    pub const fn gib(gib: u64) -> Self {
        Self(gib.saturating_mul(1024 * 1024 * 1024))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mib = self.0 / (1024 * 1024);
        if mib >= 1024 {
            write!(f, "{}.{} GiB", mib / 1024, (mib % 1024) * 10 / 1024)
        } else {
            write!(f, "{mib} MiB")
        }
    }
}

/// The result of an admission check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Capacity {
    Fits,
    Short { available: Bytes, needed: Bytes },
}

/// Admit a start or thaw when the host's available memory covers the
/// workspace's reservation plus the host's headroom.
///
/// `available` is the host's own estimate (Linux `MemAvailable`), so it already
/// accounts for every running and frozen workspace.
#[must_use]
pub fn admit(available: Bytes, reservation: Bytes, headroom: Bytes) -> Capacity {
    let needed = reservation.saturating_add(headroom);
    if available >= needed {
        Capacity::Fits
    } else {
        Capacity::Short { available, needed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_when_reservation_and_headroom_fit() {
        assert_eq!(
            admit(Bytes::gib(4), Bytes::gib(2), Bytes::gib(1)),
            Capacity::Fits
        );
        assert_eq!(
            admit(Bytes::gib(3), Bytes::gib(2), Bytes::gib(1)),
            Capacity::Fits
        );
    }

    #[test]
    fn reports_the_shortfall() {
        assert_eq!(
            admit(Bytes::gib(2), Bytes::gib(2), Bytes::gib(1)),
            Capacity::Short {
                available: Bytes::gib(2),
                needed: Bytes::gib(3)
            }
        );
    }

    #[test]
    fn displays_human_units() {
        assert_eq!(Bytes::mib(512).to_string(), "512 MiB");
        assert_eq!(Bytes::mib(1536).to_string(), "1.5 GiB");
    }
}
