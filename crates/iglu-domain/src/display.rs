//! The browser column's display: a headless Wayland output the browser is
//! drawn on, which the console shows over VNC.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;

/// How many device pixels the display draws for each CSS pixel of the
/// console showing it, as the console's `devicePixelRatio` says: text is
/// as sharp as the screen it's shown on. Between 1 and 4, in hundredths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "number"))]
#[serde(try_from = "f64", into = "f64")]
pub struct DisplayScale(u16);

impl DisplayScale {
    pub const ONE: Self = Self(100);
    const MIN: u16 = 100;
    const MAX: u16 = 400;

    #[must_use]
    pub fn hundredths(self) -> u16 {
        self.0
    }
}

impl TryFrom<f64> for DisplayScale {
    type Error = ParseError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        let hundredths = (value * 100.0).round();
        if !(f64::from(Self::MIN)..=f64::from(Self::MAX)).contains(&hundredths) {
            return Err(ParseError::new("scale", "must be between 1 and 4"));
        }
        // In range, so it fits.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(Self(hundredths as u16))
    }
}

impl From<DisplayScale> for f64 {
    fn from(scale: DisplayScale) -> Self {
        f64::from(scale.0) / 100.0
    }
}

impl FromStr for DisplayScale {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value: f64 = s
            .parse()
            .map_err(|_| ParseError::new("scale", "must be a number"))?;
        value.try_into()
    }
}

/// As a decimal with no more places than it needs: `1`, `1.5`, `1.25`.
impl fmt::Display for DisplayScale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (whole, part) = (self.0 / 100, self.0 % 100);
        match part {
            0 => write!(f, "{whole}"),
            p if p % 10 == 0 => write!(f, "{whole}.{}", p / 10),
            p => write!(f, "{whole}.{p:02}"),
        }
    }
}

/// What the display draws and encodes with, as the workspace found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct DisplayReport {
    /// Whether it draws on the host's GPU, which also encodes H.264. Without
    /// one, it draws on the CPU and is sent losslessly.
    pub hardware: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scale_is_a_screens_pixel_ratio_rounded_to_hundredths() {
        let scale = |v: f64| DisplayScale::try_from(v).map(|s| s.to_string());
        assert_eq!(scale(1.0).as_deref(), Ok("1"));
        assert_eq!(scale(2.0).as_deref(), Ok("2"));
        assert_eq!(scale(1.5).as_deref(), Ok("1.5"));
        assert_eq!(scale(1.25).as_deref(), Ok("1.25"));
        assert_eq!(scale(1.333_333).as_deref(), Ok("1.33"));
        assert!(DisplayScale::try_from(0.5).is_err());
        assert!(DisplayScale::try_from(5.0).is_err());
        assert!(DisplayScale::try_from(f64::NAN).is_err());
        assert_eq!("2".parse::<DisplayScale>().map(f64::from), Ok(2.0));
        assert!("two".parse::<DisplayScale>().is_err());
    }
}
