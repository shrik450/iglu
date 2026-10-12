use std::fmt;

/// A value from outside the core didn't satisfy its type's invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    what: &'static str,
    reason: &'static str,
}

impl ParseError {
    pub(crate) const fn new(what: &'static str, reason: &'static str) -> Self {
        Self { what, reason }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {}: {}", self.what, self.reason)
    }
}

impl std::error::Error for ParseError {}

/// Implements `TryFrom<String>` and `From<T> for String` through `FromStr` and
/// `Display`, so a type can use `#[serde(try_from = "String", into = "String")]`
/// and deserializing it is parsing it.
macro_rules! text_type {
    ($name:ident) => {
        impl TryFrom<String> for $name {
            type Error = $crate::ParseError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.to_string()
            }
        }
    };
}

pub(crate) use text_type;

/// Displays a newtype over a `String` as its text, and makes it a
/// [`text_type!`].
macro_rules! display_inner {
    ($($name:ident),*) => {$(
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        $crate::parse::text_type!($name);
    )*};
}

pub(crate) use display_inner;

/// Implements `FromStr` and `Display` through a unit enum's serde names, so
/// its text, stored or sent, is spelled in one place.
macro_rules! serde_text {
    ($name:ident, $what:literal, $expected:literal) => {
        impl std::str::FromStr for $name {
            type Err = $crate::ParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                serde_plain::from_str(s).map_err(|_| $crate::ParseError::new($what, $expected))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.pad(&serde_plain::to_string(self).map_err(|_| std::fmt::Error)?)
            }
        }
    };
}

pub(crate) use serde_text;

/// True for characters that are safe to show in a UI or log line: not
/// control characters, line or paragraph separators, or the invisible marks
/// that reorder text around them.
pub(crate) fn is_printable(c: char) -> bool {
    !c.is_control()
        && !matches!(
            c,
            '\u{2028}' | '\u{2029}' | '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        )
}

#[cfg(test)]
mod tests {
    use super::is_printable;

    #[test]
    fn printable_leaves_out_what_breaks_or_reorders_a_line() {
        for c in ['a', 'é', '漢', '🧊', ' ', '-'] {
            assert!(is_printable(c), "{c:?}");
        }
        for c in [
            '\n', '\t', '\u{1b}', '\u{2028}', '\u{202e}', '\u{2067}', '\u{200f}',
        ] {
            assert!(!is_printable(c), "{c:?}");
        }
    }
}
