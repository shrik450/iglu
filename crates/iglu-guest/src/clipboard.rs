//! `xclip` and `xsel` for a workspace, which has no display: what they copy
//! goes to the terminal as OSC 52, and the console puts it on the person's
//! clipboard. Programs such as `gh` copy by running one of them.
//!
//! The person has one clipboard, so every selection, primary or clipboard,
//! is it. Nothing reads it back: a page can't read the clipboard without
//! asking, and a workspace mustn't learn what the person copied elsewhere.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use base64::Engine;

use crate::tty;

/// What a clipboard tool was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    /// Copies the files' contents, one after another, or stdin with none.
    Copy {
        files: Vec<PathBuf>,
        /// Without the input's last newline.
        trim: bool,
        /// Writing the input to stdout too, as it's copied.
        echo: bool,
    },
    /// Reads the clipboard back, which a workspace can't.
    Paste,
    Help,
    Version,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("unknown option {0}")]
    Unknown(String),
    #[error("{0} could be more than one option")]
    Ambiguous(String),
    #[error("{0} needs a value")]
    Missing(String),
    #[error("there's no selection called {0}")]
    Selection(String),
    #[error("only text reaches the clipboard from a workspace, not {0}")]
    NotText(String),
    #[error("{0} isn't something a workspace's clipboard can do; it can only be copied to")]
    Unsupported(String),
}

/// What the terminal is sent to copy `text`: OSC 52, to the clipboard.
#[must_use]
pub fn osc52(text: &[u8]) -> Vec<u8> {
    let mut out = b"\x1b]52;c;".to_vec();
    out.extend(
        base64::engine::general_purpose::STANDARD
            .encode(text)
            .bytes(),
    );
    out.extend(b"\x1b\\");
    out
}

/// `text` without its last newline, when `trim` says so.
#[must_use]
pub fn trimmed(text: &[u8], trim: bool) -> &[u8] {
    match text {
        [rest @ .., b'\n'] if trim => rest,
        _ => text,
    }
}

const SELECTIONS: [&str; 3] = ["primary", "secondary", "clipboard"];

/// Targets that are text, which is all the console's clipboard takes.
const TEXT: [&str; 6] = [
    "UTF8_STRING",
    "STRING",
    "TEXT",
    "COMPOUND_TEXT",
    "text/plain",
    "text/plain;charset=utf-8",
];

/// A selection's name, or as much of it as names one, as xclip takes it.
fn selection(name: &str) -> Result<(), Refused> {
    if !name.is_empty() && SELECTIONS.iter().any(|s| s.starts_with(name)) {
        Ok(())
    } else {
        Err(Refused::Selection(name.to_owned()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Xclip {
    In,
    Out,
    Filter,
    RemoveNewline,
    Selection,
    Target,
    Help,
    Version,
    /// Changes only how xclip talks to X, or what it says.
    Ignored,
    /// Takes a value that changes only how xclip talks to X.
    IgnoredValue,
    Unsupported,
}

const XCLIP: [(&str, Xclip); 15] = [
    ("-in", Xclip::In),
    ("-out", Xclip::Out),
    ("-filter", Xclip::Filter),
    ("-rmlastnl", Xclip::RemoveNewline),
    ("-selection", Xclip::Selection),
    ("-target", Xclip::Target),
    ("-help", Xclip::Help),
    ("-version", Xclip::Version),
    ("-silent", Xclip::Ignored),
    ("-quiet", Xclip::Ignored),
    ("-verbose", Xclip::Ignored),
    ("-noutf8", Xclip::Ignored),
    ("-sensitive", Xclip::Ignored),
    ("-display", Xclip::IgnoredValue),
    ("-loops", Xclip::Unsupported),
];

/// xclip's option `arg` names: the whole of one, or the start of only one.
fn xclip_option(arg: &str) -> Result<Xclip, Refused> {
    if let Some((_, option)) = XCLIP.iter().find(|(name, _)| *name == arg) {
        return Ok(*option);
    }
    let mut matching = XCLIP
        .iter()
        .filter(|(name, _)| arg.len() > 1 && name.starts_with(arg));
    match (matching.next(), matching.next()) {
        (Some((_, option)), None) => Ok(*option),
        (Some(_), Some(_)) => Err(Refused::Ambiguous(arg.to_owned())),
        (None, _) => Err(Refused::Unknown(arg.to_owned())),
    }
}

/// What `xclip <args>` asks. It copies stdin, or the files named, unless
/// it's asked to print the clipboard.
///
/// # Errors
///
/// When the arguments aren't xclip's, or ask for what a workspace can't do.
pub fn xclip(args: &[String]) -> Result<Asked, Refused> {
    let mut args = args.iter();
    let mut files = Vec::new();
    let (mut paste, mut trim, mut echo) = (false, false, false);
    while let Some(arg) = args.next() {
        if !arg.starts_with('-') || arg == "-" {
            files.push(PathBuf::from(arg));
            continue;
        }
        let option = xclip_option(arg)?;
        let mut value = || args.next().ok_or_else(|| Refused::Missing(arg.clone()));
        match option {
            Xclip::In => paste = false,
            Xclip::Out => paste = true,
            Xclip::Filter => echo = true,
            Xclip::RemoveNewline => trim = true,
            Xclip::Selection => selection(value()?)?,
            Xclip::Target => {
                let target = value()?;
                if !TEXT.iter().any(|t| t.eq_ignore_ascii_case(target)) {
                    return Err(Refused::NotText(target.clone()));
                }
            }
            Xclip::Help => return Ok(Asked::Help),
            Xclip::Version => return Ok(Asked::Version),
            Xclip::Ignored => {}
            Xclip::IgnoredValue => {
                value()?;
            }
            Xclip::Unsupported => return Err(Refused::Unsupported(arg.clone())),
        }
    }
    Ok(if paste {
        Asked::Paste
    } else {
        Asked::Copy { files, trim, echo }
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Xsel {
    Input,
    Output,
    Selection,
    Trim,
    Help,
    Version,
    Ignored,
    IgnoredValue,
    Unsupported,
}

/// xsel's options: each long name and its letter, if it has one.
const XSEL: [(&str, Option<char>, Xsel); 21] = [
    ("input", Some('i'), Xsel::Input),
    ("output", Some('o'), Xsel::Output),
    ("primary", Some('p'), Xsel::Selection),
    ("secondary", Some('s'), Xsel::Selection),
    ("clipboard", Some('b'), Xsel::Selection),
    ("trim", None, Xsel::Trim),
    ("help", Some('h'), Xsel::Help),
    ("version", None, Xsel::Version),
    ("nodetach", Some('n'), Xsel::Ignored),
    ("verbose", Some('v'), Xsel::Ignored),
    ("keep", Some('k'), Xsel::Ignored),
    ("selectionTimeout", Some('t'), Xsel::IgnoredValue),
    ("logfile", Some('l'), Xsel::IgnoredValue),
    ("display", None, Xsel::IgnoredValue),
    ("windowName", Some('w'), Xsel::IgnoredValue),
    ("append", Some('a'), Xsel::Unsupported),
    ("follow", Some('f'), Xsel::Unsupported),
    ("zeroflush", Some('z'), Xsel::Unsupported),
    ("clear", Some('c'), Xsel::Unsupported),
    ("delete", Some('d'), Xsel::Unsupported),
    ("exchange", Some('x'), Xsel::Unsupported),
];

/// What `xsel <args>` asks. Without `--input` or `--output`, xsel copies
/// stdin when it isn't a terminal, and otherwise prints the selection.
///
/// # Errors
///
/// When the arguments aren't xsel's, or ask for what a workspace can't do.
pub fn xsel(args: &[String], stdin_is_terminal: bool) -> Result<Asked, Refused> {
    let mut args = args.iter();
    let (mut input, mut output, mut trim) = (false, false, false);
    while let Some(arg) = args.next() {
        // Each option a long name, or letters run together, as in `-bi`.
        let options: Vec<(String, Xsel)> = if let Some(long) = arg.strip_prefix("--") {
            let (_, _, option) = XSEL
                .iter()
                .find(|(name, _, _)| *name == long)
                .ok_or_else(|| Refused::Unknown(arg.clone()))?;
            vec![(arg.clone(), *option)]
        } else if let Some(letters) = arg.strip_prefix('-').filter(|l| !l.is_empty()) {
            letters
                .chars()
                .map(|letter| {
                    XSEL.iter()
                        .find(|(_, short, _)| *short == Some(letter))
                        .map(|(_, _, option)| (format!("-{letter}"), *option))
                        .ok_or_else(|| Refused::Unknown(format!("-{letter}")))
                })
                .collect::<Result<_, _>>()?
        } else {
            return Err(Refused::Unknown(arg.clone()));
        };
        for (name, option) in options {
            match option {
                Xsel::Input => input = true,
                Xsel::Output => output = true,
                Xsel::Trim => trim = true,
                Xsel::Help => return Ok(Asked::Help),
                Xsel::Version => return Ok(Asked::Version),
                Xsel::Selection | Xsel::Ignored => {}
                Xsel::IgnoredValue => {
                    args.next().ok_or(Refused::Missing(name))?;
                }
                Xsel::Unsupported => return Err(Refused::Unsupported(name)),
            }
        }
    }
    let copies = input || (!output && !stdin_is_terminal);
    Ok(if copies {
        Asked::Copy {
            files: Vec::new(),
            trim,
            echo: false,
        }
    } else {
        Asked::Paste
    })
}

/// Does what `tool` was asked, as `usage` describes it.
#[must_use]
pub fn run(tool: &str, usage: &str, asked: Result<Asked, Refused>) -> ExitCode {
    let fail = |message: &dyn std::fmt::Display| {
        eprintln!("{tool}: {message}");
        ExitCode::FAILURE
    };
    let (files, trim, echo) = match asked {
        Ok(Asked::Copy { files, trim, echo }) => (files, trim, echo),
        Ok(Asked::Paste) => {
            return fail(&"a workspace can't read your clipboard; it can only copy to it");
        }
        Ok(Asked::Help) => {
            println!("{usage}");
            return ExitCode::SUCCESS;
        }
        Ok(Asked::Version) => {
            println!("{tool} for iglu {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(refused) => return fail(&format!("{refused}\n{usage}")),
    };
    let mut text = Vec::new();
    let read = if files.is_empty() {
        std::io::stdin().read_to_end(&mut text).map(drop)
    } else {
        files.iter().try_for_each(|file| {
            if file.as_os_str() == "-" {
                std::io::stdin().read_to_end(&mut text).map(drop)
            } else {
                std::fs::File::open(file).and_then(|mut f| f.read_to_end(&mut text).map(drop))
            }
        })
    };
    if let Err(error) = read {
        return fail(&error);
    }
    let text = trimmed(&text, trim);
    if let Err(error) = tty::write(&osc52(text)) {
        return fail(&format!("can't reach the console to copy: {error}"));
    }
    if echo && let Err(error) = std::io::stdout().write_all(text) {
        return fail(&error);
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    fn copy(files: &[&str], trim: bool, echo: bool) -> Asked {
        Asked::Copy {
            files: files.iter().map(PathBuf::from).collect(),
            trim,
            echo,
        }
    }

    #[test]
    fn a_copy_is_osc_52_to_the_clipboard() {
        assert_eq!(osc52(b"FC70-74FE"), b"\x1b]52;c;RkM3MC03NEZF\x1b\\");
        assert_eq!(osc52(b""), b"\x1b]52;c;\x1b\\");
    }

    #[test]
    fn trimming_takes_only_the_last_newline() {
        assert_eq!(trimmed(b"a\n\n", true), b"a\n");
        assert_eq!(trimmed(b"a\n", false), b"a\n");
        assert_eq!(trimmed(b"", true), b"");
    }

    #[test]
    fn xclip_copies_as_programs_ask_it_to() {
        // gh's, nvim's, and people's.
        assert_eq!(
            xclip(&args("-in -selection clipboard")),
            Ok(copy(&[], false, false))
        );
        assert_eq!(
            xclip(&args("-quiet -i -selection clipboard")),
            Ok(copy(&[], false, false))
        );
        assert_eq!(
            xclip(&args("-sel c -t text/plain")),
            Ok(copy(&[], false, false))
        );
        assert_eq!(xclip(&args("")), Ok(copy(&[], false, false)));
        assert_eq!(
            xclip(&args("-selection primary -r -f")),
            Ok(copy(&[], true, true))
        );
        assert_eq!(
            xclip(&args("-d :0 notes.txt -")),
            Ok(copy(&["notes.txt", "-"], false, false))
        );
    }

    #[test]
    fn xclip_never_reads_the_clipboard_back() {
        assert_eq!(xclip(&args("-out -selection clipboard")), Ok(Asked::Paste));
        assert_eq!(xclip(&args("-o")), Ok(Asked::Paste));
        assert_eq!(xclip(&args("-o -i")), Ok(copy(&[], false, false)));
    }

    #[test]
    fn xclip_refuses_what_it_cant_do() {
        assert_eq!(
            xclip(&args("-t image/png")),
            Err(Refused::NotText("image/png".into()))
        );
        assert_eq!(
            xclip(&args("-selection")),
            Err(Refused::Missing("-selection".into()))
        );
        assert_eq!(
            xclip(&args("-selection nope")),
            Err(Refused::Selection("nope".into()))
        );
        assert_eq!(xclip(&args("-s")), Err(Refused::Ambiguous("-s".into())));
        assert_eq!(xclip(&args("--in")), Err(Refused::Unknown("--in".into())));
        assert_eq!(
            xclip(&args("-loops 2")),
            Err(Refused::Unsupported("-loops".into()))
        );
        assert_eq!(xclip(&args("-version")), Ok(Asked::Version));
    }

    #[test]
    fn xsel_copies_as_programs_ask_it_to() {
        assert_eq!(
            xsel(&args("--input --clipboard"), true),
            Ok(copy(&[], false, false))
        );
        assert_eq!(
            xsel(&args("--nodetach -i -b"), true),
            Ok(copy(&[], false, false))
        );
        assert_eq!(xsel(&args("-bi --trim"), true), Ok(copy(&[], true, false)));
        assert_eq!(xsel(&args("-t 500 -b"), false), Ok(copy(&[], false, false)));
    }

    #[test]
    fn xsel_reads_back_only_when_asked_to_and_never_can() {
        assert_eq!(xsel(&args("--output --clipboard"), false), Ok(Asked::Paste));
        assert_eq!(xsel(&args("-b"), true), Ok(Asked::Paste));
    }

    #[test]
    fn xsel_refuses_what_it_cant_do() {
        assert_eq!(
            xsel(&args("-ba"), false),
            Err(Refused::Unsupported("-a".into()))
        );
        assert_eq!(
            xsel(&args("--clear"), true),
            Err(Refused::Unsupported("--clear".into()))
        );
        assert_eq!(xsel(&args("-q"), true), Err(Refused::Unknown("-q".into())));
        assert_eq!(xsel(&args("-t"), true), Err(Refused::Missing("-t".into())));
        assert_eq!(
            xsel(&args("clipboard"), true),
            Err(Refused::Unknown("clipboard".into()))
        );
    }
}
