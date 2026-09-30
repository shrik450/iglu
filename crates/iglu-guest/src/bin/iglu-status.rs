//! `iglu-status`: tells the iglu console what the agent in this terminal is doing.
//!
//! Status updates are advisory, so this never fails loudly: outside an iglu
//! terminal it does nothing, and errors only go to stderr.

use std::io::Read;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use iglu_domain::attention::{AttentionState, Summary};
use iglu_domain::terminal::SessionName;
use iglu_guest::status::{self, Entry};
use serde::Deserialize;

const USAGE: &str = "usage:
  iglu-status set <working|waiting|done|idle|exited> [summary...]
  iglu-status clear
  iglu-status claude-hook      (reads a Claude Code hook event on stdin)";

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[derive(Deserialize)]
struct HookEvent {
    hook_event_name: String,
    #[serde(default)]
    message: Option<String>,
}

fn record(session: SessionName, change: Option<(AttentionState, Summary)>) -> ExitCode {
    let entry = change.map(|(state, summary)| Entry {
        state,
        summary,
        at: now_millis(),
    });
    match status::modify(|current| status::update(current, session, entry)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("iglu-status: {error}");
            ExitCode::SUCCESS
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(session) = std::env::var("ZMX_SESSION")
        .ok()
        .and_then(|s| s.parse::<SessionName>().ok())
    else {
        return ExitCode::SUCCESS;
    };
    match args.first().map(String::as_str) {
        Some("set") => {
            let Some(state) = args.get(1).and_then(|s| s.parse::<AttentionState>().ok()) else {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            };
            let summary = Summary::sanitize(&args[2..].join(" "));
            record(session, Some((state, summary)))
        }
        Some("clear") => record(session, None),
        Some("claude-hook") => {
            let mut input = String::new();
            let _ = std::io::stdin().read_to_string(&mut input);
            match serde_json::from_str::<HookEvent>(&input) {
                Ok(event) => {
                    match status::claude_event(&event.hook_event_name, event.message.as_deref()) {
                        Some(change) => record(session, Some(change)),
                        None => ExitCode::SUCCESS,
                    }
                }
                Err(_) => ExitCode::SUCCESS,
            }
        }
        Some(_) | None => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}
