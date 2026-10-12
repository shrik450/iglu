//! The terminal a program runs in, for the tools that reach the console
//! through it: what's written there goes to the browsers showing it.

use std::fs::{File, OpenOptions};
use std::io::Write;

/// How far up its ancestors a program's terminal is looked for.
const ANCESTORS: usize = 64;

/// A terminal's device, which only a pseudo-terminal's is here.
const DEVICES: &str = "/dev/pts/";

/// The process `stat` says is the parent: `pid (comm) state ppid ...`,
/// where the command's name may itself hold spaces and parentheses.
#[must_use]
pub fn parent_of(stat: &str) -> Option<u32> {
    let (_, rest) = stat.rsplit_once(')')?;
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Writes `bytes` to the terminal this program runs in: its controlling
/// terminal, or else the one its nearest ancestor reads or writes. Programs
/// that open a browser often start it in a session of its own, which has no
/// controlling terminal, though the program that asked still has one.
///
/// # Errors
///
/// When neither this program nor its ancestors have a terminal, or it can't
/// be written.
pub fn write(bytes: &[u8]) -> std::io::Result<()> {
    terminal()?.write_all(bytes)
}

fn terminal() -> std::io::Result<File> {
    if let Ok(tty) = OpenOptions::new().write(true).open("/dev/tty") {
        return Ok(tty);
    }
    let mut pid = std::process::id();
    for _ in 0..ANCESTORS {
        for fd in 0..3 {
            let Ok(device) = std::fs::read_link(format!("/proc/{pid}/fd/{fd}")) else {
                continue;
            };
            if device.to_str().is_some_and(|d| d.starts_with(DEVICES))
                && let Ok(tty) = OpenOptions::new().write(true).open(&device)
            {
                return Ok(tty);
            }
        }
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        match parent_of(&stat) {
            Some(parent) if parent > 1 => pid = parent,
            _ => break,
        }
    }
    Err(std::io::Error::other("not in a terminal"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parent_follows_the_command_however_its_named() {
        assert_eq!(parent_of("812 (gh) S 790 812 790 34816"), Some(790));
        assert_eq!(parent_of("812 (a (b) c) S 790 812"), Some(790));
        assert_eq!(parent_of("812 (gh) S"), None);
        assert_eq!(parent_of(""), None);
    }
}
