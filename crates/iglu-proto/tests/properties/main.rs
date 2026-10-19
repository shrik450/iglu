//! The protocol's paths carry domain values to hostd's router: each value
//! must read back from the URL exactly, never as another path or query.
//!
//! Each property runs `PROPTEST_CASES` cases, 256 by default. Failures are
//! saved under `proptest-regressions/`, which is committed.

use iglu_domain::id::WorkspaceId;
use iglu_domain::pasted::FileName;
use iglu_domain::port::GuestPort;
use iglu_domain::terminal::{OutputLines, SessionName};
use iglu_proto::path;
use proptest::prelude::*;
use uuid::Uuid;

fn read(path: &str) -> url::Url {
    url::Url::parse("https://host.example")
        .and_then(|base| base.join(path))
        .expect("a path joins")
}

fn segments(url: &url::Url) -> Vec<String> {
    url.path_segments()
        .expect("a path")
        .map(str::to_owned)
        .collect()
}

fn query(url: &url::Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

proptest! {
    #[test]
    fn terminal_paths_read_back_their_values(
        id in any::<u128>(),
        session in "[a-z0-9][a-z0-9-]{0,31}",
        lines in 1..=OutputLines::MOST,
        port in 1..=u16::MAX,
        name in "[A-Za-z0-9_][A-Za-z0-9._-]{0,79}",
    ) {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(id));
        let session: SessionName = session.parse().expect("the pattern is a session name");
        let lines = OutputLines::try_from(lines).expect("in range");
        let port = GuestPort::try_from(port).expect("nonzero");
        let name: FileName = name.parse().expect("the pattern is a file name");
        let ws = workspace.to_string();

        for (made, last) in [
            (path::commands(workspace), "commands"),
            (path::git(workspace), "git"),
            (path::listeners(workspace), "listeners"),
            (path::terminals(workspace), "terminals"),
        ] {
            let url = read(&made);
            prop_assert_eq!(segments(&url), ["v1", "workspaces", &ws, last]);
            prop_assert_eq!(url.query(), None);
        }
        let terminal = read(&path::terminal(workspace, &session));
        prop_assert_eq!(segments(&terminal), ["v1", "workspaces", &ws, "terminals", session.as_str()]);
        let input = read(&path::input(workspace, &session));
        prop_assert_eq!(segments(&input), ["v1", "workspaces", &ws, "terminals", session.as_str(), "input"]);

        let attach = read(&path::attach(workspace, &session));
        prop_assert_eq!(segments(&attach), ["v1", "workspaces", &ws, "terminals", session.as_str(), "attach"]);
        prop_assert_eq!(attach.query(), None);

        let output = read(&path::output(workspace, &session, lines));
        prop_assert_eq!(segments(&output), ["v1", "workspaces", &ws, "terminals", session.as_str(), "output"]);
        prop_assert_eq!(query(&output, "lines"), Some(lines.to_string()));

        let tunnel = read(&path::tunnel(workspace, port));
        prop_assert_eq!(segments(&tunnel), ["v1", "workspaces", &ws, "ports", &port.to_string(), "tunnel"]);

        let files = read(&path::files(workspace, &name));
        prop_assert_eq!(segments(&files), ["v1", "workspaces", &ws, "files"]);
        prop_assert_eq!(query(&files, "name"), Some(name.to_string()));
        prop_assert_eq!(files.query_pairs().count(), 1);
    }
}
