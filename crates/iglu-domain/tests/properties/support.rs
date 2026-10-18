use std::fmt::{Debug, Display};
use std::str::FromStr;

use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Characters that sit on the edges of the core's grammars: separators,
/// controls, characters that reorder or break a line, and ones whose case
/// mapping changes their length.
const TRICKY: &[char] = &[
    'a', 'z', 'A', 'Z', '0', '9', '-', '_', '.', '/', '\\', ':', '@', '#', '?', '%', '~', '^', '[',
    '*', '\'', '"', ' ', '\t', '\n', '\r', '\0', '\u{1b}', '\u{7f}', '\u{85}', '\u{a0}',
    '\u{200e}', '\u{2028}', '\u{202e}', '\u{2067}', '\u{3000}', '\u{feff}', 'é', 'İ', 'ẞ',
    '\u{212a}', '漢', '🧊',
];

/// Any text, weighted toward the characters and lengths where parsers go
/// wrong.
pub fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        prop::collection::vec(prop::sample::select(TRICKY), 0..80)
            .prop_map(|chars| chars.into_iter().collect()),
        // Past the length limits, which sit between 32 and 255 bytes.
        prop::collection::vec(prop::sample::select(TRICKY), 0..300)
            .prop_map(|chars| chars.into_iter().collect()),
    ]
}

/// Showing a parsed value and parsing it again gives the same value, and so
/// does storing it as JSON and reading it back: what's sent or stored is
/// read back as it was.
pub fn round_trips<T>(value: &T) -> Result<(), TestCaseError>
where
    T: FromStr + Display + Serialize + DeserializeOwned + PartialEq + Debug,
    T::Err: Debug,
{
    shows_and_parses(value)?;
    let json = serde_json::to_string(value).map_err(|e| TestCaseError::fail(e.to_string()))?;
    let read: Option<T> = serde_json::from_str(&json).ok();
    prop_assert_eq!(read.as_ref(), Some(value), "read back from {}", json);
    Ok(())
}

/// Showing a parsed value and parsing it again gives the same value.
pub fn shows_and_parses<T>(value: &T) -> Result<(), TestCaseError>
where
    T: FromStr + Display + PartialEq + Debug,
    T::Err: Debug,
{
    let shown = value.to_string();
    let parsed = shown.parse::<T>();
    prop_assert_eq!(
        parsed.as_ref().ok(),
        Some(value),
        "parsed from {:?}: {:?}",
        shown,
        parsed
    );
    Ok(())
}
