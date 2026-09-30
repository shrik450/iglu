//! Secrets and where they go inside a workspace.
//!
//! Anything an agent can read, it can copy; this module only makes sure
//! secrets land where tools expect them and never leak through debug output.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::lifecycle::SecretsGeneration;
use crate::parse::{is_printable, text_type};
use crate::repo::GitHost;

/// A secret's name, chosen by its owner.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct SecretName(String);

impl SecretName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SecretName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = !s.is_empty()
            && s.len() <= 64
            && s.bytes()
                .next()
                .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && s.bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.'));
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(ParseError::new(
                "secret name",
                "expected 1-64 of a-z, 0-9, '-', '_', '.'",
            ))
        }
    }
}

/// An environment variable name for a secret. Variables the platform relies
/// on can't be overridden.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct EnvVarName(String);

impl EnvVarName {
    const RESERVED: &'static [&'static str] = &[
        "HOME",
        "PATH",
        "USER",
        "LOGNAME",
        "SHELL",
        "TERM",
        "XDG_RUNTIME_DIR",
        "ZMX_DIR",
    ];

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for EnvVarName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .next()
                .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
            && s.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        if !valid {
            return Err(ParseError::new(
                "environment variable",
                "expected [A-Z_][A-Z0-9_]*",
            ));
        }
        if Self::RESERVED.contains(&s) || s.starts_with("IGLU_") || s.starts_with("ZMX_") {
            return Err(ParseError::new(
                "environment variable",
                "is reserved by the platform",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

/// A path relative to the guest user's home directory, with no way to
/// escape it: no absolute paths, `.` or `..` components, or empty components.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct HomePath(String);

impl HomePath {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parent directories that must exist, shallowest first.
    #[must_use]
    pub fn parents(&self) -> Vec<&str> {
        self.0
            .match_indices('/')
            .map(|(i, _)| &self.0[..i])
            .collect()
    }
}

impl FromStr for HomePath {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let components_ok = s.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 255
                && part.chars().all(is_printable)
        });
        if s.is_empty() || s.len() > 1024 || s.starts_with('/') || !components_ok {
            return Err(ParseError::new(
                "home path",
                "must be a relative path inside the home directory",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

/// The username half of a Git credential. Tokens usually pair with a fixed
/// username such as `x-access-token`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct GitUsername(String);

impl GitUsername {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for GitUsername {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = !s.is_empty()
            && s.len() <= 128
            && !s
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == ':');
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(ParseError::new(
                "Git username",
                "expected 1-128 printable characters",
            ))
        }
    }
}

macro_rules! display_inner {
    ($($name:ident),*) => {$(
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        text_type!($name);
    )*};
}

display_inner!(SecretName, EnvVarName, HomePath, GitUsername);

/// Where a secret is delivered.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(tag = "kind", rename_all = "snake_case")
)]
pub enum SecretTarget {
    /// Exported into every new terminal session.
    Env { name: EnvVarName },
    /// A file in the home directory, linked to a tmpfs copy.
    File { path: HomePath },
    /// Answers `git credential fill` for one host.
    GitCredential {
        host: GitHost,
        username: GitUsername,
    },
}

/// A secret's value. Never printed: `Debug` is redacted and there is no `Display`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct SecretValue(String);

impl SecretValue {
    pub const MAX_BYTES: usize = 64 * 1024;

    /// The value itself, for the one place that must write it into a guest.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

impl TryFrom<String> for SecretValue {
    type Error = ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > Self::MAX_BYTES {
            return Err(ParseError::new("secret value", "must be 1 byte to 64 KiB"));
        }
        Ok(Self(value))
    }
}

impl From<SecretValue> for String {
    fn from(value: SecretValue) -> Self {
        value.0
    }
}

/// Everything delivered to one workspace, grouped by destination.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretBundle {
    pub generation: SecretsGeneration,
    pub env: BTreeMap<EnvVarName, SecretValue>,
    pub files: BTreeMap<HomePath, SecretValue>,
    pub git: Vec<GitCredential>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCredential {
    pub host: GitHost,
    pub username: GitUsername,
    pub password: SecretValue,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BundleError {
    #[error("two secrets set the environment variable {0}")]
    DuplicateEnv(EnvVarName),
    #[error("two secrets write the file ~/{0}")]
    DuplicateFile(HomePath),
    #[error("two secrets are Git credentials for {0}")]
    DuplicateGitHost(GitHost),
}

/// Groups an owner's secrets for delivery, rejecting any two that claim the
/// same destination.
///
/// # Errors
///
/// When two secrets target the same variable, file, or Git host.
pub fn bundle(
    generation: SecretsGeneration,
    secrets: impl IntoIterator<Item = (SecretTarget, SecretValue)>,
) -> Result<SecretBundle, BundleError> {
    let mut result = SecretBundle {
        generation,
        env: BTreeMap::new(),
        files: BTreeMap::new(),
        git: Vec::new(),
    };
    for (target, value) in secrets {
        match target {
            SecretTarget::Env { name } => {
                if result.env.contains_key(&name) {
                    return Err(BundleError::DuplicateEnv(name));
                }
                result.env.insert(name, value);
            }
            SecretTarget::File { path } => {
                if result.files.contains_key(&path) {
                    return Err(BundleError::DuplicateFile(path));
                }
                result.files.insert(path, value);
            }
            SecretTarget::GitCredential { host, username } => {
                if result.git.iter().any(|existing| existing.host == host) {
                    return Err(BundleError::DuplicateGitHost(host));
                }
                result.git.push(GitCredential {
                    host,
                    username,
                    password: value,
                });
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(s: &str) -> SecretValue {
        SecretValue::try_from(s.to_owned()).expect("valid secret")
    }

    #[test]
    fn values_never_print() {
        let secret = value("hunter2");
        assert!(!format!("{secret:?}").contains("hunter2"));
        let bundle = bundle(
            SecretsGeneration::from_u64(1),
            [(
                SecretTarget::Env {
                    name: "API_KEY".parse().expect("valid"),
                },
                secret,
            )],
        )
        .expect("no conflicts");
        assert!(!format!("{bundle:?}").contains("hunter2"));
    }

    #[test]
    fn home_paths_cannot_escape_home() {
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a//b", "./a", "a/"] {
            assert!(bad.parse::<HomePath>().is_err(), "{bad}");
        }
        let path: HomePath = ".claude/.credentials.json".parse().expect("valid");
        assert_eq!(path.parents(), vec![".claude"]);
        let nested: HomePath = "a/b/c".parse().expect("valid");
        assert_eq!(nested.parents(), vec!["a", "a/b"]);
    }

    #[test]
    fn platform_variables_are_reserved() {
        for bad in [
            "PATH",
            "HOME",
            "IGLU_SESSION",
            "ZMX_SESSION",
            "lower",
            "1ABC",
        ] {
            assert!(bad.parse::<EnvVarName>().is_err(), "{bad}");
        }
        assert!("CLAUDE_CODE_OAUTH_TOKEN".parse::<EnvVarName>().is_ok());
    }

    #[test]
    fn conflicting_destinations_are_rejected() {
        let env = || SecretTarget::Env {
            name: "TOKEN".parse().expect("valid"),
        };
        assert!(matches!(
            bundle(
                SecretsGeneration::from_u64(1),
                [(env(), value("a")), (env(), value("b"))]
            ),
            Err(BundleError::DuplicateEnv(_))
        ));
        let git = || SecretTarget::GitCredential {
            host: "github.com".parse().expect("valid"),
            username: "x-access-token".parse().expect("valid"),
        };
        assert!(matches!(
            bundle(
                SecretsGeneration::from_u64(1),
                [(git(), value("a")), (git(), value("b"))]
            ),
            Err(BundleError::DuplicateGitHost(_))
        ));
    }
}
