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

/// True for characters that are safe to show in a UI or log line.
pub(crate) fn is_printable(c: char) -> bool {
    !c.is_control()
}
