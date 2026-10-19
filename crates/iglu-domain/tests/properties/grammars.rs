//! Each parser against its grammar, as its documentation states it,
//! written independently as a regular expression and a length: a parser
//! must accept everything the grammar allows and nothing else. Inputs
//! crowd the length limits, where an off-by-one hides.

use iglu_domain::column::{Arg, Argv};
use iglu_domain::env::{FlakeAttr, ImageFingerprint, UnixUser};
use iglu_domain::label::DnsLabel;
use iglu_domain::pasted::FileName;
use iglu_domain::preferences::{KeyCode, PastedTheme, ThemeName};
use iglu_domain::repo::{CheckoutDir, GitHost};
use iglu_domain::secret::{EnvVarName, GitUsername, HomePath, SecretName, SecretValue};
use iglu_domain::terminal::{SessionName, TerminalInput, TerminalSize};
use iglu_domain::{attention::ThreadKey, download::DownloadName};
use proptest::prelude::*;
use regex::Regex;

/// Text from `alphabet`, as long as the limit or either side of it, or short.
fn near(alphabet: &'static [char], limit: usize) -> impl Strategy<Value = String> {
    prop_oneof![0..6usize, limit.saturating_sub(2)..=limit + 2].prop_flat_map(move |length| {
        prop::collection::vec(prop::sample::select(alphabet), length).prop_map(String::from_iter)
    })
}

const LOWER: &[char] = &[
    'a', 'z', '0', '9', '-', '_', '.', 'A', 'Z', ' ', '/', ':', '\'', 'é',
];
const UPPER: &[char] = &['A', 'Z', '0', '9', '_', '-', 'a', ' '];
const HEX: &[char] = &['0', '9', 'a', 'f', 'g', 'A', 'F'];
const PRINTABLE: &[char] = &[
    'a', ' ', '-', 'é', '漢', '\t', '\n', '\u{1b}', '\u{202e}', '\0',
];

fn matches(pattern: &str, s: &str) -> bool {
    Regex::new(pattern).expect("a valid pattern").is_match(s)
}

proptest! {
    #[test]
    fn dns_labels(s in near(LOWER, 63)) {
        prop_assert_eq!(s.parse::<DnsLabel>().is_ok(), matches(r"^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$", &s), "{:?}", s);
    }

    #[test]
    fn session_names(s in near(LOWER, 32)) {
        prop_assert_eq!(s.parse::<SessionName>().is_ok(), matches(r"^[a-z0-9][a-z0-9-]{0,31}$", &s), "{:?}", s);
    }

    #[test]
    fn secret_names(s in near(LOWER, 64)) {
        prop_assert_eq!(s.parse::<SecretName>().is_ok(), matches(r"^[a-z0-9][a-z0-9._-]{0,63}$", &s), "{:?}", s);
    }

    #[test]
    fn env_var_names(s in near(UPPER, 128)) {
        let reserved = ["HOME", "PATH", "USER", "LOGNAME", "SHELL", "TERM", "XDG_RUNTIME_DIR", "ZMX_DIR"].contains(&s.as_str())
            || s.starts_with("IGLU_")
            || s.starts_with("ZMX_");
        prop_assert_eq!(s.parse::<EnvVarName>().is_ok(), matches(r"^[A-Z_][A-Z0-9_]{0,127}$", &s) && !reserved, "{:?}", s);
    }

    #[test]
    fn thread_keys(s in near(LOWER, 64)) {
        prop_assert_eq!(s.parse::<ThreadKey>().is_ok(), matches(r"^[A-Za-z0-9_-]{1,64}$", &s), "{:?}", s);
    }

    #[test]
    fn key_codes(s in near(LOWER, 32)) {
        prop_assert_eq!(s.parse::<KeyCode>().is_ok(), matches(r"^[A-Za-z0-9]{1,32}$", &s), "{:?}", s);
    }

    #[test]
    fn image_fingerprints(s in near(HEX, 64)) {
        prop_assert_eq!(s.parse::<ImageFingerprint>().is_ok(), matches(r"^[0-9a-f]{64}$", &s), "{:?}", s);
    }

    #[test]
    fn unix_users(s in prop_oneof![near(LOWER, 32), Just("root".to_owned())]) {
        prop_assert_eq!(s.parse::<UnixUser>().is_ok(), matches(r"^[a-z_][a-z0-9_-]{0,31}$", &s) && s != "root", "{:?}", s);
    }

    #[test]
    fn flake_attrs(s in near(LOWER, 128)) {
        prop_assert_eq!(s.parse::<FlakeAttr>().is_ok(), matches(r"^[A-Za-z_][A-Za-z0-9_'-]{0,127}$", &s), "{:?}", s);
    }

    #[test]
    fn checkout_dirs(s in prop_oneof![near(LOWER, 100), Just(".".to_owned()), Just("..".to_owned())]) {
        let grammar = matches(r"^[A-Za-z0-9_.][A-Za-z0-9_.-]{0,99}$", &s) && s != "." && s != "..";
        prop_assert_eq!(s.parse::<CheckoutDir>().is_ok(), grammar, "{:?}", s);
    }

    #[test]
    fn file_names(s in near(LOWER, 80)) {
        prop_assert_eq!(s.parse::<FileName>().is_ok(), matches(r"^[A-Za-z0-9_][A-Za-z0-9._-]{0,79}$", &s), "{:?}", s);
    }

    #[test]
    fn git_usernames(s in near(&['a', 'Z', '@', '%', ':', ' ', '\t', '\u{1}', 'é', '\u{3000}'], 128)) {
        let grammar = (1..=128).contains(&s.len()) && !s.chars().any(|c| c.is_whitespace() || c.is_control() || c == ':');
        prop_assert_eq!(s.parse::<GitUsername>().is_ok(), grammar, "{:?}", s);
    }

    #[test]
    fn git_hosts(labels in prop::collection::vec(near(&['a', 'Z', '0', '-', '_'], 63), 1..4), port in prop::option::of(prop_oneof![Just("0".to_owned()), Just("65535".to_owned()), Just("65536".to_owned()), Just(String::new()), "[0-9]{1,5}"])) {
        let host = labels.join(".");
        let text = port.as_ref().map_or_else(|| host.clone(), |port| format!("{host}:{port}"));
        let label = r"[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?";
        let grammar = matches(&format!(r"^{label}(\.{label})*$"), &host)
            && host.len() <= 253
            && port.as_ref().is_none_or(|port| port.parse::<u16>().is_ok_and(|p| p != 0));
        prop_assert_eq!(text.parse::<GitHost>().is_ok(), grammar, "{:?}", text);
    }

    #[test]
    fn home_paths(parts in prop::collection::vec(prop_oneof![near(LOWER, 255), Just(".".to_owned()), Just("..".to_owned())], 1..5), lead in any::<bool>()) {
        let path = format!("{}{}", if lead { "/" } else { "" }, parts.join("/"));
        let grammar = !lead
            && path.len() <= 1024
            && path.split('/').all(|part| !part.is_empty() && part != "." && part != ".." && part.len() <= 255);
        prop_assert_eq!(path.parse::<HomePath>().is_ok(), grammar, "{:?}", path);
    }

    #[test]
    fn download_names(s in prop_oneof![near(PRINTABLE, 255), near(&['a', '/', '.'], 4)]) {
        let printable = !s.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'));
        let grammar = !s.is_empty() && s != "." && s != ".." && s.len() <= 255 && !s.contains('/') && printable;
        prop_assert_eq!(s.parse::<DownloadName>().is_ok(), grammar, "{:?}", s);
    }

    #[test]
    fn theme_names(s in near(PRINTABLE, 64)) {
        let printable = !s.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'));
        prop_assert_eq!(s.parse::<ThemeName>().is_ok(), (1..=64).contains(&s.chars().count()) && printable, "{:?}", s);
    }

    #[test]
    fn byte_limits_are_exact(extra in -1i64..=1, kind in 0..5u8, odd in prop::sample::select(&['a', '\0', 'é'][..])) {
        // Each limit and either side of it, with one character that may be
        // the one each type refuses.
        let limit = match kind {
            0 | 1 => 64 * 1024,
            2 | 4 => 16 * 1024,
            _ => 255,
        };
        let length = usize::try_from(i64::try_from(limit).expect("small") + extra).expect("positive");
        let mut s = "a".repeat(length.saturating_sub(odd.len_utf8()));
        s.push(odd);
        let fits = s.len() <= limit;
        match kind {
            0 => prop_assert_eq!(s.parse::<TerminalInput>().is_ok(), fits && odd != '\0'),
            1 => prop_assert_eq!(SecretValue::try_from(s.clone()).is_ok(), fits),
            2 => prop_assert_eq!(s.parse::<Arg>().is_ok(), fits && odd != '\0'),
            3 => prop_assert_eq!(s.parse::<DownloadName>().is_ok(), fits && odd != '\0'),
            _ => prop_assert_eq!(s.parse::<PastedTheme>().is_ok(), fits && odd != '\0'),
        }
    }

    #[test]
    fn a_command_has_a_program_and_at_most_64_arguments(count in 0..70usize, empty_program in any::<bool>()) {
        let mut args: Vec<Arg> = (0..count).map(|_| "x".parse().expect("an argument")).collect();
        if empty_program && !args.is_empty() {
            args[0] = "".parse().expect("an empty argument");
        }
        let grammar = (1..=64).contains(&count) && !empty_program;
        let argv = Argv::try_from(args.clone());
        prop_assert_eq!(argv.is_ok(), grammar);
        if let Ok(argv) = argv {
            prop_assert_eq!(Vec::<Arg>::from(argv), args);
        }
    }

    #[test]
    fn terminal_sizes_are_what_they_say(cols in prop_oneof![0..1100u16, any::<u16>()], rows in prop_oneof![0..1100u16, any::<u16>()]) {
        let size = TerminalSize::new(cols, rows);
        prop_assert_eq!(size.is_ok(), (2..=1000).contains(&cols) && (1..=1000).contains(&rows));
        if let Ok(size) = size {
            prop_assert_eq!((size.cols(), size.rows()), (cols, rows));
        }
    }
}
