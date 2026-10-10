//! Where the guest tools keep their state. Everything hangs off the user's
//! home and runtime directories, so a runtime relocates a workspace by
//! setting `HOME` and `XDG_RUNTIME_DIR`.

use std::path::{Path, PathBuf};

use iglu_domain::guest;

#[derive(Debug, thiserror::Error)]
pub enum DirsError {
    #[error("{0} isn't set")]
    Unset(&'static str),
    #[error("{0} isn't an absolute path")]
    Relative(&'static str),
}

/// The user's home and runtime directories, from the environment.
#[derive(Clone, Debug)]
pub struct Dirs {
    home: PathBuf,
    runtime: PathBuf,
    channel: Option<PathBuf>,
}

fn absolute(name: &'static str) -> Result<PathBuf, DirsError> {
    let path = PathBuf::from(std::env::var_os(name).ok_or(DirsError::Unset(name))?);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(DirsError::Relative(name))
    }
}

impl Dirs {
    /// # Errors
    ///
    /// When `HOME` or `XDG_RUNTIME_DIR` is unset or relative.
    pub fn from_env() -> Result<Self, DirsError> {
        Ok(Self {
            home: absolute("HOME")?,
            runtime: absolute("XDG_RUNTIME_DIR")?,
            channel: absolute(guest::CHANNEL_ENV).ok(),
        })
    }

    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    #[must_use]
    pub fn runtime(&self) -> &Path {
        &self.runtime
    }

    /// The workspace's channel to iglu, where the host gave one.
    #[must_use]
    pub fn channel(&self) -> Option<&Path> {
        self.channel.as_deref()
    }

    /// Written once a boot has opened the workspace's columns.
    #[must_use]
    pub fn columns_opened(&self) -> PathBuf {
        self.runtime.join(iglu_domain::guest::COLUMNS_OPENED)
    }

    /// The directory delivered secrets live in, on the per-boot runtime
    /// directory.
    #[must_use]
    pub fn secrets(&self) -> PathBuf {
        self.runtime.join("iglu/secrets")
    }

    #[must_use]
    pub fn secrets_files(&self) -> PathBuf {
        self.secrets().join("files")
    }

    #[must_use]
    pub fn secrets_env(&self) -> PathBuf {
        self.secrets().join("env.json")
    }

    #[must_use]
    pub fn git_credentials(&self) -> PathBuf {
        self.secrets().join("git-credentials.json")
    }

    /// Written last by `install-secrets`; hosts read it to know what the guest holds.
    #[must_use]
    pub fn secrets_generation(&self) -> PathBuf {
        self.runtime.join(guest::SECRETS_GENERATION)
    }

    #[must_use]
    pub fn status_file(&self) -> PathBuf {
        self.runtime.join(guest::STATUS)
    }

    #[must_use]
    pub fn status_lock(&self) -> PathBuf {
        self.status_file().with_file_name(".lock")
    }

    /// Under the home directory: what iglu recorded about this workspace.
    #[must_use]
    pub fn state(&self) -> PathBuf {
        self.home.join(".local/state/iglu")
    }

    #[must_use]
    pub fn workspace_file(&self) -> PathBuf {
        self.state().join("workspace.json")
    }

    #[must_use]
    pub fn secret_links_file(&self) -> PathBuf {
        self.state().join("secret-links.json")
    }
}
