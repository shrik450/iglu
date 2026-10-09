//! Generated `adjective-verbing` names for workspaces and routes, and names
//! taken from what a workspace was started to do.
//!
//! The shell supplies entropy and checks uniqueness against storage; this
//! module only turns entropy into a well-formed name.

use crate::label::DnsLabel;

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

/// Words that say little about a task, skipped when naming it.
const FILLER: &[&str] = &[
    "about", "add", "all", "and", "any", "are", "but", "can", "could", "for", "from", "get",
    "have", "help", "into", "it's", "its", "let", "lets", "make", "need", "our", "please",
    "should", "some", "that", "the", "then", "this", "use", "want", "was", "what", "when", "with",
    "would", "you", "your",
];

/// A name from a prompt's first two telling words, such as `login-bug` for
/// "Fix the login bug". Attempts after the first add `-2`, `-3` and so on.
/// `None` when the prompt has no such words.
#[must_use]
pub fn from_prompt(prompt: &str, attempt: u32) -> Option<DnsLabel> {
    let words: Vec<String> = prompt
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
        .map(|word| word.trim_matches('\'').to_ascii_lowercase())
        .filter(|word| {
            word.len() >= 3
                && word.len() <= 20
                && word.starts_with(|c: char| c.is_ascii_lowercase())
                && word.bytes().all(|b| b.is_ascii_alphanumeric())
                && !FILLER.contains(&word.as_str())
        })
        .take(2)
        .collect();
    if words.is_empty() {
        return None;
    }
    let stem = words.join("-");
    let text = if attempt == 0 {
        stem
    } else {
        format!("{stem}-{}", attempt + 1)
    };
    text.parse().ok()
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
    fn prompts_name_by_their_telling_words() {
        let named =
            |prompt: &str, attempt| from_prompt(prompt, attempt).map(|l| l.as_str().to_owned());
        assert_eq!(named("Fix the login bug", 0).as_deref(), Some("fix-login"));
        assert_eq!(
            named("please add dark mode to the console", 0).as_deref(),
            Some("dark-mode")
        );
        assert_eq!(
            named("Can you make it faster?", 0).as_deref(),
            Some("faster")
        );
        assert_eq!(
            named("Fix the login bug", 2).as_deref(),
            Some("fix-login-3")
        );
        assert_eq!(named("修正 the 2024 bug", 0).as_deref(), Some("bug"));
        assert_eq!(named("do it", 0), None);
        let label = from_prompt("Fix the login bug", 0).expect("a name");
        assert!(WorkspaceName::try_from(label).is_ok());
    }

    #[test]
    fn later_attempts_add_a_suffix() {
        assert_eq!(candidate(7, 0).as_str().matches('-').count(), 1);
        assert_eq!(candidate(7, BARE_ATTEMPTS).as_str().matches('-').count(), 2);
    }

    #[test]
    fn attempts_produce_different_candidates() {
        let names: std::collections::HashSet<_> = (0..BARE_ATTEMPTS)
            .map(|attempt| candidate(42, attempt))
            .collect();
        assert!(names.len() > 1);
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
