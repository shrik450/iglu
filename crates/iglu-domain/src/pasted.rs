//! Files a person pastes or drops on a terminal: each is put in the
//! workspace and its path pasted, as a terminal pastes a dropped file's
//! path, so agents that take images and files read them from there.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::parse::text_type;

/// The most one file may be.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

/// A file's name, fit to keep in a workspace and paste as it is: up to 80
/// letters, digits, dots, dashes and underscores, not starting with a dot
/// or dash. Nothing in it needs quoting in a shell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct FileName(String);

impl FileName {
    pub const MAX_LEN: usize = 80;

    fn allowed(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')
    }

    /// A name as a browser gives it, made fit to keep: other characters
    /// become dashes, a long name keeps its extension, and a name with
    /// nothing but an extension left is `pasted` with it.
    #[must_use]
    pub fn from_browser(raw: &str) -> Self {
        let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
        let mut name = String::new();
        for c in base.chars() {
            let c = if Self::allowed(c) { c } else { '-' };
            if !(c == '-' && name.ends_with('-')) {
                name.push(c);
            }
        }
        let name = name.trim_start_matches('-').trim_end_matches(['-', '.']);
        // A name that was all extension keeps it, which is how agents tell
        // an image.
        let name = if name.starts_with('.') {
            format!("pasted{name}")
        } else {
            name.to_owned()
        };
        let name = if name.len() > Self::MAX_LEN {
            let extension = name
                .rfind('.')
                .filter(|&dot| name.len() - dot <= 10)
                .map_or("", |dot| &name[dot..]);
            let stem = name[..Self::MAX_LEN - extension.len()].trim_end_matches(['-', '.']);
            format!("{stem}{extension}")
        } else {
            name
        };
        if name.is_empty() {
            Self("pasted".to_owned())
        } else {
            Self(name)
        }
    }

    /// The name of the `n`th file kept with this name: `shot.png`, then
    /// `shot-2.png`.
    #[must_use]
    pub fn numbered(&self, n: u32) -> String {
        if n <= 1 {
            return self.0.clone();
        }
        match self.0.rfind('.') {
            Some(dot) if dot > 0 => format!("{}-{n}{}", &self.0[..dot], &self.0[dot..]),
            _ => format!("{}-{n}", self.0),
        }
    }
}

impl FromStr for FileName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty()
            || s.len() > Self::MAX_LEN
            || s.starts_with(['.', '-'])
            || !s.chars().all(Self::allowed)
        {
            return Err(ParseError::new(
                "file name",
                "must be up to 80 letters, digits, dots, dashes and underscores, not starting with a dot or dash",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for FileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(FileName);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browsers_name_is_made_fit_to_keep() {
        let name = |raw: &str| FileName::from_browser(raw).to_string();
        assert_eq!(
            name("Screenshot 2026-10-10 at 4.47.00 PM.png"),
            "Screenshot-2026-10-10-at-4.47.00-PM.png"
        );
        assert_eq!(name("image.png"), "image.png");
        assert_eq!(name("../../etc/passwd"), "passwd");
        assert_eq!(name("C:\\Users\\me\\report.pdf"), "report.pdf");
        assert_eq!(name(".bashrc"), "pasted.bashrc");
        assert_eq!(name("日本語.png"), "pasted.png");
        assert_eq!(name("$(rm -rf ~)"), "rm-rf");
        assert_eq!(name(""), "pasted");
        assert_eq!(name("..."), "pasted");
        let long = name(&format!("{}.jpeg", "a".repeat(200)));
        assert!(
            long.len() <= FileName::MAX_LEN
                && long.rsplit_once('.').map(|(_, e)| e) == Some("jpeg"),
            "{long}"
        );
        for raw in ["Screenshot 1.png", "../x", "$(x)", "", &"b".repeat(300)] {
            assert!(name(raw).parse::<FileName>().is_ok(), "{raw}");
        }
    }

    #[test]
    fn a_name_kept_again_is_numbered_before_its_extension() {
        let name: FileName = "shot.png".parse().expect("a name");
        assert_eq!(name.numbered(1), "shot.png");
        assert_eq!(name.numbered(2), "shot-2.png");
        let bare: FileName = "notes".parse().expect("a name");
        assert_eq!(bare.numbered(3), "notes-3");
    }

    #[test]
    fn names_that_could_escape_are_refused() {
        for bad in ["", ".hidden", "-rf", "a/b", "a b", "a;b", &"c".repeat(81)] {
            assert!(bad.parse::<FileName>().is_err(), "{bad}");
        }
    }
}
