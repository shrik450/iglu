//! What the core makes of untrusted text and lists: sanitized names and
//! summaries, cut terminal output, merged listeners, kept threads, file
//! names from browsers, cookies from preview apps, capacity and secrets.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use iglu_domain::attention::{
    AttentionState, MAX_THREADS, Seen, SessionStatus, Summary, ThreadKey, cap, keep, most_urgent,
    urgency,
};
use iglu_domain::capacity::{Bytes, Capacity, Room, admit};
use iglu_domain::download::DownloadName;
use iglu_domain::env::GuestUser;
use iglu_domain::git::ObservedBranch;
use iglu_domain::lifecycle::SecretsGeneration;
use iglu_domain::listener::{Listener, ProcessName, merge, reachable};
use iglu_domain::pasted::FileName;
use iglu_domain::port::GuestPort;
use iglu_domain::preview::{PREVIEW_COOKIE, preview_may_set_cookie};
use iglu_domain::repo::GitHost;
use iglu_domain::secret::{EnvVarName, GitUsername, HomePath, SecretTarget, SecretValue, bundle};
use iglu_domain::signin::same_browser;
use iglu_domain::terminal::{OUTPUT_MAX_BYTES, OutputLines, SessionName, close_reason, last_lines};
use iglu_domain::time::{Millis, Timestamp};
use proptest::prelude::*;

use crate::support::text;

proptest! {
    #[test]
    fn sanitized_process_names_are_clean_and_settle(s in text()) {
        let name = ProcessName::sanitize(&s);
        prop_assert!(name.as_str().chars().count() <= ProcessName::MAX_CHARS);
        prop_assert!(!name.as_str().chars().any(char::is_control));
        prop_assert_eq!(name.as_str(), name.as_str().trim());
        // What a guest sends after sanitizing is what the host accepts.
        let accepted = ProcessName::try_from(name.as_str().to_owned()).ok();
        prop_assert_eq!(accepted.as_ref(), Some(&name));
        let json = serde_json::to_string(&name).expect("a name serializes");
        let read = serde_json::from_str::<ProcessName>(&json).ok();
        prop_assert_eq!(read.as_ref(), Some(&name));
    }

    #[test]
    fn sanitized_branches_are_clean_and_settle(s in text()) {
        if let Some(branch) = ObservedBranch::sanitize(&s) {
            prop_assert!(!branch.as_str().is_empty());
            prop_assert!(branch.as_str().chars().count() <= ObservedBranch::MAX_CHARS);
            prop_assert!(!branch.as_str().chars().any(char::is_control));
            prop_assert_eq!(branch.as_str(), branch.as_str().trim());
            let accepted = ObservedBranch::try_from(branch.as_str().to_owned()).ok();
            prop_assert_eq!(accepted.as_ref(), Some(&branch));
            let sent = serde_json::to_string(&branch)?;
            prop_assert_eq!(serde_json::from_str::<ObservedBranch>(&sent).ok(), Some(branch.clone()));
        }
    }

    #[test]
    fn sanitized_summaries_are_one_clean_line(s in text()) {
        let summary = Summary::sanitize(&s);
        prop_assert!(summary.as_str().chars().count() <= Summary::MAX_CHARS);
        prop_assert!(!summary.as_str().chars().any(|c| c.is_control() || c == '\u{2028}' || c == '\u{2029}'), "{:?}", summary);
        prop_assert_eq!(summary.as_str(), summary.as_str().trim());
        prop_assert_eq!(&Summary::sanitize(summary.as_str()), &summary);
    }

    #[test]
    fn last_lines_are_the_end_of_the_history(history in history(), lines in 1..=OutputLines::MOST) {
        let lines = OutputLines::try_from(lines).expect("in range");
        let (text, truncated) = last_lines(&history, lines);
        prop_assert!(text.len() <= OUTPUT_MAX_BYTES);
        prop_assert!(text.lines().count() <= usize::from(lines.get()));
        let all = history.trim_end().lines().collect::<Vec<_>>().join("\n");
        prop_assert!(all.ends_with(&text), "not the end of the history");
        prop_assert_eq!(truncated, text != all, "truncated says {} for {:?}", truncated, text);
        // As much as 128 KiB of the lines asked for comes back whole.
        if all.lines().count() <= usize::from(lines.get()) && all.len() <= 128 * 1024 {
            prop_assert_eq!(&text, &all);
        }
    }

    #[test]
    fn close_reasons_are_the_longest_start_that_fits(s in text()) {
        let reason = close_reason(&s);
        prop_assert!(reason.len() <= 123);
        prop_assert!(s.starts_with(reason));
        if let Some(next) = s[reason.len()..].chars().next() {
            prop_assert!(reason.len() + next.len_utf8() > 123);
        }
    }

    #[test]
    fn merged_listeners_are_one_per_port_in_order(listeners in prop::collection::vec(listener(), 0..30), limit in 0..20usize) {
        let merged = merge(listeners.clone(), limit);
        let ports: Vec<GuestPort> = merged.iter().map(|l| l.port).collect();
        let distinct: BTreeSet<GuestPort> = listeners.iter().map(|l| l.port).collect();
        prop_assert_eq!(ports, distinct.into_iter().take(limit).collect::<Vec<_>>());
        for listener in &merged {
            let same: Vec<&Listener> = listeners.iter().filter(|l| l.port == listener.port).collect();
            prop_assert_eq!(listener.reachable, same.iter().any(|l| l.reachable));
            prop_assert_eq!(listener.column.is_some(), same.iter().any(|l| l.column.is_some()));
        }
    }

    #[test]
    fn loopback_and_wildcards_are_reachable_and_nothing_else(v4 in any::<[u8; 4]>(), v6 in any::<[u16; 8]>()) {
        let v4 = Ipv4Addr::from(v4);
        prop_assert_eq!(reachable(IpAddr::V4(v4)), v4.is_loopback() || v4.is_unspecified());
        let v6 = Ipv6Addr::from(v6);
        let mapped = v6.to_ipv4_mapped();
        prop_assert_eq!(
            reachable(IpAddr::V6(v6)),
            v6.is_unspecified() || mapped.is_some_and(|v4| v4.is_loopback() || v4.is_unspecified())
        );
        prop_assert!(!reachable(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn kept_threads_are_the_most_pressing(threads in prop::collection::vec((state(), any::<i64>()), 0..40)) {
        let threads: Vec<(usize, AttentionState, Timestamp)> = threads
            .into_iter()
            .enumerate()
            .map(|(i, (s, t))| (i, s, Timestamp::from_unix_millis(t)))
            .collect();
        let kept = keep(&threads);
        prop_assert_eq!(kept.len(), threads.len().min(MAX_THREADS));
        prop_assert_eq!(kept.iter().collect::<BTreeSet<_>>().len(), kept.len());
        // Waiting before working before the rest, newest first: nothing
        // dropped comes before anything kept.
        let rank = |i: usize| {
            let (_, state, at) = threads[i];
            let pressing = match state {
                AttentionState::Waiting => 0,
                AttentionState::Working => 1,
                AttentionState::Done => 2,
                AttentionState::Idle => 3,
            };
            (pressing, std::cmp::Reverse(at))
        };
        let last_kept = kept.iter().map(|&i| rank(i)).max();
        for (i, _, _) in &threads {
            if !kept.contains(i) {
                prop_assert!(Some(rank(*i)) >= last_kept);
            }
        }
    }

    #[test]
    fn capped_statuses_keep_what_keep_would(statuses in prop::collection::vec(status(), 0..30), limit in 0..20usize) {
        let capped = cap(statuses.clone(), limit);
        prop_assert_eq!(capped.len(), statuses.len().min(limit));
        for status in &capped {
            prop_assert!(statuses.contains(status));
        }
    }

    #[test]
    fn the_most_urgent_status_is_at_least_as_urgent_as_every_other(statuses in prop::collection::vec((status(), any::<bool>()), 0..10)) {
        let seen = |unseen: bool| if unseen { Seen::Unseen } else { Seen::Seen };
        let top = most_urgent(statuses.iter().map(|(s, u)| (s, seen(*u))));
        prop_assert_eq!(top.is_some(), !statuses.is_empty());
        if let Some((top, top_seen)) = top {
            for (status, unseen) in &statuses {
                prop_assert!(
                    (urgency(top.state, top_seen), top.updated_at) >= (urgency(status.state, seen(*unseen)), status.updated_at)
                );
            }
        }
    }

    #[test]
    fn file_names_from_browsers_are_file_names(raw in text()) {
        let name = FileName::from_browser(&raw);
        let parsed = name.to_string().parse::<FileName>().ok();
        prop_assert_eq!(parsed.as_ref(), Some(&name), "{:?} from {:?}", name, raw);
    }

    #[test]
    fn file_names_that_fit_keep_their_extension(stem in "[a-z0-9][a-z0-9_-]{0,200}", extension in "[a-z0-9]{1,8}") {
        let raw = format!("{stem}.{extension}");
        let name = FileName::from_browser(&raw);
        let ext = format!(".{extension}");
        prop_assert!(name.to_string().ends_with(&ext), "{:?} lost {}", name, ext);
    }

    #[test]
    fn numbered_file_names_differ_from_the_first(name in "[a-z0-9_][a-z0-9._-]{0,60}", n in 2..10_000u32) {
        let name: FileName = name.parse().expect("the pattern is a file name");
        prop_assert_eq!(name.numbered(1), name.to_string());
        prop_assert_ne!(name.numbered(n), name.to_string());
        prop_assert!(name.numbered(n).contains(&format!("-{n}")), "{}", name.numbered(n));
    }

    #[test]
    fn downloads_are_named_only_in_a_downloads_directory(
        dir in "(/[a-z.]{1,8}){0,4}",
        last in "(Downloads|downloads|Download|x)",
        name in text().prop_map(|s| s.replace('/', "")),
    ) {
        let path = format!("{dir}/{last}/{name}");
        let named = DownloadName::of_download(&path);
        if last == "Downloads" {
            prop_assert_eq!(named, name.parse::<DownloadName>().ok());
        } else {
            prop_assert_eq!(named, None);
        }
    }

    #[test]
    fn downloads_open_inside_home(name in text(), home in "(/[a-z0-9]{1,8}){1,4}/?") {
        if let Ok(name) = name.parse::<DownloadName>() {
            let user: GuestUser = serde_json::from_value(serde_json::json!({
                "name": "dev", "uid": 1000, "gid": 100, "home": home
            })).expect("a user");
            let path = name.in_home(&user);
            prop_assert!(!path.starts_with('/'));
            prop_assert!(path.ends_with(&format!("Downloads/{name}")), "{}", path);
            prop_assert!(path.split('/').all(|part| part != ".." && part != "." && !part.is_empty()), "{path:?}");
        }
    }

    #[test]
    fn a_sign_in_finishes_only_with_its_own_state(cookie in prop::option::of(text()), state in text()) {
        prop_assert_eq!(same_browser(cookie.as_deref(), &state), cookie.as_deref() == Some(state.as_str()));
        prop_assert!(same_browser(Some(&state), &state));
    }

    /// Random strings rarely differ in more than one place by the same bits,
    /// which is where a comparison that lets differences cancel goes wrong.
    #[test]
    fn a_state_off_in_several_places_by_the_same_bits_is_refused(
        state in "[ -~]{2,64}", at in prop::collection::vec(any::<prop::sample::Index>(), 1..5), bits in 1u8..0x80,
    ) {
        let mut cookie = state.clone().into_bytes();
        let at: std::collections::BTreeSet<usize> = at.iter().map(|i| i.index(cookie.len())).collect();
        for &i in &at {
            cookie[i] ^= bits;
        }
        let cookie = String::from_utf8(cookie).expect("ASCII stays ASCII");
        prop_assert!(!same_browser(Some(&cookie), &state), "{cookie:?} passed for {state:?}");
    }

    #[test]
    fn preview_apps_never_set_a_cookie_wider_than_their_host(
        name in "[A-Za-z0-9_]{1,12}", value in "[A-Za-z0-9]{0,12}",
        before in prop::collection::vec(attribute(), 0..3), after in prop::collection::vec(attribute(), 0..3),
        domain in "(domain|Domain|DOMAIN|dOmAiN)", space in "[ \t]{0,2}", target in "(=[a-z.]{0,12})?",
    ) {
        let cookie = format!("{name}={value}{}; {space}{domain}{space}{target}{}", before.concat(), after.concat());
        prop_assert!(!preview_may_set_cookie(&cookie), "{cookie:?} got through");
    }

    #[test]
    fn preview_apps_never_set_the_gateways_cookie(
        case in prop::collection::vec(any::<bool>(), PREVIEW_COOKIE.len()),
        value in "[A-Za-z0-9]{0,12}", attributes in prop::collection::vec(attribute(), 0..3), space in "[ \t]{0,2}",
    ) {
        let name: String = PREVIEW_COOKIE.chars().zip(case).map(|(c, upper)| if upper { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }).collect();
        let cookie = format!("{space}{name}{space}={value}{}", attributes.concat());
        prop_assert!(!preview_may_set_cookie(&cookie), "{cookie:?} got through");
    }

    #[test]
    fn preview_apps_set_their_own_host_only_cookies(name in "[A-Za-z0-9_]{1,12}", value in "[A-Za-z0-9]{0,12}", attributes in prop::collection::vec(attribute(), 0..4)) {
        let cookie = format!("{name}={value}{}", attributes.concat());
        prop_assert!(preview_may_set_cookie(&cookie), "{cookie:?} was refused");
    }

    #[test]
    fn cookie_checks_never_panic(s in text()) {
        let _ = preview_may_set_cookie(&s);
    }

    #[test]
    fn admission_fits_exactly_when_reservation_and_headroom_do(available in any::<u64>(), reservation in any::<u64>(), headroom in any::<u64>()) {
        let (available, reservation, headroom) = (Bytes::new(available), Bytes::new(reservation), Bytes::new(headroom));
        let needed = reservation.saturating_add(headroom);
        match admit(available, reservation, headroom) {
            Capacity::Fits => prop_assert!(available >= needed),
            Capacity::Short { available: a, needed: n } => {
                prop_assert!(available < needed);
                prop_assert_eq!((a, n), (available, needed));
            }
        }
    }

    #[test]
    fn a_pass_admits_no_more_than_the_host_holds(available in 0..64u64, reservation in 1..8u64, headroom in 0..8u64) {
        let gib = Bytes::gib;
        let mut room = Room::new(gib(available), gib(reservation), gib(headroom));
        let mut admitted = 0;
        while room.capacity() == Capacity::Fits && admitted < 100 {
            room = room.charge();
            admitted += 1;
        }
        // Every admitted start keeps the headroom free.
        let most = available.saturating_sub(headroom) / reservation;
        prop_assert_eq!(admitted, most);
    }

    #[test]
    fn bytes_show_without_panicking(bytes in any::<u64>()) {
        let shown = Bytes::new(bytes).to_string();
        prop_assert!(shown.ends_with("MiB") || shown.ends_with("GiB"));
    }

    #[test]
    fn time_never_runs_backwards(at in any::<i64>(), earlier in any::<i64>(), secs in any::<u32>()) {
        let at = Timestamp::from_unix_millis(at);
        let earlier = Timestamp::from_unix_millis(earlier);
        prop_assert!(at.since(earlier).as_millis() >= 0);
        prop_assert!(at.plus(Millis::from_secs(secs)) >= at);
        if earlier <= at && at.unix_millis().checked_sub(earlier.unix_millis()).is_some() {
            prop_assert_eq!(at.since(earlier).as_millis(), at.unix_millis() - earlier.unix_millis());
        }
    }

    #[test]
    fn conflicts_are_symmetric_and_every_target_conflicts_with_itself(a in target(), b in target()) {
        prop_assert!(a.conflicts_with(&a));
        prop_assert_eq!(a.conflicts_with(&b), b.conflicts_with(&a));
    }

    #[test]
    fn a_bundle_refuses_exactly_what_conflicts(targets in prop::collection::vec(target(), 0..8), generation in any::<u64>()) {
        let value: SecretValue = SecretValue::try_from("v".to_owned()).expect("a value");
        let secrets: Vec<(SecretTarget, SecretValue)> = targets.iter().cloned().map(|t| (t, value.clone())).collect();
        let conflict = targets.iter().enumerate().any(|(i, a)| targets[i + 1..].iter().any(|b| a.conflicts_with(b)));
        let bundled = bundle(SecretsGeneration::from_u64(generation), secrets);
        prop_assert_eq!(bundled.is_err(), conflict, "{:?}", bundled.as_ref().err());
        if let Ok(bundled) = bundled {
            prop_assert_eq!(bundled.env.len() + bundled.files.len() + bundled.git.len(), targets.len());
            prop_assert_eq!(bundled.generation, SecretsGeneration::from_u64(generation));
        }
    }
}

/// Terminal history: lines of text, with blank lines and trailing space,
/// sometimes longer than what's sent back.
fn history() -> impl Strategy<Value = String> {
    let line = prop_oneof![
        4 => "[a-z é漢]{0,40}",
        1 => Just(String::new()),
        1 => "[a-z]{0,10}".prop_map(|s| s.repeat(500)),
    ];
    let lines = (prop::collection::vec(line, 0..300), "[ \n]{0,5}")
        .prop_map(|(lines, end)| lines.join("\n") + &end);
    // One line as long as what's sent back, or either side of it.
    let at_the_limit =
        (OUTPUT_MAX_BYTES - 1..=OUTPUT_MAX_BYTES + 1).prop_map(|length| "x".repeat(length));
    prop_oneof![9 => lines, 1 => at_the_limit]
}

fn state() -> impl Strategy<Value = AttentionState> {
    prop::sample::select(
        &[
            AttentionState::Working,
            AttentionState::Waiting,
            AttentionState::Done,
            AttentionState::Idle,
        ][..],
    )
}

fn status() -> impl Strategy<Value = SessionStatus> {
    ("[a-z][a-z0-9]{0,8}", "[a-z0-9]{1,8}", state(), any::<i64>()).prop_map(
        |(session, thread, state, at)| SessionStatus {
            session: session.parse().expect("a session name"),
            thread: thread.parse::<ThreadKey>().expect("a thread key"),
            title: Summary::sanitize(""),
            state,
            summary: Summary::sanitize(""),
            updated_at: Timestamp::from_unix_millis(at),
        },
    )
}

fn listener() -> impl Strategy<Value = Listener> {
    (
        1..20u16,
        "[a-z]{0,8}",
        prop::option::of("[a-z][a-z0-9]{0,8}"),
        any::<bool>(),
    )
        .prop_map(|(port, process, column, reachable)| Listener {
            port: GuestPort::try_from(port).expect("nonzero"),
            process: ProcessName::sanitize(&process),
            column: column.map(|c| c.parse::<SessionName>().expect("a session name")),
            reachable,
        })
}

fn attribute() -> impl Strategy<Value = String> {
    prop::sample::select(
        &[
            "; Path=/",
            "; Secure",
            "; HttpOnly",
            "; SameSite=Lax",
            "; Max-Age=60",
            "; path=/x; secure",
            ";Expires=Wed, 21 Oct 2026 07:28:00 GMT",
        ][..],
    )
    .prop_map(str::to_owned)
}

fn target() -> impl Strategy<Value = SecretTarget> {
    prop_oneof![
        prop::sample::select(&["TOKEN", "API_KEY", "SECRET"][..]).prop_map(|name| {
            SecretTarget::Env {
                name: name.parse::<EnvVarName>().expect("a variable"),
            }
        }),
        prop::collection::vec(prop::sample::select(&["a", "b", "c"][..]), 1..4).prop_map(|parts| {
            SecretTarget::File {
                path: parts.join("/").parse::<HomePath>().expect("a home path"),
            }
        }),
        (
            prop::sample::select(&["github.com", "gitlab.com", "GitHub.com"][..]),
            prop::sample::select(&["git", "x-access-token"][..])
        )
            .prop_map(|(host, username)| SecretTarget::GitCredential {
                host: host.parse::<GitHost>().expect("a host"),
                username: username.parse::<GitUsername>().expect("a username"),
            }),
    ]
}
