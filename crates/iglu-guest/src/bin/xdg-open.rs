//! `xdg-open`: offers, in the console, to open a web address or a file, for
//! programs that open things by running it. See `iglu_guest::open`.

use std::process::ExitCode;

use iglu_guest::open::{self, Target};
use iglu_guest::tty;

const USAGE: &str = "usage: xdg-open <url | file>
  offers to open it in the iglu console: in your browser, or in the workspace's";

/// xdg-open's exit codes, which callers read.
const SYNTAX: u8 = 1;
const NO_FILE: u8 = 2;
const NO_TOOL: u8 = 3;

fn fail(code: u8, message: impl std::fmt::Display) -> ExitCode {
    eprintln!("xdg-open: {message}");
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = match args.as_slice() {
        [flag] if flag == "--help" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        [flag] if flag == "--version" => {
            println!("xdg-open for iglu {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        [arg] if !arg.starts_with('-') => arg,
        _ => return fail(SYNTAX, USAGE),
    };
    let address = match open::target(arg) {
        Ok(Target::Address(address)) => address,
        Ok(Target::Path(path)) => match std::fs::canonicalize(&path) {
            Ok(path) => match open::file_address(&path) {
                Ok(address) => address,
                Err(refused) => return fail(SYNTAX, refused),
            },
            Err(error) => return fail(NO_FILE, format!("{path}: {error}")),
        },
        Err(refused) => return fail(SYNTAX, refused),
    };
    match tty::write(&open::osc(&address)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(
            NO_TOOL,
            format!("can't reach the console to open {address}: {error}"),
        ),
    }
}
