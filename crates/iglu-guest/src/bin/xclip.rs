//! `xclip`: copies to the person's clipboard through the console, for
//! programs that copy by running it. See `iglu_guest::clipboard`.

use std::process::ExitCode;

use iglu_guest::clipboard;

const USAGE: &str =
    "usage: xclip [-i | -o] [-selection primary|clipboard] [-t <text target>] [-f] [-r] [file...]
  copies to your clipboard through the iglu console; a workspace can't read it back";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    clipboard::run("xclip", USAGE, clipboard::xclip(&args))
}
