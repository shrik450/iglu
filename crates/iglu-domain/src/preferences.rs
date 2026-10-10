//! What a person chooses for how the console looks and takes keys. It's kept
//! with them, so it follows them to every browser they sign in from.
//!
//! Each part fills in its defaults for what's missing, so a setting added
//! later reads from what's stored now.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::parse::{is_printable, text_type};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Preferences {
    pub look: Look,
    pub keyboard: Keyboard,
    pub terminal: TerminalStyle,
}

/// The console's colours: the system's choice, or one of the two looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "snake_case"))]
pub enum Look {
    #[default]
    Auto,
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Keyboard {
    /// The chord that reaches the console from inside a terminal. Which
    /// chords make a usable prefix is the console's to say, as it records one.
    pub prefix: Chord,
    /// Whether Alt (Option) with H, J, K or L moves between columns and
    /// workspaces, even in a terminal.
    pub alt_moves: bool,
    /// On a Mac, which Option keys a terminal reads as Meta.
    pub option_as_meta: OptionAsMeta,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            prefix: Chord {
                code: KeyCode("Space".to_owned()),
                ctrl: true,
                alt: false,
                shift: false,
                meta: false,
            },
            alt_moves: true,
            option_as_meta: OptionAsMeta::Left,
        }
    }
}

/// A key and its modifiers, as a browser's `KeyboardEvent` has them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each modifier is held or not, independently, as the browser reports it"
)]
pub struct Chord {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

/// A physical key, as `KeyboardEvent.code` names it: `KeyA`, `Space`,
/// `BracketLeft`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct KeyCode(String);

impl FromStr for KeyCode {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.len() > 32 || !s.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(ParseError::new(
                "key code",
                "must be up to 32 letters and digits, like KeyA",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for KeyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(KeyCode);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "snake_case"))]
pub enum OptionAsMeta {
    Left,
    Both,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TerminalStyle {
    /// A font installed where the browser runs; none is iglu's own.
    pub font: Option<FontName>,
    /// The text's size, in pixels, before the console nudges it so cells
    /// are whole pixels.
    pub size: FontSize,
    /// `iglu`, a theme the console has by name, or `pasted`. A name the
    /// console doesn't know reads as iglu's own.
    pub theme: ThemeName,
    /// A theme pasted in Ghostty's format, kept while another is tried.
    pub pasted: PastedTheme,
}

impl Default for TerminalStyle {
    fn default() -> Self {
        Self {
            font: None,
            size: FontSize(13),
            theme: ThemeName("iglu".to_owned()),
            pasted: PastedTheme(String::new()),
        }
    }
}

/// A font's name, as CSS can ask for it quoted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct FontName(String);

impl FromStr for FontName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let allowed = |c: char| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-');
        if s.is_empty()
            || s.chars().count() > 64
            || s.trim() != s
            || s.contains("  ")
            || !s.chars().all(allowed)
        {
            return Err(ParseError::new(
                "font name",
                "must be up to 64 letters, digits, single spaces, dots, dashes and underscores",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for FontName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(FontName);

/// A size for text, in whole pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "number"))]
#[serde(try_from = "u8", into = "u8")]
pub struct FontSize(u8);

impl FontSize {
    pub const LEAST: u8 = 6;
    pub const MOST: u8 = 40;
}

impl TryFrom<u8> for FontSize {
    type Error = ParseError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if (Self::LEAST..=Self::MOST).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ParseError::new("font size", "must be 6 to 40 pixels"))
        }
    }
}

impl From<FontSize> for u8 {
    fn from(value: FontSize) -> Self {
        value.0
    }
}

/// A terminal theme's name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct ThemeName(String);

impl FromStr for ThemeName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.chars().count() > 64 || !s.chars().all(is_printable) {
            return Err(ParseError::new(
                "theme name",
                "must be one line of up to 64 characters",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for ThemeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(ThemeName);

/// A theme as pasted: text of a few lines, read by the console.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct PastedTheme(String);

impl PastedTheme {
    pub const MAX_BYTES: usize = 16 * 1024;
}

impl FromStr for PastedTheme {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > Self::MAX_BYTES {
            return Err(ParseError::new("pasted theme", "must be at most 16 KiB"));
        }
        if !s
            .chars()
            .all(|c| matches!(c, '\n' | '\r' | '\t') || is_printable(c))
        {
            return Err(ParseError::new("pasted theme", "must be plain text"));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for PastedTheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(PastedTheme);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_missing_reads_as_its_default() {
        let stored: Preferences =
            serde_json::from_str(r#"{"look": "dark", "terminal": {"size": 16}}"#)
                .expect("a partial document reads");
        assert_eq!(stored.look, Look::Dark);
        assert_eq!(stored.keyboard, Keyboard::default());
        assert_eq!(u8::from(stored.terminal.size), 16);
        assert_eq!(stored.terminal.theme.to_string(), "iglu");
    }

    #[test]
    fn a_whole_document_goes_out_and_back() {
        let preferences = Preferences {
            look: Look::Light,
            keyboard: Keyboard {
                option_as_meta: OptionAsMeta::Both,
                ..Keyboard::default()
            },
            terminal: TerminalStyle {
                font: Some("Berkeley Mono".parse().expect("a font name")),
                pasted: "background = #000000\nforeground = #ffffff\n"
                    .parse()
                    .expect("a theme"),
                ..TerminalStyle::default()
            },
        };
        let json = serde_json::to_string(&preferences).expect("serializes");
        assert_eq!(
            serde_json::from_str::<Preferences>(&json).expect("reads back"),
            preferences
        );
    }

    #[test]
    fn values_that_could_do_harm_are_refused() {
        assert!("Fira\"; color: red".parse::<FontName>().is_err());
        assert!(" Fira".parse::<FontName>().is_err());
        assert!("Key A".parse::<KeyCode>().is_err());
        assert!(FontSize::try_from(41).is_err());
        assert!(FontSize::try_from(5).is_err());
        assert!("a\u{202e}b".parse::<PastedTheme>().is_err());
        assert!(
            "x".repeat(PastedTheme::MAX_BYTES + 1)
                .parse::<PastedTheme>()
                .is_err()
        );
        assert!("line\nbreak".parse::<ThemeName>().is_err());
    }
}
