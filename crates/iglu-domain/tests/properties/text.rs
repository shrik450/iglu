//! Parsing text into the core's types: whatever comes in, parsing never
//! panics, and what it accepts survives being shown, sent and stored.

use iglu_domain::attention::{AttentionState, Summary, ThreadKey};
use iglu_domain::auth::{DisplayName, Email, Issuer, Permission, PrincipalStatus, Subject};
use iglu_domain::column::{Arg, ColumnLabel, ColumnWidth};
use iglu_domain::display::DisplayScale;
use iglu_domain::download::DownloadName;
use iglu_domain::env::{EnvSource, FlakeAttr, FlakeRef, GuestPath, ImageFingerprint, UnixUser};
use iglu_domain::id::{InstanceName, WorkspaceId};
use iglu_domain::label::{DnsLabel, HostId, ProjectName, RouteName, WorkspaceName};
use iglu_domain::lifecycle::{DesiredState, Phase};
use iglu_domain::names::WorkspaceLabel;
use iglu_domain::pasted::FileName;
use iglu_domain::port::GuestPort;
use iglu_domain::preferences::{FontName, KeyCode, PastedTheme, ThemeName};
use iglu_domain::repo::{BranchName, CheckoutDir, GitHost, RepoUrl};
use iglu_domain::secret::{EnvVarName, GitUsername, HomePath, SecretName, SecretValue};
use iglu_domain::signin::ReturnPath;
use iglu_domain::terminal::{OutputLines, SessionName, TerminalInput};
use proptest::prelude::*;

use crate::support::{round_trips, shows_and_parses, text};

/// Types whose parsing keeps the text as it is.
macro_rules! keeps_its_text {
    ($($test:ident: $ty:ty,)*) => {$(
        proptest! {
            #[test]
            fn $test(s in text()) {
                if let Ok(value) = s.parse::<$ty>() {
                    prop_assert_eq!(value.to_string(), s);
                    round_trips(&value)?;
                }
            }
        }
    )*};
}

keeps_its_text! {
    dns_labels: DnsLabel,
    host_ids: HostId,
    workspace_names: WorkspaceName,
    project_names: ProjectName,
    route_names: RouteName,
    workspace_labels: WorkspaceLabel,
    session_names: SessionName,
    terminal_input: TerminalInput,
    download_names: DownloadName,
    file_names: FileName,
    issuers: Issuer,
    subjects: Subject,
    secret_names: SecretName,
    env_var_names: EnvVarName,
    home_paths: HomePath,
    git_usernames: GitUsername,
    flake_refs: FlakeRef,
    flake_attrs: FlakeAttr,
    env_sources: EnvSource,
    image_fingerprints: ImageFingerprint,
    unix_users: UnixUser,
    guest_paths: GuestPath,
    repo_urls: RepoUrl,
    checkout_dirs: CheckoutDir,
    branch_names: BranchName,
    args: Arg,
    column_labels: ColumnLabel,
    column_widths: ColumnWidth,
    attention_states: AttentionState,
    thread_keys: ThreadKey,
    key_codes: KeyCode,
    font_names: FontName,
    theme_names: ThemeName,
    pasted_themes: PastedTheme,
    instance_names: InstanceName,
    desired_states: DesiredState,
    phases: Phase,
    permissions: Permission,
    principal_statuses: PrincipalStatus,
}

/// Types that normalize what they're given, once.
macro_rules! normalizes {
    ($($test:ident: $ty:ty,)*) => {$(
        proptest! {
            #[test]
            fn $test(s in text()) {
                if let Ok(value) = s.parse::<$ty>() {
                    round_trips(&value)?;
                }
            }
        }
    )*};
}

/// Types whose `as_str` is their text, as it's shown.
macro_rules! say_their_text {
    ($($test:ident: $ty:ty,)*) => {$(
        proptest! {
            #[test]
            fn $test(s in text()) {
                if let Ok(value) = s.parse::<$ty>() {
                    prop_assert_eq!(value.as_str(), value.to_string());
                }
            }
        }
    )*};
}

say_their_text! {
    dns_label_text: DnsLabel,
    workspace_name_text: WorkspaceName,
    session_name_text: SessionName,
    terminal_input_text: TerminalInput,
    download_name_text: DownloadName,
    issuer_text: Issuer,
    subject_text: Subject,
    email_text: Email,
    display_name_text: DisplayName,
    secret_name_text: SecretName,
    env_var_name_text: EnvVarName,
    home_path_text: HomePath,
    git_username_text: GitUsername,
    flake_ref_text: FlakeRef,
    flake_attr_text: FlakeAttr,
    image_fingerprint_text: ImageFingerprint,
    unix_user_text: UnixUser,
    guest_path_text: GuestPath,
    git_host_text: GitHost,
    repo_url_text: RepoUrl,
    checkout_dir_text: CheckoutDir,
    branch_name_text: BranchName,
    arg_text: Arg,
    column_label_text: ColumnLabel,
    thread_key_text: ThreadKey,
    summary_text: Summary,
    workspace_label_text: WorkspaceLabel,
    return_path_text: ReturnPath,
}

normalizes! {
    emails: Email,
    display_names: DisplayName,
    git_hosts: GitHost,
    summaries: Summary,
    output_lines: OutputLines,
    guest_ports: GuestPort,
    workspace_ids: WorkspaceId,
}

proptest! {
    #[test]
    fn emails_are_lowercase(s in text()) {
        if let Ok(email) = s.parse::<Email>() {
            prop_assert_eq!(email.as_str(), email.as_str().to_lowercase());
        }
    }

    #[test]
    fn display_names_have_no_space_at_either_end(s in text()) {
        if let Ok(name) = s.parse::<DisplayName>() {
            prop_assert_eq!(name.as_str(), name.as_str().trim());
        }
    }

    #[test]
    fn git_hosts_are_lowercase(s in text()) {
        if let Ok(host) = s.parse::<GitHost>() {
            prop_assert_eq!(host.as_str(), host.as_str().to_ascii_lowercase());
        }
    }

    #[test]
    fn named_labels_are_dns_labels_that_start_with_a_letter(s in text()) {
        let label = s.parse::<DnsLabel>();
        let starts_with_letter = s.bytes().next().is_some_and(|b| b.is_ascii_lowercase());
        let named = label.is_ok() && starts_with_letter;
        prop_assert_eq!(s.parse::<WorkspaceName>().is_ok(), named);
        prop_assert_eq!(s.parse::<ProjectName>().is_ok(), named);
        prop_assert_eq!(s.parse::<RouteName>().is_ok(), named && s != "auth");
        prop_assert_eq!(s.parse::<HostId>().is_ok(), label.is_ok());
    }

    #[test]
    fn dns_labels_take_their_whole_grammar(s in "[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?") {
        prop_assert!(s.parse::<DnsLabel>().is_ok());
    }

    #[test]
    fn session_names_take_their_whole_grammar(s in "[a-z0-9][a-z0-9-]{0,31}") {
        prop_assert!(s.parse::<SessionName>().is_ok());
    }

    #[test]
    fn file_names_take_their_whole_grammar(s in "[A-Za-z0-9_][A-Za-z0-9._-]{0,79}") {
        prop_assert!(s.parse::<FileName>().is_ok());
    }

    #[test]
    fn thread_keys_take_their_whole_grammar(s in "[A-Za-z0-9_-]{1,64}") {
        prop_assert!(s.parse::<ThreadKey>().is_ok());
    }

    #[test]
    fn home_paths_take_relative_paths(parts in prop::collection::vec("[a-z0-9_][a-z0-9._-]{0,20}", 1..8)) {
        let path = parts.join("/");
        let parsed = path.parse::<HomePath>();
        prop_assert!(parsed.is_ok(), "{path:?}");
        let parsed = parsed.expect("checked just now");
        // Every parent is a proper prefix that ends where a component does.
        let parents = parsed.parents();
        prop_assert_eq!(parents.len(), parts.len() - 1);
        for parent in parents {
            prop_assert!(path.starts_with(&format!("{parent}/")), "{:?} isn't a parent of {:?}", parent, path);
        }
    }

    #[test]
    fn home_paths_never_leave_home(s in text()) {
        if let Ok(path) = s.parse::<HomePath>() {
            prop_assert!(!path.as_str().starts_with('/'));
            prop_assert!(path.as_str().split('/').all(|part| !part.is_empty() && part != "." && part != ".."));
        }
    }

    #[test]
    fn guest_paths_have_no_dot_components(s in text()) {
        if let Ok(path) = s.parse::<GuestPath>() {
            prop_assert!(path.as_str().starts_with('/'));
            prop_assert!(path.as_str().split('/').all(|part| part != "." && part != ".."));
        }
    }

    #[test]
    fn env_var_names_leave_the_platform_its_own(s in "[A-Z_][A-Z0-9_]{0,40}") {
        if s.starts_with("IGLU_") || s.starts_with("ZMX_") || ["HOME", "PATH", "USER", "LOGNAME", "SHELL", "TERM", "XDG_RUNTIME_DIR"].contains(&s.as_str()) {
            prop_assert!(s.parse::<EnvVarName>().is_err());
        } else {
            prop_assert!(s.parse::<EnvVarName>().is_ok());
        }
    }

    #[test]
    fn branch_names_never_read_as_options_or_revisions(s in text()) {
        if let Ok(branch) = s.parse::<BranchName>() {
            let b = branch.as_str();
            prop_assert!(!b.starts_with('-') && !b.contains("..") && !b.contains("@{"), "{:?}", b);
            prop_assert!(!b.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c)));
        }
    }

    #[test]
    fn repo_urls_check_out_into_one_safe_directory(s in text()) {
        if let Ok(url) = s.parse::<RepoUrl>() {
            let dir = url.checkout_dir().as_str();
            prop_assert!(dir != "." && dir != ".." && !dir.contains('/') && !dir.starts_with('-'));
        }
    }

    #[test]
    fn scp_like_and_https_repo_urls_name_their_host(
        host in "[a-z][a-z0-9]{0,10}(\\.[a-z][a-z0-9]{0,10}){0,3}",
        owner in "[a-z0-9][a-z0-9_.-]{0,15}",
        repo in "[a-z0-9_][a-z0-9_-]{0,15}",
        scp in any::<bool>(),
    ) {
        let url = if scp {
            format!("git@{host}:{owner}/{repo}.git")
        } else {
            format!("https://{host}/{owner}/{repo}.git")
        };
        let parsed = url.parse::<RepoUrl>();
        prop_assert!(parsed.is_ok(), "{url:?}");
        let parsed = parsed.expect("checked just now");
        prop_assert_eq!(parsed.host().as_str(), host);
        prop_assert_eq!(parsed.checkout_dir().as_str(), repo);
    }

    #[test]
    fn secret_values_round_trip(s in text()) {
        if let Ok(value) = SecretValue::try_from(s.clone()) {
            prop_assert_eq!(value.expose(), s.as_str());
            let json = serde_json::to_string(&value).expect("a string serializes");
            let read: SecretValue = serde_json::from_str(&json).expect("what was written reads back");
            prop_assert_eq!(read.expose(), s.as_str());
            // Never shown, however it's printed.
            prop_assert_eq!(format!("{value:?}"), "SecretValue(<redacted>)");
        }
    }

    #[test]
    fn display_scales_stay_between_one_and_four(value in any::<f64>()) {
        if let Ok(scale) = DisplayScale::try_from(value) {
            prop_assert!((100..=400).contains(&scale.hundredths()));
            prop_assert_eq!(DisplayScale::try_from(f64::from(scale)).ok(), Some(scale));
            round_trips(&scale)?;
        }
    }

    #[test]
    fn display_scales_parse_from_any_text(s in text()) {
        if let Ok(scale) = s.parse::<DisplayScale>() {
            shows_and_parses(&scale)?;
        }
    }

    #[test]
    fn instance_names_are_their_workspaces(id in any::<u128>()) {
        let workspace = WorkspaceId::from_uuid(uuid::Uuid::from_u128(id));
        let instance = workspace.instance_name();
        prop_assert_eq!(instance.workspace(), workspace);
        round_trips(&instance)?;
        round_trips(&workspace)?;
    }

    #[test]
    fn output_lines_are_1_to_2000(lines in prop_oneof![0..2100u16, any::<u16>()]) {
        let parsed = OutputLines::try_from(lines);
        prop_assert_eq!(parsed.is_ok(), (1..=2000).contains(&lines));
        if let Ok(parsed) = parsed {
            prop_assert_eq!(u16::from(parsed), lines);
            prop_assert_eq!(parsed.get(), lines);
            round_trips(&parsed)?;
        }
    }

    #[test]
    fn guest_ports_are_every_nonzero_port(port in any::<u16>()) {
        let parsed = GuestPort::try_from(port);
        prop_assert_eq!(parsed.is_ok(), port != 0);
        if let Ok(parsed) = parsed {
            prop_assert_eq!(parsed.get(), port);
            round_trips(&parsed)?;
        }
    }

    #[test]
    fn return_paths_stay_on_the_console(s in return_path_like()) {
        if let Ok(path) = s.parse::<ReturnPath>() {
            shows_and_parses(&path)?;
            // Where a browser lands, following the path from the console.
            let console = url::Url::parse("https://iglu.example.com/settings").expect("a URL");
            let landed = console.join(path.as_str());
            prop_assert!(landed.is_ok(), "{path:?} doesn't resolve");
            let landed = landed.expect("checked just now");
            prop_assert_eq!(landed.origin(), console.origin(), "{:?} leaves the console for {}", path, landed);
        }
    }
}

/// Text that looks like a path, weighted toward what browsers read as
/// another host: doubled and back slashes, schemes, `@` and dot segments.
fn return_path_like() -> impl Strategy<Value = String> {
    let piece = prop::sample::select(
        &[
            "/",
            "//",
            "\\",
            "/\\",
            ".",
            "..",
            "@",
            ":",
            "%2F",
            "%5C",
            "%2E",
            "?",
            "#",
            "evil.example",
            "a",
            "https:",
            "\t",
            "%09",
            "%0A",
            ";",
            "~",
        ][..],
    );
    prop_oneof![
        prop::collection::vec(piece, 0..8).prop_map(|pieces| pieces.concat()),
        text(),
    ]
}

proptest! {
    #[test]
    fn emails_whose_case_changes_their_length_round_trip(local in "[a-zİK\u{212a}]{1,130}") {
        if let Ok(email) = format!("{local}@example.org").parse::<Email>() {
            round_trips(&email)?;
        }
    }

    // Values near zero are drawn often, where a boundary would be off by one.
    #[test]
    fn durations_off_the_wire_are_exactly_the_non_negative_ones_and_go_back_as_sent(
        millis in prop_oneof![any::<i64>(), -2i64..=2],
    ) {
        let parsed = serde_json::from_str::<iglu_domain::time::Millis>(&millis.to_string());
        prop_assert_eq!(parsed.is_ok(), millis >= 0, "{:?}", parsed);
        if let Ok(duration) = parsed {
            prop_assert_eq!(serde_json::to_string(&duration)?, millis.to_string());
        }
    }
}

#[test]
fn names_from_text_take_any_attempt() {
    assert!(iglu_domain::names::from_label("a", u32::MAX).is_some());
}
