//! Terminal sessions inside a workspace. Each is one zmx session.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::parse::text_type;

/// A zmx session name: 1–32 characters of `a-z`, `0-9` and `-`, starting with
/// a letter or digit.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct SessionName(String);

impl SessionName {
    /// The longest a session name can be.
    pub const MAX_LEN: usize = 32;

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
        if s.is_empty() || s.len() > Self::MAX_LEN || !valid_chars || s.starts_with('-') {
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

/// Text frames a terminal client sends. Binary frames are input bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalControl {
    Resize(TerminalSize),
}

/// What's typed into a terminal from outside it: exact text, as bytes the
/// program reads, up to 64 KiB and without NUL. Nothing is added; a program
/// that waits for Enter needs a carriage return at the end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct TerminalInput(String);

impl TerminalInput {
    pub const MAX_BYTES: usize = 64 * 1024;

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TerminalInput {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.len() > Self::MAX_BYTES || s.contains('\0') {
            return Err(ParseError::new(
                "terminal input",
                "must be 1 byte to 64 KiB without NUL",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for TerminalInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(TerminalInput);

/// How many of a terminal's last lines to read: 1 to 2000.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "number"))]
#[serde(try_from = "u16", into = "u16")]
pub struct OutputLines(u16);

impl OutputLines {
    pub const MOST: u16 = 2000;
    pub const DEFAULT: Self = Self(200);

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for OutputLines {
    type Error = ParseError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if (1..=Self::MOST).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ParseError::new("lines", "must be 1 to 2000"))
        }
    }
}

impl From<OutputLines> for u16 {
    fn from(value: OutputLines) -> Self {
        value.0
    }
}

impl FromStr for OutputLines {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u16>()
            .map_err(|_| ParseError::new("lines", "must be 1 to 2000"))
            .and_then(Self::try_from)
    }
}

impl fmt::Display for OutputLines {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The text a terminal shows, read from zmx: its last lines, history
/// included, as plain text. At most this much comes back.
pub const OUTPUT_MAX_BYTES: usize = 128 * 1024;

/// The last `lines` lines of `history`, without the blank ones a screen
/// ends with, and cut to [`OUTPUT_MAX_BYTES`] from the end at a character
/// boundary. Says whether anything was cut.
#[must_use]
pub fn last_lines(history: &str, lines: OutputLines) -> (String, bool) {
    let all: Vec<&str> = history.trim_end().lines().collect();
    let from = all.len().saturating_sub(usize::from(lines.get()));
    let mut text = all[from..].join("\n");
    let mut truncated = from > 0;
    if text.len() > OUTPUT_MAX_BYTES {
        let mut cut = text.len() - OUTPUT_MAX_BYTES;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        text = text[cut..].to_owned();
        truncated = true;
    }
    (text, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_is_exact_and_bounded() {
        assert_eq!(
            "ls\r".parse::<TerminalInput>().map(|i| i.to_string()),
            Ok("ls\r".to_owned())
        );
        assert!("".parse::<TerminalInput>().is_err());
        assert!("a\0b".parse::<TerminalInput>().is_err());
        assert!(
            "x".repeat(TerminalInput::MAX_BYTES + 1)
                .parse::<TerminalInput>()
                .is_err()
        );
    }

    #[test]
    fn output_is_the_last_lines_without_the_blank_end() {
        let lines = OutputLines::try_from(2).expect("lines");
        assert_eq!(
            last_lines("a\nb\nc\n\n\n", lines),
            ("b\nc".to_owned(), true)
        );
        assert_eq!(last_lines("a\n", lines), ("a".to_owned(), false));
        assert_eq!(last_lines("", lines), (String::new(), false));
        assert!(OutputLines::try_from(0).is_err());
        assert!("2001".parse::<OutputLines>().is_err());
    }

    #[test]
    fn long_output_keeps_its_end_whole() {
        let line = "é".repeat(1000);
        let history = vec![line.as_str(); 100].join("\n");
        let (text, truncated) = last_lines(&history, OutputLines::try_from(100).expect("lines"));
        assert!(truncated);
        assert!(text.len() <= OUTPUT_MAX_BYTES);
        assert!(text.ends_with('é'));
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
