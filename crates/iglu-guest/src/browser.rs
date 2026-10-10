//! The workspace's browser: Chromium without a window, which the console
//! shows and agents drive, both over Chrome's debugging protocol (CDP) on
//! loopback.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use iglu_domain::column::BROWSER_DEBUG_PORT;

use crate::paths::Dirs;

/// Chromium's arguments: headless, debuggable only from inside the
/// workspace, with a profile kept in the home directory so sign-ins last.
#[must_use]
pub fn args(profile: &Path) -> Vec<String> {
    vec![
        "--headless=new".into(),
        "--remote-debugging-address=127.0.0.1".into(),
        format!("--remote-debugging-port={BROWSER_DEBUG_PORT}"),
        format!("--user-data-dir={}", profile.display()),
        // Cookies are kept in the profile as they are: no keyring to wait on.
        "--password-store=basic".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--window-size=1280,800".into(),
        "about:blank".into(),
    ]
}

/// Replaces this process with Chromium. Returns only if it can't start.
#[must_use]
pub fn run(dirs: &Dirs) -> std::io::Error {
    let profile = dirs.home().join(".cache/iglu/browser");
    if let Err(error) = std::fs::create_dir_all(&profile) {
        return error;
    }
    // Ending a column kills the browser, which takes it for a crash and
    // would bring the last session's tabs back, unloaded. It starts afresh
    // instead, with one blank tab; sign-ins and history stay.
    match std::fs::remove_dir_all(profile.join("Default/Sessions")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return error,
    }
    let error = Command::new("chromium").args(args(&profile)).exec();
    if error.kind() == std::io::ErrorKind::NotFound {
        std::io::Error::other(
            "this workspace's environment has no chromium; the iglu module adds it unless iglu.browser.enable is off",
        )
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_is_debuggable_only_from_inside() {
        let args = args(Path::new("/home/dev/.cache/iglu/browser"));
        assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_owned()));
        assert!(args.contains(&format!("--remote-debugging-port={BROWSER_DEBUG_PORT}")));
        assert!(args.contains(&"--user-data-dir=/home/dev/.cache/iglu/browser".to_owned()));
    }
}
