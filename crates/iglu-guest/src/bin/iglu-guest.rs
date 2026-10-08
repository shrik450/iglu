//! `iglu-guest`: what hostd runs inside a workspace. Not meant for people.

use std::io::Read;
use std::process::ExitCode;

use iglu_domain::repo::{BranchName, RepoUrl};
use iglu_domain::terminal::SessionName;
use iglu_guest::credential::{self, Stored};
use iglu_guest::paths::Dirs;
use iglu_guest::{provision, secrets, session};

const USAGE: &str = "usage:
  iglu-guest install-secrets      (reads the bundle on stdin)
  iglu-guest provision --repo <url> --branch <name> [--base <name>]
  iglu-guest sessions
  iglu-guest attach <session>
  iglu-guest close <session>
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
        _ => fail(USAGE),
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
