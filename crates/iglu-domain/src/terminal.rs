//! Terminal sessions inside a workspace. Each is one zmx session.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::parse::text_type;

/// A zmx session name: 1–32 characters of `a-z`, `0-9` and `-`, starting with
/// a letter or digit.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SessionName(String);

impl SessionName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SessionName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid_chars = s
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'));
        if s.is_empty() || s.len() > 32 || !valid_chars || s.starts_with('-') {
            return Err(ParseError::new(
                "session name",
                "expected 1-32 of a-z, 0-9, '-'",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for SessionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(SessionName);

/// The first `t<n>` name not already taken.
///
/// # Panics
///
/// Only if every name from `t1` to `t4294967295` is taken.
pub fn next_session_name<'a>(existing: impl IntoIterator<Item = &'a SessionName>) -> SessionName {
    let taken: std::collections::HashSet<&str> =
        existing.into_iter().map(SessionName::as_str).collect();
    (1..=u32::MAX)
        .map(|n| format!("t{n}"))
        .find(|name| !taken.contains(name.as_str()))
        .and_then(|name| name.parse().ok())
        .expect("some t<n> is always free and well-formed")
}

/// A terminal size in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSize")]
pub struct TerminalSize {
    cols: u16,
    rows: u16,
}

#[derive(Deserialize)]
struct RawSize {
    cols: u16,
    rows: u16,
}

impl TerminalSize {
    pub const DEFAULT: Self = Self { cols: 80, rows: 24 };

    /// # Errors
    ///
    /// When either dimension is outside what a terminal can sensibly have.
    pub fn new(cols: u16, rows: u16) -> Result<Self, ParseError> {
        if (2..=1000).contains(&cols) && (1..=1000).contains(&rows) {
            Ok(Self { cols, rows })
        } else {
            Err(ParseError::new(
                "terminal size",
                "columns must be 2-1000 and rows 1-1000",
            ))
        }
    }

    #[must_use]
    pub const fn cols(self) -> u16 {
        self.cols
    }

    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows
    }
}

impl TryFrom<RawSize> for TerminalSize {
    type Error = ParseError;

    fn try_from(raw: RawSize) -> Result<Self, Self::Error> {
        Self::new(raw.cols, raw.rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<SessionName> {
        list.iter()
            .map(|n| n.parse().expect("valid session name"))
            .collect()
    }

    #[test]
    fn next_session_name_fills_gaps() {
        assert_eq!(next_session_name(&names(&[])).as_str(), "t1");
        assert_eq!(next_session_name(&names(&["t1", "t2"])).as_str(), "t3");
        assert_eq!(next_session_name(&names(&["t2", "build"])).as_str(), "t1");
    }

    #[test]
    fn session_names_are_bounded() {
        assert!("".parse::<SessionName>().is_err());
        assert!("-x".parse::<SessionName>().is_err());
        assert!("a b".parse::<SessionName>().is_err());
        assert!("x".repeat(33).parse::<SessionName>().is_err());
        assert!("dev-server".parse::<SessionName>().is_ok());
    }

    #[test]
    fn terminal_sizes_are_bounded() {
        assert!(TerminalSize::new(0, 24).is_err());
        assert!(TerminalSize::new(80, 0).is_err());
        assert!(TerminalSize::new(80, 24).is_ok());
    }
}
