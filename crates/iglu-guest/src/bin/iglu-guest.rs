//! `iglu-guest`: what hostd runs inside a workspace. Not meant for people.

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use iglu_domain::repo::{BranchName, RepoUrl};
use iglu_domain::terminal::SessionName;
use iglu_guest::credential::{self, Stored};
use iglu_guest::{paths, provision, secrets, session};

const USAGE: &str = "usage:
  iglu-guest install-secrets <bundle.json>
  iglu-guest provision --repo <url> --branch <name> [--base <name>]
  iglu-guest sessions
  iglu-guest attach <session>
  iglu-guest close <session>
  iglu-guest git-credential <get|store|erase>";

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
    match command.as_str() {
        "install-secrets" => match rest.first() {
            Some(file) => match secrets::install(Path::new(file)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => fail(error),
            },
            None => fail(USAGE),
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
                Ok(spec) => match provision::run(&spec) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => fail(error),
                },
                Err(error) => fail(error),
            }
        }
        "sessions" => match session::list() {
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
            Some(Ok(name)) => fail(session::attach(&name)),
            Some(Err(error)) => fail(error),
            None => fail(USAGE),
        },
        "close" => match rest.first().map(|s| s.parse::<SessionName>()) {
            Some(Ok(name)) => match session::close(&name) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => fail(error),
            },
            Some(Err(error)) => fail(error),
            None => fail(USAGE),
        },
        "git-credential" => {
            // Only `get` answers; `store` and `erase` are no-ops because
            // credentials come from the control plane.
            if rest.first().map(String::as_str) != Some("get") {
                return ExitCode::SUCCESS;
            }
            let mut input = String::new();
            if std::io::stdin().read_to_string(&mut input).is_err() {
                return ExitCode::SUCCESS;
            }
            let stored: Vec<Stored> = std::fs::read(paths::GIT_CREDENTIALS)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            if let Some(answer) = credential::answer(&credential::parse_request(&input), &stored) {
                print!("{answer}");
            }
            ExitCode::SUCCESS
        }
        _ => fail(USAGE),
    }
}
