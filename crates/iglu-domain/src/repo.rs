//! Git repositories and branches that workspaces are created from.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::parse::text_type;

/// A Git host name, used to match credentials to remotes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GitHost(String);

impl GitHost {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for GitHost {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = ParseError::new("Git host", "expected a host name such as github.com");
        let lower = s.to_ascii_lowercase();
        let (host, port) = match lower.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (lower.as_str(), None),
        };
        let labels_ok = !host.is_empty()
            && host.len() <= 253
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !label.starts_with('-')
                    && !label.ends_with('-')
            });
        let port_ok = port.is_none_or(|p| p.parse::<u16>().is_ok_and(|p| p != 0));
        if labels_ok && port_ok {
            Ok(Self(lower))
        } else {
            Err(error)
        }
    }
}

impl fmt::Display for GitHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(GitHost);

/// How a remote is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Https,
    Ssh,
}

/// A clone URL: `https://host/path`, `ssh://[user@]host/path`, or the scp-like
/// `user@host:path`. Parsed so the host and checkout directory are known.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepoUrl {
    original: String,
    transport: Transport,
    host: GitHost,
    dir: CheckoutDir,
}

impl RepoUrl {
    #[must_use]
    pub const fn transport(&self) -> Transport {
        self.transport
    }

    #[must_use]
    pub const fn host(&self) -> &GitHost {
        &self.host
    }

    /// The directory the repository is cloned into, under the guest user's home.
    #[must_use]
    pub const fn checkout_dir(&self) -> &CheckoutDir {
        &self.dir
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.original
    }
}

impl FromStr for RepoUrl {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = ParseError::new(
            "repository URL",
            "expected https://host/path, ssh://host/path or user@host:path",
        );
        if s.len() > 1024 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(error);
        }
        let (transport, authority, path) = if let Some(rest) = s.strip_prefix("https://") {
            let (authority, path) = rest.split_once('/').ok_or(error.clone())?;
            (Transport::Https, authority, path)
        } else if let Some(rest) = s.strip_prefix("ssh://") {
            let (authority, path) = rest.split_once('/').ok_or(error.clone())?;
            (Transport::Ssh, authority, path)
        } else if !s.contains("://") && !s.starts_with('-') {
            let (authority, path) = s.split_once(':').ok_or(error.clone())?;
            // `transport::address` is Git's remote-helper syntax, not scp-like.
            if path.starts_with(':') {
                return Err(error);
            }
            (Transport::Ssh, authority, path)
        } else {
            return Err(error);
        };
        if authority.contains('?') || path.contains('?') || path.contains('#') {
            return Err(error);
        }
        let host_part = match authority.rsplit_once('@') {
            Some((user, host)) if !user.is_empty() && !user.contains(':') => host,
            Some(_) => return Err(error),
            None => authority,
        };
        let host: GitHost = host_part.parse().map_err(|_| error.clone())?;
        let dir = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .map(|last| last.strip_suffix(".git").unwrap_or(last))
            .ok_or(error.clone())?
            .parse()
            .map_err(|_| error)?;
        Ok(Self {
            original: s.to_owned(),
            transport,
            host,
            dir,
        })
    }
}

impl fmt::Display for RepoUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.original)
    }
}

text_type!(RepoUrl);

/// A single, safe path component for a checkout directory.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CheckoutDir(String);

impl CheckoutDir {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for CheckoutDir {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = !s.is_empty()
            && s.len() <= 100
            && s != "."
            && s != ".."
            && !s.starts_with('-')
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(ParseError::new("checkout directory", "not a safe name"))
        }
    }
}

impl fmt::Display for CheckoutDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(CheckoutDir);

/// A Git branch name, following the rules of `git check-ref-format --branch`
/// closely enough to never be interpreted as an option or a revision expression.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BranchName(String);

impl BranchName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Git reserves a `.lock` suffix on every ref component, and compares it
/// case-sensitively.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "Git's own check is case-sensitive"
)]
fn ends_with_lock(component: &str) -> bool {
    component.ends_with(".lock")
}

impl FromStr for BranchName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let forbidden_char =
            |c: char| c.is_ascii_control() || c.is_whitespace() || "~^:?*[\\".contains(c);
        let valid = !s.is_empty()
            && s.len() <= 200
            && s != "@"
            && s != "HEAD"
            && !s.starts_with('-')
            && !s.starts_with('/')
            && !s.ends_with('/')
            && !s.ends_with('.')
            && !s.contains("..")
            && !s.contains("//")
            && !s.contains("@{")
            && !s
                .split('/')
                .any(|part| part.starts_with('.') || ends_with_lock(part))
            && !s.chars().any(forbidden_char);
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(ParseError::new("branch name", "not a valid Git branch"))
        }
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(BranchName);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_remote_forms() {
        let cases = [
            (
                "https://github.com/acme/widgets.git",
                Transport::Https,
                "github.com",
                "widgets",
            ),
            (
                "https://git.example.com:8443/team/app",
                Transport::Https,
                "git.example.com:8443",
                "app",
            ),
            (
                "ssh://git@github.com/acme/widgets.git",
                Transport::Ssh,
                "github.com",
                "widgets",
            ),
            (
                "git@github.com:acme/widgets.git",
                Transport::Ssh,
                "github.com",
                "widgets",
            ),
            (
                "https://GitHub.com/acme/widgets/",
                Transport::Https,
                "github.com",
                "widgets",
            ),
        ];
        for (url, transport, host, dir) in cases {
            let parsed: RepoUrl = url.parse().expect(url);
            assert_eq!(parsed.transport(), transport, "{url}");
            assert_eq!(parsed.host().as_str(), host, "{url}");
            assert_eq!(parsed.checkout_dir().as_str(), dir, "{url}");
            assert_eq!(parsed.to_string(), url);
        }
    }

    #[test]
    fn rejects_urls_that_could_be_options_or_local_paths() {
        for url in [
            "",
            "-uhttps://evil",
            "file:///etc/passwd",
            "/srv/repo",
            "ext::sh -c touch",
            "https://github.com",
            "https://github.com/acme/..",
            "https://user:pass@github.com/acme/app",
            "https://github.com/a b",
            "ext::sh/repo",
        ] {
            assert!(url.parse::<RepoUrl>().is_err(), "{url}");
        }
    }

    #[test]
    fn branch_names_follow_git_rules() {
        for ok in ["main", "feature/login", "swift-dancing", "v1.2"] {
            assert!(ok.parse::<BranchName>().is_ok(), "{ok}");
        }
        for bad in [
            "", "-f", "a..b", "a b", "a~1", "x.lock", "a.lock/b", "/a", "a/", "@", "a@{1}",
            ".hidden", "a/.b", "HEAD",
        ] {
            assert!(bad.parse::<BranchName>().is_err(), "{bad}");
        }
    }
}
