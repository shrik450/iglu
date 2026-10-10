//! Who may sign in and what they may do.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::id::{PrincipalId, WorkspaceId};
use crate::parse::{is_printable, text_type};

/// An OIDC issuer URL, exactly as the provider states it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct Issuer(String);

/// An OIDC subject: stable and unique within its issuer.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct Subject(String);

/// An email address the provider says it verified.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct Email(String);

/// A name for display only. Never used to identify anyone.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
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

impl FromStr for PrincipalStatus {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            _ => Err(ParseError::new(
                "principal status",
                "expected active or disabled",
            )),
        }
    }
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
    ManageProject,
    /// Add, close, restart, label and arrange a workspace's columns.
    ManageColumns,
    /// Read what a column's terminal shows.
    ReadColumnOutput,
    /// Type into a column's terminal: whatever runs there acts on it, so
    /// this is as much as running commands in the workspace.
    SendColumnInput,
}

/// What a workspace may be granted on a workspace of the same owner. Only
/// these can be granted, so a workspace never manages secrets, projects,
/// environments or access, and never creates or deletes a workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "snake_case"))]
pub enum Permission {
    /// See it, its columns and what they're doing.
    View,
    ReadOutput,
    SendInput,
    ManageColumns,
    PublishRoutes,
    /// Start, freeze, stop and rename it.
    Operate,
}

impl Permission {
    pub const ALL: [Self; 6] = [
        Self::View,
        Self::ReadOutput,
        Self::SendInput,
        Self::ManageColumns,
        Self::PublishRoutes,
        Self::Operate,
    ];

    /// What a new workspace may do to itself: work with its own columns and
    /// publish its own ports, but not stop or freeze itself.
    pub const OWN: [Self; 5] = [
        Self::View,
        Self::ReadOutput,
        Self::SendInput,
        Self::ManageColumns,
        Self::PublishRoutes,
    ];

    #[must_use]
    pub const fn action(self) -> Action {
        match self {
            Self::View => Action::ViewWorkspace,
            Self::ReadOutput => Action::ReadColumnOutput,
            Self::SendInput => Action::SendColumnInput,
            Self::ManageColumns => Action::ManageColumns,
            Self::PublishRoutes => Action::PublishRoute,
            Self::Operate => Action::OperateWorkspace,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::ReadOutput => "read_output",
            Self::SendInput => "send_input",
            Self::ManageColumns => "manage_columns",
            Self::PublishRoutes => "publish_routes",
            Self::Operate => "operate",
        }
    }
}

impl FromStr for Permission {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| {
                ParseError::new(
                    "permission",
                    "expected view, read_output, send_input, manage_columns, publish_routes or operate",
                )
            })
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A permission a workspace holds on a workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Grant {
    pub workspace: WorkspaceId,
    pub permission: Permission,
}

/// Who is asking. A workspace acts for its owner but only as far as its
/// grants go: it never stands in for the owner.
#[derive(Clone, Copy, Debug)]
pub enum Actor<'a> {
    Person(Principal),
    Workspace {
        owner: Principal,
        grants: &'a [Grant],
    },
}

/// The facts about a resource that authorization depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resource {
    pub owner: PrincipalId,
    /// The workspace it is or belongs to, if it's one.
    pub workspace: Option<WorkspaceId>,
}

impl Resource {
    #[must_use]
    pub const fn owned_by(owner: PrincipalId) -> Self {
        Self {
            owner,
            workspace: None,
        }
    }

    #[must_use]
    pub const fn workspace(id: WorkspaceId, owner: PrincipalId) -> Self {
        Self {
            owner,
            workspace: Some(id),
        }
    }
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
    #[error("this workspace hasn't been granted that")]
    NotGranted,
}

/// The single authorization decision for every protected request.
///
/// A person may do everything with what they own. A workspace may do what
/// it's been granted on a workspace of its owner's, and nothing else.
#[must_use]
pub fn authorize(actor: Actor<'_>, action: Action, resource: Resource) -> Decision {
    let person = match actor {
        Actor::Person(person) | Actor::Workspace { owner: person, .. } => person,
    };
    match person.status {
        PrincipalStatus::Disabled => return Decision::Deny(DenyReason::Disabled),
        PrincipalStatus::Active => {}
    }
    if person.id != resource.owner {
        return Decision::Deny(DenyReason::NotOwner);
    }
    match actor {
        Actor::Person(_) => match action {
            Action::ViewWorkspace
            | Action::OperateWorkspace
            | Action::DeleteWorkspace
            | Action::PublishRoute
            | Action::UsePreview
            | Action::ManageSecrets
            | Action::ManageEnvironment
            | Action::ManageProject
            | Action::ManageColumns
            | Action::ReadColumnOutput
            | Action::SendColumnInput => Decision::Allow,
        },
        Actor::Workspace { grants, .. } => {
            let granted = resource.workspace.is_some_and(|workspace| {
                grants
                    .iter()
                    .any(|g| g.workspace == workspace && g.permission.action() == action)
            });
            if granted {
                Decision::Allow
            } else {
                Decision::Deny(DenyReason::NotGranted)
            }
        }
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

    fn person(n: u128) -> Actor<'static> {
        Actor::Person(principal(n, PrincipalStatus::Active))
    }

    fn owned_by(n: u128) -> Resource {
        Resource::owned_by(PrincipalId::from_uuid(Uuid::from_u128(n)))
    }

    fn workspace(n: u128, owner: u128) -> Resource {
        Resource::workspace(
            WorkspaceId::from_uuid(Uuid::from_u128(n)),
            PrincipalId::from_uuid(Uuid::from_u128(owner)),
        )
    }

    fn grant(n: u128, permission: Permission) -> Grant {
        Grant {
            workspace: WorkspaceId::from_uuid(Uuid::from_u128(n)),
            permission,
        }
    }

    const ACTIONS: [Action; 11] = [
        Action::ViewWorkspace,
        Action::OperateWorkspace,
        Action::DeleteWorkspace,
        Action::PublishRoute,
        Action::UsePreview,
        Action::ManageSecrets,
        Action::ManageEnvironment,
        Action::ManageProject,
        Action::ManageColumns,
        Action::ReadColumnOutput,
        Action::SendColumnInput,
    ];

    #[test]
    fn owners_may_do_everything() {
        for action in ACTIONS {
            assert_eq!(authorize(person(1), action, owned_by(1)), Decision::Allow);
            assert_eq!(
                authorize(person(1), action, workspace(7, 1)),
                Decision::Allow
            );
        }
    }

    #[test]
    fn others_may_do_nothing() {
        for action in ACTIONS {
            assert_eq!(
                authorize(person(2), action, workspace(7, 1)),
                Decision::Deny(DenyReason::NotOwner)
            );
        }
    }

    #[test]
    fn disabled_owners_may_do_nothing() {
        let disabled = principal(1, PrincipalStatus::Disabled);
        let grants = Permission::ALL.map(|p| grant(7, p));
        for action in ACTIONS {
            assert_eq!(
                authorize(Actor::Person(disabled), action, owned_by(1)),
                Decision::Deny(DenyReason::Disabled)
            );
            let through = Actor::Workspace {
                owner: disabled,
                grants: &grants,
            };
            assert_eq!(
                authorize(through, action, workspace(7, 1)),
                Decision::Deny(DenyReason::Disabled)
            );
        }
    }

    #[test]
    fn a_workspace_may_do_exactly_what_it_was_granted() {
        let grants = [grant(7, Permission::View), grant(8, Permission::SendInput)];
        let actor = Actor::Workspace {
            owner: principal(1, PrincipalStatus::Active),
            grants: &grants,
        };
        assert_eq!(
            authorize(actor, Action::ViewWorkspace, workspace(7, 1)),
            Decision::Allow
        );
        assert_eq!(
            authorize(actor, Action::SendColumnInput, workspace(8, 1)),
            Decision::Allow
        );
        // Not on another workspace, and not another permission.
        assert_eq!(
            authorize(actor, Action::SendColumnInput, workspace(7, 1)),
            Decision::Deny(DenyReason::NotGranted)
        );
        assert_eq!(
            authorize(actor, Action::ViewWorkspace, workspace(9, 1)),
            Decision::Deny(DenyReason::NotGranted)
        );
    }

    #[test]
    fn a_workspace_never_acts_as_its_owner() {
        let grants = Permission::ALL.map(|p| grant(7, p));
        let actor = Actor::Workspace {
            owner: principal(1, PrincipalStatus::Active),
            grants: &grants,
        };
        // What isn't a workspace's to have, whatever it holds.
        for action in [
            Action::DeleteWorkspace,
            Action::UsePreview,
            Action::ManageSecrets,
            Action::ManageEnvironment,
            Action::ManageProject,
        ] {
            assert_eq!(
                authorize(actor, action, workspace(7, 1)),
                Decision::Deny(DenyReason::NotGranted)
            );
            assert_eq!(
                authorize(actor, action, owned_by(1)),
                Decision::Deny(DenyReason::NotGranted)
            );
        }
        // Nor anything of someone else's, even with a grant naming it.
        assert_eq!(
            authorize(actor, Action::ViewWorkspace, workspace(7, 2)),
            Decision::Deny(DenyReason::NotOwner)
        );
    }

    #[test]
    fn a_new_workspace_works_with_its_columns_but_cant_stop_itself() {
        assert!(Permission::OWN.contains(&Permission::ManageColumns));
        assert!(!Permission::OWN.contains(&Permission::Operate));
    }

    #[test]
    fn permissions_are_named_as_stored() {
        for permission in Permission::ALL {
            assert_eq!(permission.as_str().parse::<Permission>(), Ok(permission));
        }
        assert!("delete".parse::<Permission>().is_err());
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
