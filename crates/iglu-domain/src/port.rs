//! Guest ports that routes may point at.

use std::fmt;
use std::num::NonZeroU16;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;

/// A TCP port on the guest's loopback interface. Routes can only ever point
/// at `127.0.0.1:<GuestPort>` inside their own workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "number"))]
// Parsed from any integer, so an out-of-range port gets the port's own
// message rather than serde's "expected u16".
#[serde(try_from = "i64", into = "u16")]
pub struct GuestPort(NonZeroU16);

impl GuestPort {
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

impl TryFrom<u16> for GuestPort {
    type Error = ParseError;

    fn try_from(port: u16) -> Result<Self, Self::Error> {
        NonZeroU16::new(port)
            .map(Self)
            .ok_or(ParseError::new("guest port", "must be between 1 and 65535"))
    }
}

impl TryFrom<i64> for GuestPort {
    type Error = ParseError;

    fn try_from(port: i64) -> Result<Self, Self::Error> {
        u16::try_from(port)
            .map_err(|_| ParseError::new("guest port", "must be between 1 and 65535"))?
            .try_into()
    }
}

impl From<GuestPort> for u16 {
    fn from(port: GuestPort) -> Self {
        port.get()
    }
}

impl FromStr for GuestPort {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u16>()
            .map_err(|_| ParseError::new("guest port", "must be between 1 and 65535"))?
            .try_into()
    }
}

impl fmt::Display for GuestPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_are_nonzero_u16() {
        assert!("0".parse::<GuestPort>().is_err());
        assert!("65536".parse::<GuestPort>().is_err());
        assert!("-1".parse::<GuestPort>().is_err());
        assert_eq!("3000".parse::<GuestPort>().map(GuestPort::get), Ok(3000));
    }

    #[test]
    fn out_of_range_json_ports_get_the_ports_message() {
        let error = serde_json::from_str::<GuestPort>("70000").expect_err("out of range");
        assert!(
            error.to_string().starts_with("invalid guest port"),
            "{error}"
        );
        assert_eq!(
            serde_json::from_str::<GuestPort>("3000")
                .map(GuestPort::get)
                .ok(),
            Some(3000)
        );
    }
}
