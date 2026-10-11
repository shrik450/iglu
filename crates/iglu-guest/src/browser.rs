//! The workspace's browser: Chromium without a window, which the console
//! shows and agents drive, both over Chrome's debugging protocol (CDP) on
//! loopback.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use iglu_domain::column::BROWSER_DEBUG_PORT;
use iglu_domain::download;
use serde_json::{Value, json};

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

/// The profile's preferences, as Chromium keeps them in `Default/Preferences`,
/// with downloads going to `dir` without asking: a screencast can't show the
/// browser's own prompt, and the person saves a download from there. The
/// rest are kept; preferences Chromium can't read are started afresh, as
/// Chromium would.
#[must_use]
pub fn with_downloads(preferences: Option<&str>, dir: &Path) -> String {
    let mut preferences = preferences
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let download = &mut preferences["download"];
    if !download.is_object() {
        *download = json!({});
    }
    download["default_directory"] = json!(dir.display().to_string());
    download["prompt_for_download"] = json!(false);
    download["directory_upgrade"] = json!(true);
    preferences.to_string()
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
    let downloads = dirs.home().join(download::DIR);
    let preferences = profile.join("Default/Preferences");
    let set = std::fs::create_dir_all(&downloads)
        .and_then(|()| std::fs::create_dir_all(profile.join("Default")))
        .and_then(|()| {
            let now = std::fs::read_to_string(&preferences).ok();
            std::fs::write(&preferences, with_downloads(now.as_deref(), &downloads))
        });
    if let Err(error) = set {
        return error;
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

    #[test]
    fn downloads_go_to_downloads_without_asking_and_other_preferences_stay() {
        let dir = Path::new("/home/dev/Downloads");
        let kept = r#"{"profile":{"name":"Person"},"download":{"extensions_to_open":"x"}}"#;
        let set: Value = serde_json::from_str(&with_downloads(Some(kept), dir)).expect("JSON");
        assert_eq!(set["profile"]["name"], "Person");
        assert_eq!(set["download"]["extensions_to_open"], "x");
        assert_eq!(set["download"]["default_directory"], "/home/dev/Downloads");
        assert_eq!(set["download"]["prompt_for_download"], false);
        for unreadable in [
            None,
            Some("not json"),
            Some("[1]"),
            Some(r#"{"download":3}"#),
        ] {
            let set: Value = serde_json::from_str(&with_downloads(unreadable, dir)).expect("JSON");
            assert_eq!(
                set["download"]["default_directory"], "/home/dev/Downloads",
                "{unreadable:?}"
            );
        }
    }
}
