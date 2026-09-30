//! Paths shared with hostd and the platform NixOS module.

/// The user-owned tmpfs directory secrets are delivered into.
pub const SECRETS_DIR: &str = "/run/iglu/secrets";
/// Where hostd drops a bundle for `install-secrets`.
pub const SECRETS_INCOMING: &str = "/run/iglu/secrets/.incoming.json";
pub const SECRETS_FILES: &str = "/run/iglu/secrets/files";
pub const SECRETS_ENV: &str = "/run/iglu/secrets/env.json";
pub const GIT_CREDENTIALS: &str = "/run/iglu/secrets/git-credentials.json";
/// Written last by `install-secrets`; hostd reads it to know what the guest holds.
pub const SECRETS_GENERATION: &str = "/run/iglu/secrets/generation";

/// The user-owned directory attention status lives in.
pub const STATUS_DIR: &str = "/run/iglu/status";
pub const STATUS_FILE: &str = "/run/iglu/status/sessions.json";
pub const STATUS_LOCK: &str = "/run/iglu/status/.lock";

/// Under the user's home: what iglu recorded about this workspace.
pub const STATE_DIR: &str = ".local/state/iglu";
pub const WORKSPACE_FILE: &str = ".local/state/iglu/workspace.json";
pub const SECRET_LINKS_FILE: &str = ".local/state/iglu/secret-links.json";
