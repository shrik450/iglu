//! `caffeinate`: keeps the workspace awake while a command runs. It reports
//! the command as a working thread of this terminal, the way a busy agent
//! does, so auto-idle doesn't freeze a long build or test run.
//!
//!     caffeinate <command> [args...]   awake until the command ends
//!     caffeinate                       awake until Ctrl-C
//!
//! Outside an iglu terminal it only runs the command.

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitCode, ExitStatus};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use iglu_domain::attention::{AttentionState, Summary, ThreadKey};
use iglu_domain::terminal::SessionName;
use iglu_guest::paths::Dirs;
use iglu_guest::status::{self, Report};

/// Where the status goes, inside an iglu terminal.
struct Reporting {
    dirs: Dirs,
    session: SessionName,
    thread: ThreadKey,
}

impl Reporting {
    fn here() -> Option<Self> {
        let session = std::env::var("ZMX_SESSION").ok()?.parse().ok()?;
        let thread = format!("caffeinate-{}", std::process::id()).parse().ok()?;
        Some(Self {
            dirs: Dirs::from_env().ok()?,
            session,
            thread,
        })
    }

    /// Advisory, like every status: a failure is mentioned, never fatal.
    fn report(&self, report: Report) {
        let (session, thread) = (self.session.clone(), self.thread.clone());
        if let Err(error) = status::modify(&self.dirs, |current| {
            status::update(current, session, thread, report)
        }) {
            eprintln!("caffeinate: {error}");
        }
    }
}

/// The command's exit code, or 128 plus the signal that ended it, as shells do.
fn code(status: ExitStatus) -> u8 {
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reporting = Reporting::here();
    let what = Summary::sanitize(&format!("caffeinate: {}", args.join(" ")));
    if let Some(reporting) = &reporting {
        reporting.report(Report::Set {
            state: AttentionState::Working,
            summary: what.clone(),
            title: Some(what),
            at: status::now_millis(),
        });
    }
    // Interrupts and hangups reach the command, which shares this terminal;
    // this stays to clear its status after it, so a workspace never stays
    // awake for a command that's gone.
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        if let Err(error) = signal_hook::flag::register(signal, Arc::clone(&stop)) {
            eprintln!("caffeinate: {error}");
        }
    }
    let exit = match args.split_first() {
        None => {
            eprintln!("caffeinate: keeping this workspace awake; Ctrl-C stops");
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(200));
            }
            0
        }
        Some((program, rest)) => match Command::new(program).args(rest).status() {
            Ok(status) => code(status),
            Err(error) => {
                eprintln!("caffeinate: {program}: {error}");
                127
            }
        },
    };
    if let Some(reporting) = &reporting {
        reporting.report(Report::Clear);
    }
    ExitCode::from(exit)
}
