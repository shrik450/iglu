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

    #[must_use]
    pub const fn what(&self) -> &'static str {
        self.what
    }

    #[must_use]
    pub const fn reason(&self) -> &'static str {
        self.reason
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
