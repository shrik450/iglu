//! Rules for the console's sign-in: where it may send a person afterwards,
//! and which browser may finish a sign-in.

use std::fmt;
use std::str::FromStr;

use crate::ParseError;
use crate::parse::text_type;

/// A path on the console to land on after signing in.
///
/// Only a path: never `//host` or `/\host`, which browsers read as another
/// host, and nothing outside printable ASCII, since browsers drop tabs and
/// newlines from URLs and `/\t/evil.example` would otherwise become
/// `//evil.example`. Paths reach here percent-encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReturnPath(String);

impl ReturnPath {
    #[must_use]
    pub fn root() -> Self {
        Self("/".to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ReturnPath {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let printable = s.bytes().all(|b| (0x21..=0x7e).contains(&b) && b != b'\\');
        let path_only = s.starts_with('/') && !s.starts_with("//");
        if printable && path_only && s.len() <= 2048 {
            Ok(Self(s.to_owned()))
        } else {
            Err(ParseError::new(
                "return path",
                "expected a path on the console",
            ))
        }
    }
}

impl fmt::Display for ReturnPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(ReturnPath);

/// Whether the browser finishing a sign-in is the one that started it.
///
/// A sign-in's `state` is handed to the browser twice: in the provider's
/// redirect and in a cookie. Without the cookie check, someone could start a
/// sign-in as themselves and send the callback link to someone else, who'd
/// end up signed in to the wrong account and, say, store secrets in it.
#[must_use]
pub fn same_browser(cookie: Option<&str>, state: &str) -> bool {
    cookie.is_some_and(|cookie| {
        cookie.len() == state.len()
            && cookie
                .bytes()
                .zip(state.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_paths_stay_on_the_console() {
        for good in ["/", "/w/abc", "/w/abc?tab=t1", "/w/%E2%9C%93"] {
            assert!(good.parse::<ReturnPath>().is_ok(), "{good}");
        }
        for bad in [
            "",
            "w/abc",
            "//evil.example",
            "/\\evil.example",
            "/\t/evil.example",
            "/\n/evil.example",
            "/ /evil.example",
            "https://evil.example",
            "/w/✓",
        ] {
            assert!(bad.parse::<ReturnPath>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn only_the_starting_browser_finishes_a_sign_in() {
        assert!(same_browser(Some("abc"), "abc"));
        assert!(!same_browser(None, "abc"));
        assert!(!same_browser(Some("abd"), "abc"));
        assert!(!same_browser(Some("ab"), "abc"));
    }
}
