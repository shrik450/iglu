//! Installing a secrets bundle: files on the tmpfs, links from the home
//! directory, and the generation marker last.
//!
//! Runs as the workspace user, so it can only ever touch what the user can.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use iglu_domain::secret::{HomePath, SecretBundle};
use serde::Deserialize;

use crate::credential::Stored;
use crate::paths;
use crate::status::write_atomic;

/// What hostd hands over. The user field is informational; the tool acts as
/// whoever runs it.
#[derive(Deserialize)]
pub struct InstallRequest {
    pub bundle: SecretBundle,
}

/// The layout for one bundle: which tmpfs file each home path links to.
#[derive(Debug, PartialEq, Eq)]
pub struct Layout {
    /// `(home path, tmpfs file name)`, in bundle order.
    pub links: Vec<(HomePath, String)>,
    /// Links from an earlier delivery that no secret claims any more.
    pub stale: Vec<HomePath>,
}

#[must_use]
pub fn layout(bundle: &SecretBundle, previous: &[HomePath]) -> Layout {
    let links: Vec<(HomePath, String)> = bundle
        .files
        .keys()
        .enumerate()
        .map(|(index, path)| (path.clone(), format!("{index}")))
        .collect();
    let stale = previous
        .iter()
        .filter(|old| !bundle.files.contains_key(*old))
        .cloned()
        .collect();
    Layout { links, stale }
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{0}: {1}")]
    Io(String, #[source] std::io::Error),
    #[error("reading the bundle: {0}")]
    Json(#[from] serde_json::Error),
    #[error("HOME isn't set")]
    NoHome,
}

fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> InstallError {
    let what = what.into();
    move |error| InstallError::Io(what, error)
}

fn write_private(path: &Path, content: &[u8]) -> Result<(), InstallError> {
    write_atomic(path, content).map_err(io(path.display().to_string()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(io(path.display().to_string()))
}

/// Replaces whatever sits at `link` with a symlink to `target`. A real file
/// the user created is kept alongside, never deleted.
fn place_link(link: &Path, target: &Path) -> Result<(), InstallError> {
    match fs::symlink_metadata(link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::remove_file(link).map_err(io(link.display().to_string()))?;
        }
        Ok(_) => {
            let kept = link.with_extension("iglu-replaced");
            fs::rename(link, &kept).map_err(io(link.display().to_string()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io(link.display().to_string())(error)),
    }
    symlink(target, link).map_err(io(link.display().to_string()))
}

/// Removes a link only if it still points into the secrets directory.
fn remove_stale(link: &Path) {
    if let Ok(target) = fs::read_link(link)
        && target.starts_with(paths::SECRETS_DIR)
    {
        let _ = fs::remove_file(link);
    }
}

/// Replaces the delivered secrets with a new bundle: the environment, Git
/// credentials, files and their links, and last the generation hostd reads.
///
/// # Errors
///
/// When the bundle is unreadable or a file can't be written.
pub fn install(incoming: &Path) -> Result<(), InstallError> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or(InstallError::NoHome)?);
    let request: InstallRequest =
        serde_json::from_slice(&fs::read(incoming).map_err(io("the incoming bundle"))?)?;
    let bundle = request.bundle;

    let links_file = home.join(paths::SECRET_LINKS_FILE);
    let previous: Vec<HomePath> = fs::read(&links_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let plan = layout(&bundle, &previous);

    // Start from an empty directory, keeping only the incoming bundle.
    let secrets = Path::new(paths::SECRETS_DIR);
    for entry in fs::read_dir(secrets).map_err(io(paths::SECRETS_DIR))? {
        let entry = entry.map_err(io(paths::SECRETS_DIR))?;
        let path = entry.path();
        if path == incoming {
            continue;
        }
        let result = if entry
            .file_type()
            .map_err(io(path.display().to_string()))?
            .is_dir()
        {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        result.map_err(io(path.display().to_string()))?;
    }
    fs::create_dir(paths::SECRETS_FILES).map_err(io(paths::SECRETS_FILES))?;
    fs::set_permissions(paths::SECRETS_FILES, fs::Permissions::from_mode(0o700))
        .map_err(io(paths::SECRETS_FILES))?;

    let env: BTreeMap<&str, &str> = bundle
        .env
        .iter()
        .map(|(k, v)| (k.as_str(), v.expose()))
        .collect();
    write_private(Path::new(paths::SECRETS_ENV), &serde_json::to_vec(&env)?)?;

    let git: Vec<Stored> = bundle
        .git
        .iter()
        .map(|c| Stored {
            host: c.host.clone(),
            username: c.username.to_string(),
            password: c.password.expose().to_owned(),
        })
        .collect();
    write_private(
        Path::new(paths::GIT_CREDENTIALS),
        &serde_json::to_vec(&git)?,
    )?;

    for old in &plan.stale {
        remove_stale(&home.join(old.as_str()));
    }
    for ((path, file), value) in plan.links.iter().zip(bundle.files.values()) {
        let target = Path::new(paths::SECRETS_FILES).join(file);
        write_private(&target, value.expose().as_bytes())?;
        for parent in path.parents() {
            let dir = home.join(parent);
            if !dir.exists() {
                fs::create_dir(&dir).map_err(io(dir.display().to_string()))?;
                fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
                    .map_err(io(dir.display().to_string()))?;
            }
        }
        place_link(&home.join(path.as_str()), &target)?;
    }

    let state_dir = home.join(paths::STATE_DIR);
    fs::create_dir_all(&state_dir).map_err(io(state_dir.display().to_string()))?;
    let linked: Vec<&HomePath> = plan.links.iter().map(|(path, _)| path).collect();
    write_atomic(&links_file, &serde_json::to_vec(&linked)?)
        .map_err(io(links_file.display().to_string()))?;

    fs::remove_file(incoming).map_err(io("the incoming bundle"))?;
    write_atomic(
        Path::new(paths::SECRETS_GENERATION),
        bundle.generation.get().to_string().as_bytes(),
    )
    .map_err(io(paths::SECRETS_GENERATION))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iglu_domain::lifecycle::SecretsGeneration;
    use iglu_domain::secret::{SecretTarget, SecretValue, bundle};

    fn file(path: &str) -> (SecretTarget, SecretValue) {
        (
            SecretTarget::File {
                path: path.parse().expect("valid path"),
            },
            SecretValue::try_from("x".to_owned()).expect("valid value"),
        )
    }

    #[test]
    fn stale_links_are_the_ones_no_longer_claimed() {
        let bundle =
            bundle(SecretsGeneration::from_u64(2), [file(".a"), file(".b")]).expect("no conflicts");
        let previous: Vec<HomePath> =
            vec![".a".parse().expect("valid"), ".old".parse().expect("valid")];
        let plan = layout(&bundle, &previous);
        assert_eq!(plan.links.len(), 2);
        assert_eq!(plan.stale, vec![".old".parse::<HomePath>().expect("valid")]);
        let names: Vec<_> = plan.links.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(names, ["0", "1"]);
    }
}
