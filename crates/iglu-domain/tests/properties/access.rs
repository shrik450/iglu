//! Who may do what: authorization, sign-in, and the rules that keep
//! previews and the console apart. The access rules are finite, so they're
//! checked on every combination; authorization takes generated grants.

use iglu_domain::auth::{
    Action, Actor, AllowEntry, Decision, DenyReason, Email, Grant, Issuer, Permission, Principal,
    PrincipalStatus, Resource, SignInPolicy, Subject, VerifiedIdentity, authorize,
};
use iglu_domain::id::{PrincipalId, WorkspaceId};
use iglu_domain::preview::{
    ConsoleOrigin, ConsoleRejection, Credential, FetchMetadata, FetchMode, FetchSite, MethodClass,
    PreviewAccess, RequestSource, Transport, console_access, preview_access,
};
use proptest::prelude::*;
use uuid::Uuid;

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

/// A few IDs, so generated principals and workspaces often coincide.
fn principal_id() -> impl Strategy<Value = PrincipalId> {
    (0..3u128).prop_map(|n| PrincipalId::from_uuid(Uuid::from_u128(n)))
}

fn workspace_id() -> impl Strategy<Value = WorkspaceId> {
    (0..4u128).prop_map(|n| WorkspaceId::from_uuid(Uuid::from_u128(n)))
}

fn principal() -> impl Strategy<Value = Principal> {
    (
        principal_id(),
        prop_oneof![3 => Just(PrincipalStatus::Active), 1 => Just(PrincipalStatus::Disabled)],
    )
        .prop_map(|(id, status)| Principal { id, status })
}

fn grants() -> impl Strategy<Value = Vec<Grant>> {
    prop::collection::vec(
        (workspace_id(), prop::sample::select(&Permission::ALL[..])).prop_map(
            |(workspace, permission)| Grant {
                workspace,
                permission,
            },
        ),
        0..8,
    )
}

fn resource() -> impl Strategy<Value = Resource> {
    (principal_id(), prop::option::of(workspace_id()))
        .prop_map(|(owner, workspace)| Resource { owner, workspace })
}

proptest! {
    #[test]
    fn nobody_acts_on_what_another_person_owns(
        person in principal(), grants in grants(), action in prop::sample::select(&ACTIONS[..]), workspace in prop::option::of(workspace_id()),
    ) {
        // Someone the generated people never are.
        let resource = Resource { owner: PrincipalId::from_uuid(Uuid::from_u128(99)), workspace };
        for actor in [Actor::Person(person), Actor::Workspace { owner: person, grants: &grants }] {
            let decided = authorize(actor, action, resource);
            prop_assert!(matches!(decided, Decision::Deny(DenyReason::NotOwner | DenyReason::Disabled)), "{decided:?}");
        }
    }

    #[test]
    fn a_disabled_account_does_nothing(
        id in principal_id(), grants in grants(), action in prop::sample::select(&ACTIONS[..]), resource in resource(),
    ) {
        let person = Principal { id, status: PrincipalStatus::Disabled };
        for actor in [Actor::Person(person), Actor::Workspace { owner: person, grants: &grants }] {
            prop_assert_eq!(authorize(actor, action, resource), Decision::Deny(DenyReason::Disabled));
        }
    }

    #[test]
    fn an_active_person_does_everything_with_what_they_own(
        id in principal_id(), action in prop::sample::select(&ACTIONS[..]), workspace in prop::option::of(workspace_id()),
    ) {
        let person = Principal { id, status: PrincipalStatus::Active };
        prop_assert_eq!(authorize(Actor::Person(person), action, Resource { owner: id, workspace }), Decision::Allow);
    }

    #[test]
    fn a_workspace_does_only_what_it_was_granted_on_that_workspace(
        owner in principal(), grants in grants(), action in prop::sample::select(&ACTIONS[..]), resource in resource(),
    ) {
        let decided = authorize(Actor::Workspace { owner, grants: &grants }, action, resource);
        if decided == Decision::Allow {
            prop_assert_eq!(owner.status, PrincipalStatus::Active);
            prop_assert_eq!(owner.id, resource.owner);
            let on_it: Vec<Permission> = grants
                .iter()
                .filter(|g| Some(g.workspace) == resource.workspace)
                .map(|g| g.permission)
                .collect();
            // Any grant includes seeing; otherwise only the granted action.
            prop_assert!(
                on_it.iter().any(|p| p.action() == action) || (action == Action::ViewWorkspace && !on_it.is_empty()),
                "{action:?} allowed with {on_it:?}"
            );
        }
        // What can't be granted is never allowed to a workspace.
        if matches!(action, Action::DeleteWorkspace | Action::UsePreview | Action::ManageSecrets | Action::ManageEnvironment | Action::ManageProject) {
            prop_assert_ne!(decided, Decision::Allow);
        }
    }

    #[test]
    fn more_grants_never_allow_less(
        owner in principal(), grants in grants(), more in grants(), action in prop::sample::select(&ACTIONS[..]), resource in resource(),
    ) {
        let all: Vec<Grant> = grants.iter().chain(&more).copied().collect();
        if authorize(Actor::Workspace { owner, grants: &grants }, action, resource) == Decision::Allow {
            prop_assert_eq!(authorize(Actor::Workspace { owner, grants: &all }, action, resource), Decision::Allow);
        }
    }

    #[test]
    fn a_workspace_never_does_more_than_its_owner(
        owner in principal(), grants in grants(), action in prop::sample::select(&ACTIONS[..]), resource in resource(),
    ) {
        if authorize(Actor::Workspace { owner, grants: &grants }, action, resource) == Decision::Allow {
            prop_assert_eq!(authorize(Actor::Person(owner), action, resource), Decision::Allow);
        }
    }

    #[test]
    fn sign_in_admits_exactly_who_the_policy_lists(
        entries in prop::collection::vec((any::<bool>(), 0..4u8), 0..4),
        subject in 0..4u8,
        email in prop::option::of(0..4u8),
    ) {
        let subject_of = |n: u8| format!("subject-{n}").parse::<Subject>().expect("a subject");
        let email_of = |n: u8| format!("person{n}@example.org").parse::<Email>().expect("an email");
        let policy = SignInPolicy::new(
            entries
                .iter()
                .map(|&(by_subject, n)| if by_subject { AllowEntry::Subject(subject_of(n)) } else { AllowEntry::VerifiedEmail(email_of(n)) })
                .collect(),
        );
        let identity = VerifiedIdentity {
            issuer: "https://idp.example.org".parse::<Issuer>().expect("an issuer"),
            subject: subject_of(subject),
            email: email.map(email_of),
            name: None,
        };
        let listed = entries.iter().any(|&(by_subject, n)| if by_subject { n == subject } else { email == Some(n) });
        prop_assert_eq!(policy.admits(&identity), listed);
    }
}

const SITES: [Option<FetchSite>; 5] = [
    None,
    Some(FetchSite::SameOrigin),
    Some(FetchSite::SameSite),
    Some(FetchSite::CrossSite),
    Some(FetchSite::None),
];
const MODES: [Option<FetchMode>; 6] = [
    None,
    Some(FetchMode::Navigate),
    Some(FetchMode::Cors),
    Some(FetchMode::NoCors),
    Some(FetchMode::SameOrigin),
    Some(FetchMode::WebSocket),
];
const TRANSPORTS: [Transport; 3] = [
    Transport::Http(MethodClass::Safe),
    Transport::Http(MethodClass::Unsafe),
    Transport::WebSocket,
];
const SOURCES: [RequestSource; 4] = [
    RequestSource::Unknown,
    RequestSource::SameOrigin,
    RequestSource::SameWorkspace,
    RequestSource::Elsewhere,
];
const ORIGINS: [ConsoleOrigin; 3] = [
    ConsoleOrigin::Absent,
    ConsoleOrigin::Exact,
    ConsoleOrigin::Other,
];

fn every_request() -> impl Iterator<Item = (Transport, FetchMetadata)> {
    TRANSPORTS.into_iter().flat_map(|transport| {
        SITES.into_iter().flat_map(move |site| {
            MODES
                .into_iter()
                .map(move |mode| (transport, FetchMetadata { site, mode }))
        })
    })
}

#[test]
fn previews_take_whatever_comes_from_their_own_workspace() {
    for (transport, fetch) in every_request() {
        for source in [RequestSource::SameOrigin, RequestSource::SameWorkspace] {
            assert_eq!(
                preview_access(transport, fetch, source),
                PreviewAccess::Forward,
                "{transport:?} {fetch:?} {source:?}"
            );
        }
    }
}

#[test]
fn another_site_reaches_a_preview_only_by_navigating_to_it() {
    for (transport, fetch) in every_request() {
        let from_another_site =
            matches!(fetch.site, Some(FetchSite::SameSite | FetchSite::CrossSite));
        let navigates = transport == Transport::Http(MethodClass::Safe)
            && fetch.mode == Some(FetchMode::Navigate);
        for source in [RequestSource::Unknown, RequestSource::Elsewhere] {
            let access = preview_access(transport, fetch, source);
            if transport != Transport::WebSocket && from_another_site && !navigates {
                assert_eq!(
                    access,
                    PreviewAccess::Reject,
                    "{transport:?} {fetch:?} {source:?}"
                );
            }
        }
        // A WebSocket from elsewhere is refused, whatever it claims.
        if transport == Transport::WebSocket {
            assert_eq!(
                preview_access(transport, fetch, RequestSource::Elsewhere),
                PreviewAccess::Reject
            );
        }
    }
}

#[test]
fn trusting_a_source_more_never_rejects_more() {
    // Unknown and elsewhere are untrusted; same origin and same workspace
    // are trusted. Anything an untrusted source gets, a trusted one gets.
    for (transport, fetch) in every_request() {
        for untrusted in [RequestSource::Unknown, RequestSource::Elsewhere] {
            if preview_access(transport, fetch, untrusted) == PreviewAccess::Forward {
                for trusted in [RequestSource::SameOrigin, RequestSource::SameWorkspace] {
                    assert_eq!(
                        preview_access(transport, fetch, trusted),
                        PreviewAccess::Forward
                    );
                }
            }
        }
    }
    assert_eq!(SOURCES.len(), 4, "a new source needs a place in this order");
}

#[test]
fn a_cookie_never_changes_state_from_another_origin() {
    for (transport, fetch) in every_request() {
        for origin in ORIGINS {
            let accepted = console_access(Credential::Cookie, transport, fetch, origin).is_ok();
            match transport {
                Transport::WebSocket => assert_eq!(accepted, origin == ConsoleOrigin::Exact),
                Transport::Http(MethodClass::Safe) => assert!(accepted),
                Transport::Http(MethodClass::Unsafe) => assert_eq!(
                    accepted,
                    fetch.site == Some(FetchSite::SameOrigin) && origin != ConsoleOrigin::Other,
                    "{fetch:?} {origin:?}"
                ),
            }
            // A bearer token can't be sent by a page, so it's never refused here.
            assert_eq!(
                console_access(Credential::Bearer, transport, fetch, origin),
                Ok(())
            );
        }
    }
}

#[test]
fn console_refusals_name_the_right_reason() {
    for (transport, fetch) in every_request() {
        for origin in ORIGINS {
            match console_access(Credential::Cookie, transport, fetch, origin) {
                Err(ConsoleRejection::CrossOriginWebSocket) => {
                    assert_eq!(transport, Transport::WebSocket);
                }
                Err(ConsoleRejection::CrossOriginMutation) => {
                    assert_eq!(transport, Transport::Http(MethodClass::Unsafe));
                }
                Ok(()) => {}
            }
        }
    }
}

#[test]
fn fetch_metadata_reads_as_browsers_send_it() {
    for (sent, site) in [
        ("same-origin", FetchSite::SameOrigin),
        ("same-site", FetchSite::SameSite),
        ("cross-site", FetchSite::CrossSite),
        ("none", FetchSite::None),
    ] {
        assert_eq!(sent.parse::<FetchSite>(), Ok(site));
    }
    for (sent, mode) in [
        ("navigate", FetchMode::Navigate),
        ("cors", FetchMode::Cors),
        ("no-cors", FetchMode::NoCors),
        ("same-origin", FetchMode::SameOrigin),
        ("websocket", FetchMode::WebSocket),
    ] {
        assert_eq!(sent.parse::<FetchMode>(), Ok(mode));
    }
    // What a browser never sends reads as nothing it knows.
    for sent in ["", "Same-Origin", "same_origin", "nested-navigate", " cors"] {
        assert!(
            sent.parse::<FetchSite>().is_err() && sent.parse::<FetchMode>().is_err(),
            "{sent:?}"
        );
    }
}
