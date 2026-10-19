//! What hostd makes of a guest's files and output, which are untrusted:
//! nothing a guest writes makes hostd panic, and what's kept from it is
//! bounded and well-formed.
//!
//! Each property runs `PROPTEST_CASES` cases, 256 by default. Failures are
//! saved under `proptest-regressions/`, which is committed.

use std::collections::{BTreeMap, BTreeSet};

use iglu_domain::attention::MAX_THREADS;
use iglu_domain::guest::StatusEntry;
use iglu_domain::lifecycle::SecretsGeneration;
use iglu_domain::port::GuestPort;
use iglu_hostd::guest::{
    parse_generation, parse_git_state, parse_listeners, parse_sessions, parse_statuses,
};
use proptest::prelude::*;
use serde_json::{Value, json};

fn bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..200),
        "[ -~\\n\\t{}\\[\\]\":,0-9a-z]{0,200}".prop_map(String::into_bytes),
    ]
}

/// A JSON value of any shape, a few levels deep.
fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<f64>()
            .prop_map(|f| serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)),
        "[a-z0-9 \\-]{0,12}".prop_map(Value::from),
        prop::sample::select(&["waiting", "working", "done", "idle", "session", "t1"][..])
            .prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 24, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::from),
            prop::collection::btree_map("[a-z0-9.\\-]{0,8}", inner, 0..6)
                .prop_map(|map| Value::Object(map.into_iter().collect())),
        ]
    })
}

/// A status file as a guest might write it: sessions of threads, some
/// well-formed and some not.
fn status_file() -> impl Strategy<Value = Value> {
    let entry = prop_oneof![
        3 => (prop::sample::select(&["waiting", "working", "done", "idle"][..]), "[ -~]{0,30}", any::<i64>())
            .prop_map(|(state, summary, at)| json!({"state": state, "summary": summary, "title": "", "at": at})),
        1 => json_value(),
    ];
    let threads = prop::collection::btree_map("[a-zA-Z0-9_./-]{0,10}", entry, 0..24)
        .prop_map(|threads| Value::Object(threads.into_iter().collect()));
    prop::collection::btree_map(
        "[a-z0-9./-]{0,10}",
        prop_oneof![4 => threads, 1 => json_value()],
        0..8,
    )
    .prop_map(|sessions| Value::Object(sessions.into_iter().collect()))
}

proptest! {
    #[test]
    fn nothing_a_guest_writes_makes_hostd_panic(raw in bytes()) {
        let _ = parse_generation(&raw);
        let _ = parse_statuses(&raw);
        let text = String::from_utf8_lossy(&raw);
        let _ = parse_listeners(&text);
        let _ = parse_git_state(&text);
        let _ = parse_sessions(&text);
    }

    #[test]
    fn a_generation_reads_back_whatever_space_is_around_it(n in any::<u64>(), before in "[ \\t\\n]{0,3}", after in "[ \\t\\n]{0,3}") {
        let file = format!("{before}{n}{after}");
        prop_assert_eq!(parse_generation(file.as_bytes()).map(SecretsGeneration::get), Some(n));
    }

    #[test]
    fn statuses_keep_only_well_formed_threads_and_few_of_them(file in status_file()) {
        let raw = serde_json::to_vec(&file).expect("a value serializes");
        let statuses = parse_statuses(&raw);
        prop_assert!(statuses.len() <= 64);
        let mut per_session: BTreeMap<String, usize> = BTreeMap::new();
        for status in &statuses {
            *per_session.entry(status.session.to_string()).or_default() += 1;
            // Every status kept was written, under its session and thread.
            let written = &file[status.session.as_str()][status.thread.as_str()];
            let entry: Option<StatusEntry> = serde_json::from_value(written.clone()).ok();
            prop_assert!(entry.is_some(), "kept from {}", written);
            let entry = entry.expect("checked just now");
            prop_assert_eq!(entry.at, status.updated_at.unix_millis());
            prop_assert_eq!(entry.state, status.state);
            prop_assert_eq!(&entry.summary, &status.summary);
        }
        for (session, count) in per_session {
            prop_assert!(count <= MAX_THREADS, "{} keeps {}", session, count);
        }
    }

    #[test]
    fn listeners_are_one_per_port_and_few(
        listeners in prop::collection::vec((any::<u16>(), prop::sample::select(&["127.0.0.1", "0.0.0.0", "::", "::1", "10.0.0.2"][..]), "[ -~\\u{1b}]{0,40}", prop::option::of("[ -~]{0,12}")), 0..100),
    ) {
        let report: Vec<Value> = listeners
            .iter()
            .map(|(port, address, process, session)| json!({"port": port, "address": address, "process": process, "session": session}))
            .collect();
        let parsed = parse_listeners(&serde_json::to_string(&report).expect("a value serializes"));
        prop_assert!(parsed.len() <= 64);
        let ports: Vec<GuestPort> = parsed.iter().map(|l| l.port).collect();
        let distinct: BTreeSet<GuestPort> = ports.iter().copied().collect();
        prop_assert_eq!(ports.len(), distinct.len());
        for listener in &parsed {
            prop_assert!(listeners.iter().any(|(port, ..)| *port == listener.port.get()));
            prop_assert!(!listener.process.as_str().chars().any(char::is_control));
        }
    }
}
