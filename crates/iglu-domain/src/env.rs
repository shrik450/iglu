//! Environments: a flake output that builds a workspace image.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::agent::AgentSpec;
use crate::label::DnsLabel;
use crate::parse::text_type;

/// A flake reference as Nix accepts it, such as `github:alice/dotfiles`.
/// Nix parses it properly; this only keeps it from being anything else.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct FlakeRef(String);

impl FlakeRef {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for FlakeRef {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = !s.is_empty()
            && s.len() <= 512
            && !s.starts_with('-')
            && !s.contains('#')
            && !s.chars().any(|c| c.is_whitespace() || c.is_control());
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(ParseError::new(
                "flake reference",
                "expected a flake reference without '#'",
            ))
        }
    }
}

/// The name of an attribute under `nixosConfigurations`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct FlakeAttr(String);

impl FlakeAttr {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for FlakeAttr {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'\''));
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(ParseError::new(
                "flake attribute",
                "expected a Nix identifier",
            ))
        }
    }
}

/// Where an environment comes from: `<flake>#<nixosConfigurations attribute>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct EnvSource {
    pub flake: FlakeRef,
    pub attr: FlakeAttr,
}

impl EnvSource {
    /// The installable that builds this environment's image.
    #[must_use]
    pub fn image_installable(&self) -> String {
        format!(
            "{}#nixosConfigurations.{}.config.system.build.igluImage",
            self.flake, self.attr
        )
    }
}

impl FromStr for EnvSource {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (flake, attr) = s.split_once('#').ok_or(ParseError::new(
            "environment source",
            "expected <flake>#<nixosConfigurations attribute>",
        ))?;
        Ok(Self {
            flake: flake.parse()?,
            attr: attr.parse()?,
        })
    }
}

impl fmt::Display for EnvSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.flake, self.attr)
    }
}

text_type!(EnvSource);

/// An environment's name, unique per owner.
pub type EnvName = DnsLabel;

/// An Incus image fingerprint: 64 lowercase hex digits (SHA-256).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct ImageFingerprint(String);

impl ImageFingerprint {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ImageFingerprint {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Self(s.to_owned()))
        } else {
            Err(ParseError::new(
                "image fingerprint",
                "expected 64 lowercase hex digits",
            ))
        }
    }
}

/// A Unix account name inside a guest.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct UnixUser(String);

impl UnixUser {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for UnixUser {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = !s.is_empty()
            && s.len() <= 32
            && s.bytes()
                .next()
                .is_some_and(|b| b.is_ascii_lowercase() || b == b'_')
            && s.bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
            && s != "root";
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(ParseError::new(
                "guest user",
                "expected a non-root Unix account name",
            ))
        }
    }
}

/// An absolute path inside a guest.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct GuestPath(String);

impl GuestPath {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `self/relative`, for paths the core already proved stay inside `self`.
    #[must_use]
    pub fn join(&self, relative: &str) -> String {
        format!("{}/{}", self.0.trim_end_matches('/'), relative)
    }
}

impl FromStr for GuestPath {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = s.starts_with('/')
            && s.len() <= 1024
            && !s.split('/').any(|part| part == "." || part == "..")
            && !s.chars().any(char::is_control);
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(ParseError::new("guest path", "expected an absolute path"))
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

display_inner!(FlakeRef, FlakeAttr, ImageFingerprint, UnixUser, GuestPath);

/// The account terminals run as, read from the built image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GuestUser {
    pub name: UnixUser,
    pub uid: Uid,
    pub gid: Uid,
    pub home: GuestPath,
}

/// A non-root user or group ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "number"))]
#[serde(try_from = "u32", into = "u32")]
pub struct Uid(u32);

impl Uid {
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for Uid {
    type Error = ParseError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(ParseError::new("user ID", "must not be root"))
        } else {
            Ok(Self(value))
        }
    }
}

impl From<Uid> for u32 {
    fn from(value: Uid) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "snake_case"))]
pub enum Arch {
    #[serde(rename = "x86_64")]
    #[cfg_attr(feature = "ts", ts(rename = "x86_64"))]
    X86_64,
    Aarch64,
}

impl FromStr for Arch {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "x86_64" => Ok(Self::X86_64),
            "aarch64" => Ok(Self::Aarch64),
            _ => Err(ParseError::new(
                "architecture",
                "expected x86_64 or aarch64",
            )),
        }
    }
}

/// What an image build produced, as reported by the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BuiltImage {
    pub fingerprint: ImageFingerprint,
    pub arch: Arch,
    pub user: GuestUser,
    /// The agents the environment declares, which its workspaces can run.
    pub agents: Vec<AgentSpec>,
    /// Whether it has the browser a browser column runs: images from before
    /// iglu had one don't.
    #[serde(default)]
    pub browser: bool,
    /// The Nix store path the image came from, for provenance.
    pub store_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_sources_split_on_hash() {
        let source: EnvSource = "github:alice/dotfiles#workspace".parse().expect("valid");
        assert_eq!(source.flake.as_str(), "github:alice/dotfiles");
        assert_eq!(source.attr.as_str(), "workspace");
        assert_eq!(
            source.image_installable(),
            "github:alice/dotfiles#nixosConfigurations.workspace.config.system.build.igluImage"
        );
        assert_eq!(source.to_string(), "github:alice/dotfiles#workspace");
    }

    #[test]
    fn env_sources_cannot_smuggle_arguments() {
        for bad in [
            "github:a/b",
            "#x",
            "--impure#x",
            "github:a/b#x y",
            "github:a/b#x#y",
            "a b#c",
        ] {
            assert!(bad.parse::<EnvSource>().is_err(), "{bad}");
        }
    }

    #[test]
    fn guest_users_are_not_root() {
        assert!("root".parse::<UnixUser>().is_err());
        assert!(Uid::try_from(0).is_err());
        assert!("alice".parse::<UnixUser>().is_ok());
    }

    #[test]
    fn guest_paths_are_absolute_and_normal() {
        assert!("/home/alice".parse::<GuestPath>().is_ok());
        for bad in ["home", "/home/../etc", "/a/./b"] {
            assert!(bad.parse::<GuestPath>().is_err(), "{bad}");
        }
    }
}
