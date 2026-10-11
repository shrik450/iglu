//! Files the workspace's browser downloads: Chromium keeps them in the
//! workspace user's Downloads, under the names it gives them, and the person
//! saves them from there.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::env::GuestUser;
use crate::parse::{is_printable, text_type};

/// Where downloads go, in the workspace user's home.
pub const DIR: &str = "Downloads";

/// A downloaded file's name as Chromium gave it, such as `note (1).txt`:
/// one part of a path, of text a line can show.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct DownloadName(String);

impl DownloadName {
    /// The most a file name may be on Linux, in bytes.
    pub const MAX_BYTES: usize = 255;

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The download's path in `user`'s home, from the root and without
    /// the leading slash, as the host opens it inside the guest.
    #[must_use]
    pub fn in_home(&self, user: &GuestUser) -> String {
        user.home
            .join(&format!("{DIR}/{}", self.0))
            .trim_start_matches('/')
            .to_owned()
    }

    /// The name of a file the browser downloaded to `path`, if it's in a
    /// Downloads: the browser's own setting puts downloads in the user's,
    /// and a file is only ever saved from there, by name. Downloads an
    /// agent had the browser put elsewhere aren't named.
    #[must_use]
    pub fn of_download(path: &str) -> Option<Self> {
        let (dir, name) = path.rsplit_once('/')?;
        if !dir.ends_with(&format!("/{DIR}")) {
            return None;
        }
        name.parse().ok()
    }
}

impl FromStr for DownloadName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s == "." || s == ".." {
            return Err(ParseError::new("download name", "must name a file"));
        }
        if s.len() > Self::MAX_BYTES {
            return Err(ParseError::new(
                "download name",
                "must be at most 255 bytes",
            ));
        }
        if s.contains('/') || !s.chars().all(is_printable) {
            return Err(ParseError::new(
                "download name",
                "must be one part of a path, of plain text",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for DownloadName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(DownloadName);

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> GuestUser {
        serde_json::from_value(serde_json::json!({
            "name": "dev", "uid": 1000, "gid": 100, "home": "/home/dev",
        }))
        .expect("a user")
    }

    #[test]
    fn a_name_is_one_part_of_a_path() {
        for good in ["note.txt", "note (1).txt", "Ünïcode ✓.pdf", ".hidden"] {
            assert!(good.parse::<DownloadName>().is_ok(), "{good}");
        }
        for bad in ["", ".", "..", "a/b", "../x", "a\nb", "\u{202e}gpj.exe"] {
            assert!(bad.parse::<DownloadName>().is_err(), "{bad:?}");
        }
        assert!("x".repeat(256).parse::<DownloadName>().is_err());
    }

    #[test]
    fn only_downloads_in_the_users_downloads_are_named() {
        let named = |path| DownloadName::of_download(path).map(|n| n.0);
        assert_eq!(
            named("/home/dev/Downloads/note (1).txt").as_deref(),
            Some("note (1).txt")
        );
        assert_eq!(named("/home/dev/Downloads/sub/note.txt"), None);
        assert_eq!(named("/home/dev/Downloadsx/note.txt"), None);
        assert_eq!(named("/tmp/playwright/1f3a"), None);
        let name: DownloadName = "note.txt".parse().expect("a name");
        assert_eq!(name.in_home(&user()), "home/dev/Downloads/note.txt");
    }
}
