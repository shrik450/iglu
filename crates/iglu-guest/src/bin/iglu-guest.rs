//! `iglu-guest`: what hostd runs inside a workspace. Not meant for people.

use std::io::Read;
use std::process::ExitCode;

use iglu_domain::guest::OutputReport;
use iglu_domain::pasted::{self, FileName};
use iglu_domain::repo::{BranchName, RepoUrl};
use iglu_domain::terminal::{self, OutputLines, SessionName, TerminalInput};
use iglu_guest::credential::{self, Stored};
use iglu_guest::paths::Dirs;
use iglu_guest::{git, listeners, provision, secrets, session};

const USAGE: &str = "usage:
  iglu-guest install-secrets      (reads the bundle on stdin)
  iglu-guest provision [--repo <url> --branch <name> [--base <name>]]
  iglu-guest open [--boot]          (reads the sessions on stdin)
  iglu-guest sessions
  iglu-guest listeners            (what the user is listening on)
  iglu-guest git-state            (where the checkout stands)
  iglu-guest attach <session>
  iglu-guest close <session>
  iglu-guest output <session> --lines <n>   (its last lines, as JSON)
  iglu-guest input <session>      (types what's on stdin into it)
  iglu-guest keep <file-name>     (keeps a pasted file from stdin; prints its path)
  iglu-guest git-credential <get|store|erase>
  iglu-guest interface             (the version of the interface hosts use)";

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("iglu-guest: {message}");
    ExitCode::FAILURE
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        return fail(USAGE);
    };
    let rest = &args[1..];
    if command == "interface" {
        println!("{}", iglu_domain::guest::INTERFACE);
        return ExitCode::SUCCESS;
    }
    if command == "git-credential" {
        return git_credential(rest);
    }
    if command == "listeners" {
        return match serde_json::to_string(&listeners::scan(std::path::Path::new("/proc"))) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => fail(error),
        };
    }
    let dirs = match Dirs::from_env() {
        Ok(dirs) => dirs,
        Err(error) => return fail(error),
    };
    match command.as_str() {
        "install-secrets" => {
            let mut request = Vec::new();
            if let Err(error) = std::io::stdin().read_to_end(&mut request) {
                return fail(format!("reading the bundle: {error}"));
            }
            match secrets::install(&dirs, &request) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => fail(error),
            }
        }
        "provision" if rest.is_empty() => match provision::without_repository(&dirs) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => fail(error),
        },
        "provision" => {
            let parsed = (|| -> Result<provision::Spec, String> {
                let repo: RepoUrl = flag(rest, "--repo")
                    .ok_or("--repo is required")?
                    .parse()
                    .map_err(|e| format!("{e}"))?;
                let branch: BranchName = flag(rest, "--branch")
                    .ok_or("--branch is required")?
                    .parse()
                    .map_err(|e| format!("{e}"))?;
                let base = flag(rest, "--base")
                    .map(|b| b.parse::<BranchName>())
                    .transpose()
                    .map_err(|e| format!("{e}"))?;
                Ok(provision::Spec { repo, branch, base })
            })();
            match parsed {
                Ok(spec) => match provision::run(&dirs, &spec) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => fail(error),
                },
                Err(error) => fail(error),
            }
        }
        "open" => open(&dirs, rest),
        "git-state" => match serde_json::to_string(&git::state(&dirs)) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => fail(error),
        },
        "sessions" => match session::list(&dirs) {
            Ok(list) => match serde_json::to_string(&list) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(error) => fail(error),
            },
            Err(error) => fail(error),
        },
        "attach" => match rest.first().map(|s| s.parse::<SessionName>()) {
            Some(Ok(name)) => fail(session::attach(&dirs, &name)),
            Some(Err(error)) => fail(error),
            None => fail(USAGE),
        },
        "close" => match rest.first().map(|s| s.parse::<SessionName>()) {
            Some(Ok(name)) => match session::close(&dirs, &name) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => fail(error),
            },
            Some(Err(error)) => fail(error),
            None => fail(USAGE),
        },
        "output" => output(&dirs, rest),
        "input" => input(&dirs, rest),
        "keep" => keep(&dirs, rest),
        _ => fail(USAGE),
    }
}

fn output(dirs: &Dirs, rest: &[String]) -> ExitCode {
    let name = match rest.first().map(|s| s.parse::<SessionName>()) {
        Some(Ok(name)) => name,
        Some(Err(error)) => return fail(error),
        None => return fail(USAGE),
    };
    let lines = match flag(rest, "--lines").map(|n| n.parse::<OutputLines>()) {
        Some(Ok(lines)) => lines,
        Some(Err(error)) => return fail(error),
        None => return fail(USAGE),
    };
    let history = match session::history(dirs, &name) {
        Ok(history) => history,
        Err(error) => return fail(error),
    };
    let (text, truncated) = terminal::last_lines(&history, lines);
    match serde_json::to_string(&OutputReport { text, truncated }) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => fail(error),
    }
}

fn keep(dirs: &Dirs, rest: &[String]) -> ExitCode {
    let name = match rest.first().map(|s| s.parse::<FileName>()) {
        Some(Ok(name)) => name,
        Some(Err(error)) => return fail(error),
        None => return fail(USAGE),
    };
    let mut bytes = Vec::new();
    if let Err(error) = std::io::stdin()
        .take(u64::try_from(pasted::MAX_BYTES).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
    {
        return fail(format!("reading the file: {error}"));
    }
    if bytes.len() > pasted::MAX_BYTES {
        return fail("the file is larger than 16 MiB");
    }
    match iglu_guest::pasted::keep(dirs, &name, &bytes) {
        Ok(path) => {
            println!("{}", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => fail(error),
    }
}

fn input(dirs: &Dirs, rest: &[String]) -> ExitCode {
    let name = match rest.first().map(|s| s.parse::<SessionName>()) {
        Some(Ok(name)) => name,
        Some(Err(error)) => return fail(error),
        None => return fail(USAGE),
    };
    let mut text = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut text) {
        return fail(format!("reading the input: {error}"));
    }
    let text = match text.parse::<TerminalInput>() {
        Ok(text) => text,
        Err(error) => return fail(error),
    };
    match session::send(dirs, &name, &text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}

/// Opens the sessions on stdin. One column on request is all-or-nothing; a
/// boot's opening carries on past a broken one.
fn open(dirs: &Dirs, rest: &[String]) -> ExitCode {
    let opening = if rest.iter().any(|a| a == "--boot") {
        session::Opening::Boot
    } else {
        session::Opening::Column
    };
    let mut request = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut request) {
        return fail(format!("reading the sessions: {error}"));
    }
    match session::open(dirs, &request, opening) {
        Ok(failures) => {
            for failure in &failures {
                eprintln!(
                    "iglu-guest: {} didn't open: {}",
                    failure.name, failure.reason
                );
            }
            if opening == session::Opening::Column && !failures.is_empty() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => fail(error),
    }
}

/// Only `get` answers; `store` and `erase` are no-ops because credentials
/// come from the control plane. Without the delivered secrets there's
/// nothing to answer with, which Git takes as "ask the next helper".
fn git_credential(rest: &[String]) -> ExitCode {
    if rest.first().map(String::as_str) != Some("get") {
        return ExitCode::SUCCESS;
    }
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return ExitCode::SUCCESS;
    }
    let stored: Vec<Stored> = Dirs::from_env()
        .ok()
        .and_then(|dirs| std::fs::read(dirs.git_credentials()).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if let Some(answer) = credential::answer(&credential::parse_request(&input), &stored) {
        print!("{answer}");
    }
    ExitCode::SUCCESS
}
