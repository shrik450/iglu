//! Properties of the guest tools' pure parts, checked on generated inputs:
//! what they read from the system and from programs never makes them
//! panic, and what they send the terminal can't break out of its frame.
//!
//! Each property runs `PROPTEST_CASES` cases, 256 by default. Failures are
//! saved under `proptest-regressions/`, which is committed.

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use base64::Engine;
use iglu_guest::clipboard::{self, Asked, osc52, trimmed};
use iglu_guest::listeners::{parse_lsof, parse_net_tcp, parse_ps, placement, session_of};
use iglu_guest::open::{self, MAX_BYTES, Target, file_address, osc};
use iglu_guest::{claude, git, session, tty};
use proptest::prelude::*;

/// Any text, weighted toward separators, controls and multi-byte
/// characters at the start of a line.
fn text() -> impl Strategy<Value = String> {
    let tricky = prop::sample::select(
        &[
            "a",
            "Z",
            "0",
            " ",
            "\t",
            "\n",
            "\r",
            "\0",
            "\u{1b}",
            "\u{7}",
            ":",
            "=",
            "/",
            "\\",
            "%",
            "[",
            "]",
            "(",
            ")",
            "é",
            "漢",
            "🧊",
            "p",
            "c",
            "t",
            "n",
            "IPv6",
            "ZMX_SESSION=",
            "zmx",
            "attach",
            "0A",
            "*",
        ][..],
    );
    prop_oneof![
        any::<String>(),
        prop::collection::vec(tricky, 0..60).prop_map(|parts| parts.concat()),
    ]
}

fn args() -> impl Strategy<Value = Vec<String>> {
    let arg = prop_oneof![
        prop::sample::select(
            &[
                "-i",
                "-in",
                "-o",
                "-out",
                "-selection",
                "-sel",
                "clipboard",
                "primary",
                "-t",
                "-target",
                "text/plain",
                "image/png",
                "-rmlastnl",
                "-f",
                "-filter",
                "-h",
                "-help",
                "-version",
                "-b",
                "--clipboard",
                "-p",
                "-i",
                "--input",
                "-o",
                "--output",
                "-c",
                "--clear",
                "-k",
                "--keep",
                "-x",
                "--exchange",
                "-a",
                "--append",
                "-l",
                "--logfile",
                "-",
                "--",
                "file.txt",
                "",
            ][..],
        )
        .prop_map(str::to_owned),
        text(),
    ];
    prop::collection::vec(arg, 0..6)
}

/// A local address as the kernel writes it in `/proc/net/tcp`: each
/// 32-bit word in host order.
fn kernel_hex(address: IpAddr) -> String {
    let bytes: Vec<u8> = match address {
        IpAddr::V4(v4) => v4.octets().to_vec(),
        IpAddr::V6(v6) => v6.octets().to_vec(),
    };
    bytes.chunks(4).fold(String::new(), |mut hex, word| {
        let word = u32::from_le_bytes(word.try_into().expect("a word"));
        let _ = write!(hex, "{word:08X}");
        hex
    })
}

proptest! {
    #[test]
    fn listening_sockets_read_back_as_the_kernel_wrote_them(
        v4 in any::<[u8; 4]>(), v6 in any::<[u8; 16]>(), six in any::<bool>(),
        port in any::<u16>(), uid in any::<u32>(), inode in any::<u64>(), listening in any::<bool>(),
    ) {
        let address = if six { IpAddr::V6(Ipv6Addr::from(v6)) } else { IpAddr::V4(Ipv4Addr::from(v4)) };
        let state = if listening { "0A" } else { "01" };
        let table = format!(
            "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: {}:{port:04X} 00000000:0000 {state} 00000000:00000000 00:00000000 00000000  {uid}        0 {inode} 1 0000000000000000 100 0 0 10 0\n",
            kernel_hex(address)
        );
        let sockets = parse_net_tcp(&table);
        if listening {
            prop_assert_eq!(sockets.len(), 1);
            prop_assert_eq!(sockets[0].address, address);
            prop_assert_eq!(sockets[0].port, port);
            prop_assert_eq!(sockets[0].uid, uid);
            prop_assert_eq!(sockets[0].inode, inode);
        } else {
            prop_assert!(sockets.is_empty());
        }
    }

    #[test]
    fn system_and_tool_output_never_panics(s in text(), pid in any::<u32>()) {
        let _ = parse_net_tcp(&s);
        let _ = parse_lsof(&s);
        let _ = placement(&parse_ps(&s), pid);
        let _ = session_of(s.as_bytes());
        let _ = session::parse_list(&s);
        let _ = tty::parent_of(&s);
        let _ = git::parse(&s);
        let _ = claude::thread(&s);
        if let Some(event) = claude::parse(&s) {
            let _ = claude::title(&event);
            let _ = claude::attention(&event);
        }
    }

    #[test]
    fn a_session_is_named_by_its_environment(before in prop::collection::vec("[A-Z_]{1,8}=[a-z]{0,8}", 0..4), name in "[a-z0-9-]{0,16}") {
        let mut environ: Vec<u8> = before.join("\0").into_bytes();
        if !environ.is_empty() {
            environ.push(0);
        }
        environ.extend(format!("ZMX_SESSION={name}\0").bytes());
        prop_assert_eq!(session_of(&environ), Some(name));
    }

    #[test]
    fn a_process_s_placement_comes_from_it_or_its_ancestors(
        parents in prop::collection::vec(0..12u32, 1..12),
        sessions in prop::collection::vec(prop::option::of("[a-z]{1,6}"), 12),
        pid in 0..12u32,
    ) {
        // A table where each process may name a session, with loops allowed.
        let ps: String = parents
            .iter()
            .enumerate()
            .map(|(pid, parent)| match &sessions[pid] {
                Some(session) => format!("{pid} {parent} bash ZMX_SESSION={session}\n"),
                None => format!("{pid} {parent} bash\n"),
            })
            .collect();
        let found = placement(&parse_ps(&ps), pid).session;
        // The nearest of it and its ancestors that names a session, following
        // parents until one repeats.
        let mut current = pid;
        let mut seen = std::collections::HashSet::new();
        let mut expected = None;
        while let Some(&parent) = parents.get(current as usize) {
            if let Some(session) = &sessions[current as usize] {
                expected = Some(session.clone());
                break;
            }
            if !seen.insert(current) || parent == current {
                break;
            }
            current = parent;
        }
        prop_assert_eq!(found, expected);
    }

    #[test]
    fn web_addresses_reach_the_console_as_printable_ascii(arg in text()) {
        if let Ok(Target::Address(address)) = open::target(&arg) {
            prop_assert!(address.bytes().all(|b| b.is_ascii_graphic()), "{:?}", address);
            prop_assert!(address.len() <= MAX_BYTES);
            // An address is already as it's sent.
            prop_assert_eq!(open::target(&address), Ok(Target::Address(address.clone())));
        }
    }

    #[test]
    fn what_opens_is_framed_once(address in "[!-~]{0,200}") {
        let sent = osc(&address);
        prop_assert!(sent.starts_with(b"\x1b]4458;"));
        prop_assert!(sent.ends_with(b"\x1b\\"));
        // The frame's own escapes are its only ones.
        prop_assert_eq!(String::from_utf8_lossy(&sent).matches('\u{1b}').count(), 2);
        prop_assert!(!sent.contains(&0x07));
    }

    #[test]
    fn file_addresses_name_their_file(parts in prop::collection::vec(prop_oneof!["[a-zA-Z0-9._~-]{1,10}", "[ %#?&;:@é漢\\[\\]]{1,4}"], 1..6)) {
        // URLs drop dot segments, as browsers do; real paths don't have them.
        let parts: Vec<String> = parts.into_iter().map(|part| if part == "." || part == ".." { "dot".to_owned() } else { part }).collect();
        let path = format!("/{}", parts.join("/"));
        let address = file_address(Path::new(&path));
        prop_assert!(address.is_ok());
        let address = address.expect("checked just now");
        prop_assert!(address.bytes().all(|b| b.is_ascii_graphic()), "{:?}", address);
        let url = url::Url::parse(&address);
        prop_assert!(url.is_ok(), "{address:?} isn't a URL");
        let read = url.expect("checked just now").to_file_path();
        prop_assert_eq!(read.ok(), Some(std::path::PathBuf::from(&path)));
    }

    #[test]
    fn copies_reach_the_clipboard_whole(text in prop::collection::vec(any::<u8>(), 0..2000)) {
        let sent = osc52(&text);
        prop_assert!(sent.starts_with(b"\x1b]52;c;"));
        prop_assert!(sent.ends_with(b"\x1b\\"));
        let payload = &sent[7..sent.len() - 2];
        prop_assert!(payload.iter().all(u8::is_ascii_graphic));
        let decoded = base64::engine::general_purpose::STANDARD.decode(payload);
        prop_assert_eq!(decoded.ok(), Some(text));
    }

    #[test]
    fn trimming_drops_only_one_final_newline(text in prop::collection::vec(prop_oneof![Just(b'\n'), Just(b'a'), Just(b'\r')], 0..10), trim in any::<bool>()) {
        let kept = trimmed(&text, trim);
        if trim && text.last() == Some(&b'\n') {
            prop_assert_eq!(kept, &text[..text.len() - 1]);
        } else {
            prop_assert_eq!(kept, &text[..]);
        }
    }

    #[test]
    fn clipboard_tools_take_any_arguments(args in args(), terminal in any::<bool>()) {
        for asked in [clipboard::xclip(&args), clipboard::xsel(&args, terminal)] {
            if let Ok(Asked::Copy { files, .. }) = asked {
                // Only what was named is read.
                for file in files {
                    prop_assert!(args.iter().any(|a| Path::new(a) == file));
                }
            }
        }
    }

    #[test]
    fn porcelain_counts_what_it_lists(changed in 0..20u32, untracked in 0..20u32, conflicted in 0..20u32, ahead in any::<u32>(), behind in any::<u32>()) {
        let mut out = format!("# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +{ahead} -{behind}\n");
        out.extend((0..changed).map(|i| format!("1 .M N... 100644 100644 100644 a b f{i}\n")));
        out.extend((0..untracked).map(|i| format!("? new{i}\n")));
        out.extend((0..conflicted).map(|i| format!("u UU N... 1 2 3 4 a b c c{i}\n")));
        let report = git::parse(&out);
        prop_assert_eq!(report.branch.as_deref(), Some("main"));
        prop_assert_eq!(report.upstream.as_deref(), Some("origin/main"));
        prop_assert_eq!((report.changed, report.untracked, report.conflicted), (changed, untracked, conflicted));
        prop_assert_eq!((report.ahead, report.behind), (ahead, behind));
    }

    #[test]
    fn zmx_lists_every_session_it_names(sessions in prop::collection::vec(("[a-z0-9][a-z0-9-]{0,15}", 0..5u32), 0..8)) {
        let listing = sessions.iter().fold(String::new(), |mut listing, (name, clients)| {
            let _ = writeln!(listing, "session_name={name}\tpid=1\tclients={clients}\tcreated_at=0");
            listing
        });
        let listed = session::parse_list(&listing);
        let read: Vec<(String, u32)> = listed.iter().map(|info| (info.name.to_string(), info.clients)).collect();
        prop_assert_eq!(read, sessions);
    }

    #[test]
    fn a_process_s_parent_reads_through_any_name(pid in any::<u32>(), name in "[ -~]{0,20}", parent in any::<u32>()) {
        let stat = format!("{pid} ({name}) S {parent} 1 1 0 -1");
        prop_assert_eq!(tty::parent_of(&stat), Some(parent));
    }
}

#[test]
fn lsof_lines_that_start_with_another_character_are_skipped() {
    assert_eq!(parse_lsof("⭶\np1\ncnode\ntIPv4\nn*:3000\n").len(), 1);
}
