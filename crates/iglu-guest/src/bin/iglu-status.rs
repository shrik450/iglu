//! `iglu-status`: tells the iglu console what the agent in this terminal is doing.
//!
//! Status updates are advisory, so this never fails loudly: outside an iglu
//! terminal it does nothing, and errors only go to stderr.

use std::io::Read;
use std::process::ExitCode;

use iglu_domain::attention::{AttentionState, Summary, ThreadKey};
use iglu_domain::terminal::SessionName;
use iglu_guest::claude;
use iglu_guest::paths::Dirs;
use iglu_guest::status::{self, Report};

const USAGE: &str = "usage:
  iglu-status set <working|waiting|done|idle|exited> [summary...]
  iglu-status clear
  iglu-status claude-hook      (reads a Claude Code hook event on stdin)";

fn record(dirs: &Dirs, session: SessionName, thread: ThreadKey, report: Report) -> ExitCode {
    match status::modify(dirs, |current| {
        status::update(current, session, thread, report)
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("iglu-status: {error}");
            ExitCode::SUCCESS
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(session), Ok(dirs)) = (
        std::env::var("ZMX_SESSION")
            .ok()
            .and_then(|s| s.parse::<SessionName>().ok()),
        Dirs::from_env(),
    ) else {
        return ExitCode::SUCCESS;
    };
    match args.first().map(String::as_str) {
        Some("set") => {
            let Some(state) = args.get(1).and_then(|s| s.parse::<AttentionState>().ok()) else {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            };
            let summary = Summary::sanitize(&args[2..].join(" "));
            let report = Report::Set {
                state,
                summary,
                title: None,
                at: status::now_millis(),
            };
            record(&dirs, session, ThreadKey::session(), report)
        }
        Some("clear") => record(&dirs, session, ThreadKey::session(), Report::Clear),
        Some("claude-hook") => {
            let mut input = String::new();
            let _ = std::io::stdin().read_to_string(&mut input);
            let Some(event) = claude::parse(&input) else {
                return ExitCode::SUCCESS;
            };
            match claude::attention(&event) {
                Some((state, summary)) => {
                    let report = Report::Set {
                        state,
                        summary,
                        title: claude::title(&event),
                        at: status::now_millis(),
                    };
                    record(&dirs, session, claude::thread(&input), report)
                }
                None => ExitCode::SUCCESS,
            }
        }
        Some(_) | None => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}
