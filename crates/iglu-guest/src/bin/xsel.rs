//! `xsel`: copies to the person's clipboard through the console, for
//! programs that copy by running it. See `iglu_guest::clipboard`.

use std::io::IsTerminal;
use std::process::ExitCode;

use iglu_guest::clipboard;

const USAGE: &str = "usage: xsel [-i | -o] [-b | -p | -s] [--trim]
  copies to your clipboard through the iglu console; a workspace can't read it back";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let asked = clipboard::xsel(&args, std::io::stdin().is_terminal());
    clipboard::run("xsel", USAGE, asked)
}
