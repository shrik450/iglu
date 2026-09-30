//! Time as a value. The shell reads the clock and passes it in.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(i64);

impl Timestamp {
    pub const fn from_unix_millis(millis: i64) -> Self {
        Self(millis)
    }

    pub const fn unix_millis(self) -> i64 {
        self.0
    }

    /// `self + duration`, saturating at the end of time.
    pub const fn plus(self, duration: Millis) -> Self {
        Self(self.0.saturating_add(duration.0))
    }

    /// How long ago `earlier` was, or zero if it's in the future.
    pub const fn since(self, earlier: Self) -> Millis {
        let elapsed = self.0.saturating_sub(earlier.0);
        Millis(if elapsed < 0 { 0 } else { elapsed })
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

/// A non-negative duration in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Millis(i64);

impl Millis {
    pub const fn from_secs(secs: u32) -> Self {
        Self(secs as i64 * 1000)
    }

    pub const fn as_millis(self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_never_goes_negative() {
        let t = Timestamp::from_unix_millis(1_000);
        assert_eq!(t.since(Timestamp::from_unix_millis(400)).as_millis(), 600);
        assert_eq!(t.since(Timestamp::from_unix_millis(5_000)).as_millis(), 0);
    }

    #[test]
    fn plus_adds() {
        let t = Timestamp::from_unix_millis(1_000).plus(Millis::from_secs(5));
        assert_eq!(t.unix_millis(), 6_000);
    }
}
