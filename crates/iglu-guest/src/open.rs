//! `xdg-open` for a workspace, which has no desktop to open things on: it
//! asks the console, through the terminal, to offer to open a web address
//! or file. The person chooses where: their own browser, or the
//! workspace's, which is the one that reaches what listens inside it.

use std::path::Path;

/// The OSC that asks the console to offer to open an address: iglu's own,
/// "iglu" on a phone's keys. The console reads it in
/// `console/src/state/replies.ts`; terminals that don't know it ignore it.
pub const OSC: u16 = 4458;

/// The longest address asked for, as sent: sign-in addresses run long, but
/// not this long.
pub const MAX_BYTES: usize = 16 * 1024;

/// What `xdg-open` was given to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A web or file address, as the console is sent it: printable ASCII.
    Address(String),
    /// A file's path, which opens as a file address.
    Path(String),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("a workspace opens web addresses and files, not {0}: addresses")]
    Scheme(String),
    #[error("the address has control characters in it")]
    Control,
    #[error("the address is longer than 16 KiB")]
    Long,
}

/// What `arg` asks to open: an address, if it starts with a scheme, and
/// otherwise a path.
///
/// # Errors
///
/// When it's an address of a kind only a desktop could open, or one that
/// couldn't be sent as it is.
pub fn target(arg: &str) -> Result<Target, Refused> {
    let Some((scheme, _)) = arg.split_once(':').filter(|(scheme, _)| is_scheme(scheme)) else {
        return Ok(Target::Path(arg.to_owned()));
    };
    if !["http", "https", "file"]
        .iter()
        .any(|s| s.eq_ignore_ascii_case(scheme))
    {
        return Err(Refused::Scheme(scheme.to_owned()));
    }
    if arg.chars().any(char::is_control) {
        return Err(Refused::Control);
    }
    let address = encoded(arg, |byte| byte.is_ascii_graphic());
    if address.len() > MAX_BYTES {
        return Err(Refused::Long);
    }
    Ok(Target::Address(address))
}

/// A scheme as RFC 3986 has it, of two letters or more, so a Windows
/// drive's letter isn't one.
fn is_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    s.len() > 1
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
}

/// `text` with each byte `keep` doesn't take written as `%XX`.
fn encoded(text: &str, keep: impl Fn(u8) -> bool) -> String {
    text.bytes()
        .map(|byte| {
            if keep(byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// The file address of an absolute `path`.
///
/// # Errors
///
/// When the address would be longer than the console takes.
pub fn file_address(path: &Path) -> Result<String, Refused> {
    let path = path.as_os_str().to_string_lossy();
    let address = format!(
        "file://{}",
        encoded(&path, |b| b.is_ascii_alphanumeric()
            || b"/-._~".contains(&b))
    );
    if address.len() > MAX_BYTES {
        return Err(Refused::Long);
    }
    Ok(address)
}

/// What the terminal is sent to offer to open `address`.
#[must_use]
pub fn osc(address: &str) -> Vec<u8> {
    format!("\x1b]{OSC};{address}\x1b\\").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_and_file_addresses_are_sent_as_given() {
        let device = "https://github.com/login/device";
        assert_eq!(target(device), Ok(Target::Address(device.to_owned())));
        assert_eq!(
            target("HTTP://localhost:3000/a?b=c#d"),
            Ok(Target::Address("HTTP://localhost:3000/a?b=c#d".to_owned()))
        );
        assert_eq!(
            target("file:///home/dev/report.html"),
            Ok(Target::Address("file:///home/dev/report.html".to_owned()))
        );
    }

    #[test]
    fn what_a_terminal_could_misread_is_encoded() {
        assert_eq!(
            target("https://example.com/caf\u{e9} x"),
            Ok(Target::Address(
                "https://example.com/caf%C3%A9%20x".to_owned()
            ))
        );
        assert_eq!(
            target("https://example.com/\x1b]0;x\x07"),
            Err(Refused::Control)
        );
        assert_eq!(target("https://example.com/\u{9c}"), Err(Refused::Control));
        let long = format!("https://example.com/{}", "a".repeat(MAX_BYTES));
        assert_eq!(target(&long), Err(Refused::Long));
    }

    #[test]
    fn other_schemes_are_refused_and_the_rest_are_paths() {
        assert_eq!(
            target("mailto:a@b.c"),
            Err(Refused::Scheme("mailto".to_owned()))
        );
        assert_eq!(
            target("javascript:alert(1)"),
            Err(Refused::Scheme("javascript".to_owned()))
        );
        assert_eq!(
            target("report.html"),
            Ok(Target::Path("report.html".to_owned()))
        );
        assert_eq!(target("C:notes"), Ok(Target::Path("C:notes".to_owned())));
        assert_eq!(target("./a:b"), Ok(Target::Path("./a:b".to_owned())));
    }

    #[test]
    fn a_files_address_encodes_what_isnt_a_plain_path() {
        assert_eq!(
            file_address(Path::new("/home/dev/my report#1.html")),
            Ok("file:///home/dev/my%20report%231.html".to_owned())
        );
    }

    #[test]
    fn the_offer_is_iglus_osc() {
        assert_eq!(osc("https://a.b/"), b"\x1b]4458;https://a.b/\x1b\\");
    }
}
