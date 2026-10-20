//! Generated `adjective-verbing` names for workspaces and routes, names
//! taken from what a workspace was started to do, and what a person calls a
//! workspace.
//!
//! The shell supplies entropy and checks uniqueness against storage; this
//! module only turns entropy and text into a well-formed name.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::label::DnsLabel;
use crate::parse::{is_printable, text_type};

/// What a person calls a workspace, shown in place of its name: 1–80
/// characters of text a line can show, with no space at either end. A label
/// only names; the workspace's name stays its identity, in URLs, the CLI and
/// its branch.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct WorkspaceLabel(String);

impl WorkspaceLabel {
    pub const MAX_CHARS: usize = 80;

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for WorkspaceLabel {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.trim() != s {
            return Err(ParseError::new(
                "workspace label",
                "must have text and no space at either end",
            ));
        }
        if s.chars().count() > Self::MAX_CHARS {
            return Err(ParseError::new(
                "workspace label",
                "must be at most 80 characters",
            ));
        }
        if !s.chars().all(is_printable) {
            return Err(ParseError::new(
                "workspace label",
                "must be one line of plain text",
            ));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for WorkspaceLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(WorkspaceLabel);

/// The longest name made from text, leaving room for a number after it.
const MADE_MAX: usize = 40;

/// A name made from what a person typed, such as `more-ui-tweaks` for "More
/// UI tweaks": ASCII letters and digits, lowercased, with dashes for
/// everything between them, starting at the first letter. Attempts after
/// the first add `-2`, `-3` and so on. `None` when the text has no letter.
#[must_use]
pub fn from_label(text: &str, attempt: u32) -> Option<DnsLabel> {
    let mut stem = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if stem.is_empty() && !c.is_ascii_alphabetic() {
                continue;
            }
            stem.push(c.to_ascii_lowercase());
        } else if !stem.is_empty() && !stem.ends_with('-') {
            stem.push('-');
        }
    }
    stem.truncate(MADE_MAX);
    let stem = stem.trim_end_matches('-');
    if stem.is_empty() {
        return None;
    }
    let text = if attempt == 0 {
        stem.to_owned()
    } else {
        format!("{stem}-{}", u64::from(attempt) + 1)
    };
    text.parse().ok()
}

/// A name made from a branch's last part, such as `login-fix` for
/// `su/login-fix`.
#[must_use]
pub fn from_branch(branch: &str, attempt: u32) -> Option<DnsLabel> {
    from_label(branch.rsplit('/').next().unwrap_or(branch), attempt)
}

/// Attempts before candidates get a numeric suffix, so the name space never runs out.
const BARE_ATTEMPTS: u32 = 8;

/// A candidate name for `entropy` on the given attempt. Attempts from
/// [`BARE_ATTEMPTS`] onward append `-<n>`.
///
/// # Panics
///
/// Never: the word lists are lowercase ASCII, so every candidate is a label.
#[must_use]
pub fn candidate(entropy: u64, attempt: u32) -> DnsLabel {
    let mixed = splitmix64(entropy ^ u64::from(attempt).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let adjective = pick(ADJECTIVES, mixed);
    let verb = pick(VERBS, mixed.rotate_right(32));
    let text = if attempt < BARE_ATTEMPTS {
        format!("{adjective}-{verb}")
    } else {
        format!("{adjective}-{verb}-{}", mixed.rotate_right(16) % 10_000)
    };
    text.parse()
        .expect("word lists are lowercase ASCII, so every candidate is a DNS label")
}

fn pick(words: &'static [&'static str], value: u64) -> &'static str {
    let len = u64::try_from(words.len()).expect("word lists are small");
    let index = usize::try_from(value % len).expect("index is below the list length");
    words[index]
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

const ADJECTIVES: &[&str] = &[
    "able", "agile", "amber", "ample", "azure", "balmy", "bold", "brassy", "brave", "breezy",
    "bright", "brisk", "calm", "candid", "cheery", "civic", "clear", "clever", "cosmic", "cozy",
    "crisp", "curly", "dapper", "daring", "deft", "dewy", "dreamy", "eager", "early", "easy",
    "elated", "epic", "even", "fair", "fancy", "fleet", "fluent", "fresh", "frosty", "fuzzy",
    "gentle", "giddy", "glad", "golden", "grand", "green", "happy", "hardy", "hazy", "honest",
    "humble", "icy", "jaunty", "jolly", "jovial", "keen", "kind", "lively", "lofty", "loyal",
    "lucid", "lucky", "lunar", "mellow", "merry", "mighty", "misty", "modest", "nimble", "noble",
    "novel", "oaken", "olive", "open", "patient", "peppy", "placid", "plucky", "polar", "polite",
    "proud", "quick", "quiet", "rapid", "ready", "regal", "rosy", "royal", "rustic", "sandy",
    "savvy", "serene", "sharp", "shiny", "silent", "silver", "simple", "sleek", "smooth", "snappy",
    "snowy", "solar", "solid", "sonic", "spry", "stable", "steady", "stellar", "sturdy", "sunny",
    "super", "swift", "tidy", "tranquil", "true", "trusty", "upbeat", "urban", "valiant", "vivid",
    "warm", "wavy", "wild", "windy", "wise", "witty", "young", "zany", "zen", "zesty",
];

const VERBS: &[&str] = &[
    "baking",
    "biking",
    "blinking",
    "blooming",
    "bouncing",
    "bowling",
    "boxing",
    "brewing",
    "bubbling",
    "building",
    "camping",
    "carving",
    "chasing",
    "chirping",
    "climbing",
    "coasting",
    "coding",
    "cooking",
    "crafting",
    "crawling",
    "cruising",
    "cycling",
    "dancing",
    "darting",
    "dashing",
    "diving",
    "drawing",
    "dreaming",
    "drifting",
    "drumming",
    "fencing",
    "fishing",
    "floating",
    "flowing",
    "flying",
    "folding",
    "gardening",
    "gazing",
    "gliding",
    "glowing",
    "golfing",
    "grinning",
    "growing",
    "hiking",
    "hoping",
    "hopping",
    "humming",
    "jogging",
    "joking",
    "juggling",
    "jumping",
    "kayaking",
    "knitting",
    "landing",
    "laughing",
    "leaping",
    "learning",
    "lifting",
    "listening",
    "looping",
    "mapping",
    "marching",
    "mending",
    "mixing",
    "napping",
    "nesting",
    "nodding",
    "pacing",
    "paddling",
    "painting",
    "planting",
    "playing",
    "plotting",
    "pondering",
    "prancing",
    "quilting",
    "racing",
    "rafting",
    "rambling",
    "reading",
    "rhyming",
    "riding",
    "roaming",
    "rolling",
    "rowing",
    "running",
    "sailing",
    "scouting",
    "sculpting",
    "singing",
    "sipping",
    "skating",
    "sketching",
    "skiing",
    "skipping",
    "sliding",
    "smiling",
    "snoozing",
    "soaring",
    "sparkling",
    "spinning",
    "sprinting",
    "stacking",
    "stretching",
    "strolling",
    "surfing",
    "swaying",
    "sweeping",
    "swimming",
    "swinging",
    "tapping",
    "tending",
    "thinking",
    "tinkering",
    "trekking",
    "trotting",
    "tumbling",
    "twirling",
    "typing",
    "voyaging",
    "waddling",
    "walking",
    "wandering",
    "waving",
    "weaving",
    "whistling",
    "winking",
    "writing",
    "yodeling",
    "zipping",
    "zooming",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::{RouteName, WorkspaceName};

    #[test]
    fn every_word_makes_valid_names() {
        for word in ADJECTIVES.iter().chain(VERBS) {
            let name = format!("{word}-{word}");
            assert!(name.parse::<RouteName>().is_ok(), "{word}");
        }
    }

    #[test]
    fn candidates_are_valid_route_and_workspace_names() {
        for entropy in 0..500u64 {
            for attempt in [0, 1, BARE_ATTEMPTS, BARE_ATTEMPTS + 5] {
                let label = candidate(entropy, attempt);
                assert!(RouteName::try_from(label.clone()).is_ok(), "{label}");
                assert!(WorkspaceName::try_from(label).is_ok());
            }
        }
    }

    #[test]
    fn labels_name_by_their_letters_and_digits() {
        let named = |text: &str, attempt| from_label(text, attempt).map(|l| l.as_str().to_owned());
        assert_eq!(
            named("More UI Tweaks", 0).as_deref(),
            Some("more-ui-tweaks")
        );
        assert_eq!(
            named("  fix: the login (again)! ", 0).as_deref(),
            Some("fix-the-login-again")
        );
        assert_eq!(named("2024 review", 0).as_deref(), Some("review"));
        assert_eq!(named("Café ☕ notes", 1).as_deref(), Some("caf-notes-2"));
        assert_eq!(named("修正", 0), None);
        let long = named(&"word ".repeat(20), 0).expect("a name");
        assert!(long.len() <= MADE_MAX && !long.ends_with('-'), "{long}");
        let label = from_label("More UI Tweaks", 3).expect("a name");
        assert!(WorkspaceName::try_from(label).is_ok());
    }

    #[test]
    fn branches_name_by_their_last_part() {
        let named = |text: &str| from_branch(text, 0).map(|l| l.as_str().to_owned());
        assert_eq!(named("su/login-fix").as_deref(), Some("login-fix"));
        assert_eq!(named("main").as_deref(), Some("main"));
        assert_eq!(
            named("feature/JIRA-123_Thing").as_deref(),
            Some("jira-123-thing")
        );
    }

    #[test]
    fn a_label_is_one_line_of_text() {
        assert!("More UI tweaks".parse::<WorkspaceLabel>().is_ok());
        assert!("x".repeat(80).parse::<WorkspaceLabel>().is_ok());
        assert!("x".repeat(81).parse::<WorkspaceLabel>().is_err());
        assert!(" padded".parse::<WorkspaceLabel>().is_err());
        assert!("".parse::<WorkspaceLabel>().is_err());
        assert!("two\nlines".parse::<WorkspaceLabel>().is_err());
    }

    #[test]
    fn later_attempts_add_a_suffix() {
        assert_eq!(candidate(7, 0).as_str().matches('-').count(), 1);
        assert_eq!(candidate(7, BARE_ATTEMPTS).as_str().matches('-').count(), 2);
    }

    /// Even entropies one apart get unrelated names, spread over every word:
    /// 2,000 draws from about 17,000 names give about 1,880 different ones.
    #[test]
    fn candidates_spread_over_every_word() {
        let names: Vec<DnsLabel> = (0..2000u64).map(|entropy| candidate(entropy, 0)).collect();
        let words = |at: usize| {
            names
                .iter()
                .map(|n| n.as_str().split('-').nth(at).map(str::to_owned))
                .collect::<std::collections::HashSet<_>>()
                .len()
        };
        let distinct: std::collections::HashSet<_> = names.iter().collect();
        assert!(distinct.len() > 1800, "{} different names", distinct.len());
        assert!(
            words(0) > ADJECTIVES.len() * 9 / 10,
            "{} adjectives",
            words(0)
        );
        assert!(words(1) > VERBS.len() * 9 / 10, "{} verbs", words(1));
    }

    /// `SplitMix64`'s published outputs from a state of zero.
    #[test]
    fn the_mixer_is_splitmix64() {
        const GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;
        assert_eq!(splitmix64(0), 0xe220_a839_7b1d_cdaf);
        assert_eq!(splitmix64(GAMMA), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(splitmix64(GAMMA.wrapping_mul(2)), 0x06c4_5d18_8009_454f);
    }

    /// Each retry after a taken name offers a new one.
    #[test]
    fn attempts_produce_different_candidates() {
        for entropy in (0..200u64).chain((0..200).map(|i| u64::MAX - i)) {
            let names: std::collections::HashSet<_> = (0..BARE_ATTEMPTS)
                .map(|attempt| candidate(entropy, attempt))
                .collect();
            assert!(
                u32::try_from(names.len()).is_ok_and(|n| n + 1 >= BARE_ATTEMPTS),
                "{entropy}: {names:?}"
            );
        }
    }

    #[test]
    fn words_are_unique() {
        let mut all: Vec<_> = ADJECTIVES.iter().chain(VERBS).collect();
        let len = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), len);
    }
}
