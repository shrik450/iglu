//! Who may sign in and what they may do.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::id::PrincipalId;
use crate::parse::{is_printable, text_type};

/// An OIDC issuer URL, exactly as the provider states it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Issuer(String);

/// An OIDC subject: stable and unique within its issuer.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Subject(String);

/// An email address the provider says it verified.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Email(String);

/// A name for display only. Never used to identify anyone.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DisplayName(String);

impl Issuer {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Subject {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Email {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl DisplayName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn bounded_text(s: &str, what: &'static str, max: usize) -> Result<String, ParseError> {
    if s.is_empty() || s.len() > max {
        return Err(ParseError::new(what, "has the wrong length"));
    }
    if !s.chars().all(is_printable) {
        return Err(ParseError::new(what, "contains control characters"));
    }
    Ok(s.to_owned())
}

impl FromStr for Issuer {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let text = bounded_text(s, "issuer", 512)?;
        if !(text.starts_with("https://") || text.starts_with("http://")) {
            return Err(ParseError::new("issuer", "must be an http(s) URL"));
        }
        Ok(Self(text))
    }
}

impl FromStr for Subject {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        bounded_text(s, "subject", 255).map(Self)
    }
}

impl FromStr for Email {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let text = bounded_text(s, "email", 254)?;
        match text.split_once('@') {
            Some((local, domain))
                if !local.is_empty() && domain.contains('.') && !text.contains(' ') =>
            {
                Ok(Self(text.to_lowercase()))
            }
            Some(_) | None => Err(ParseError::new("email", "is not an address")),
        }
    }
}

impl FromStr for DisplayName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        bounded_text(s.trim(), "display name", 128).map(Self)
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

display_inner!(Issuer, Subject, Email, DisplayName);

/// An identity the `IdP` vouched for, parsed from verified token claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedIdentity {
    pub issuer: Issuer,
    pub subject: Subject,
    /// Present only when the provider marked the address verified.
    pub email: Option<Email>,
    pub name: Option<DisplayName>,
}

/// One entry in the operator's list of people allowed to sign in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowEntry {
    Subject(Subject),
    VerifiedEmail(Email),
}

/// Who may sign in. Configured by the operator; the first person to sign in
/// gains nothing special.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SignInPolicy(Vec<AllowEntry>);

impl SignInPolicy {
    #[must_use]
    pub const fn new(entries: Vec<AllowEntry>) -> Self {
        Self(entries)
    }

    #[must_use]
    pub fn admits(&self, identity: &VerifiedIdentity) -> bool {
        self.0.iter().any(|entry| match entry {
            AllowEntry::Subject(subject) => *subject == identity.subject,
            AllowEntry::VerifiedEmail(email) => identity.email.as_ref() == Some(email),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalStatus {
    Active,
    Disabled,
}

/// A signed-in principal, as authorization sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Principal {
    pub id: PrincipalId,
    pub status: PrincipalStatus,
}

/// Everything a principal can ask to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ViewWorkspace,
    OperateWorkspace,
    DeleteWorkspace,
    PublishRoute,
    UsePreview,
    ManageSecrets,
    ManageEnvironment,
}

/// The facts about a resource that authorization depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resource {
    pub owner: PrincipalId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(DenyReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DenyReason {
    #[error("this account is disabled")]
    Disabled,
    #[error("only the owner can do this")]
    NotOwner,
}

/// The single authorization decision for every protected request.
///
/// iglu is owner-only for now: an owner may do everything with what they own.
/// Sharing adds grants as another input here and a per-action match.
#[must_use]
pub fn authorize(principal: Principal, action: Action, resource: Resource) -> Decision {
    match principal.status {
        PrincipalStatus::Disabled => return Decision::Deny(DenyReason::Disabled),
        PrincipalStatus::Active => {}
    }
    if principal.id != resource.owner {
        return Decision::Deny(DenyReason::NotOwner);
    }
    match action {
        Action::ViewWorkspace
        | Action::OperateWorkspace
        | Action::DeleteWorkspace
        | Action::PublishRoute
        | Action::UsePreview
        | Action::ManageSecrets
        | Action::ManageEnvironment => Decision::Allow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn principal(n: u128, status: PrincipalStatus) -> Principal {
        Principal {
            id: PrincipalId::from_uuid(Uuid::from_u128(n)),
            status,
        }
    }

    fn owned_by(n: u128) -> Resource {
        Resource {
            owner: PrincipalId::from_uuid(Uuid::from_u128(n)),
        }
    }

    const ACTIONS: [Action; 7] = [
        Action::ViewWorkspace,
        Action::OperateWorkspace,
        Action::DeleteWorkspace,
        Action::PublishRoute,
        Action::UsePreview,
        Action::ManageSecrets,
        Action::ManageEnvironment,
    ];

    #[test]
    fn owners_may_do_everything() {
        for action in ACTIONS {
            assert_eq!(
                authorize(principal(1, PrincipalStatus::Active), action, owned_by(1)),
                Decision::Allow
            );
        }
    }

    #[test]
    fn others_may_do_nothing() {
        for action in ACTIONS {
            assert_eq!(
                authorize(principal(2, PrincipalStatus::Active), action, owned_by(1)),
                Decision::Deny(DenyReason::NotOwner)
            );
        }
    }

    #[test]
    fn disabled_owners_may_do_nothing() {
        for action in ACTIONS {
            assert_eq!(
                authorize(principal(1, PrincipalStatus::Disabled), action, owned_by(1)),
                Decision::Deny(DenyReason::Disabled)
            );
        }
    }

    fn identity(subject: &str, email: Option<&str>) -> VerifiedIdentity {
        VerifiedIdentity {
            issuer: "https://idp.example.com".parse().expect("valid issuer"),
            subject: subject.parse().expect("valid subject"),
            email: email.map(|e| e.parse().expect("valid email")),
            name: None,
        }
    }

    #[test]
    fn sign_in_needs_a_matching_subject_or_verified_email() {
        let policy = SignInPolicy::new(vec![
            AllowEntry::Subject("alice-sub".parse().expect("valid subject")),
            AllowEntry::VerifiedEmail("bob@example.com".parse().expect("valid email")),
        ]);
        assert!(policy.admits(&identity("alice-sub", None)));
        assert!(policy.admits(&identity("other", Some("Bob@Example.com"))));
        assert!(!policy.admits(&identity("mallory", Some("mallory@example.com"))));
        assert!(!SignInPolicy::default().admits(&identity("alice-sub", None)));
    }

    #[test]
    fn emails_are_parsed() {
        assert!("not-an-email".parse::<Email>().is_err());
        assert!("a@b".parse::<Email>().is_err());
        assert!("a b@example.com".parse::<Email>().is_err());
    }
}
