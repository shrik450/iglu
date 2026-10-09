//! Installing a secrets bundle: files on the runtime directory, links from
//! the home directory, and the generation marker last.
//!
//! Runs as the workspace user, so it can only ever touch what the user can.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use iglu_domain::secret::{HomePath, SecretBundle};
use serde::Deserialize;

use crate::credential::Stored;
use crate::paths::Dirs;
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
    /// Says where the bundle is malformed, never what it holds: serde's
    /// messages quote the values they reject, and these are secrets.
    #[error("reading the bundle: {0}")]
    Bundle(String),
    #[error("writing the secrets: {0}")]
    Json(#[from] serde_json::Error),
}

fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> InstallError {
    let what = what.into();
    move |error| InstallError::Io(what, error)
}

fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> InstallError {
    io(path.display().to_string())
}

fn write_private(path: &Path, content: &[u8]) -> Result<(), InstallError> {
    write_atomic(path, content).map_err(io_at(path))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io_at(path))
}

fn create_private_dir(dir: &Path) -> Result<(), InstallError> {
    fs::create_dir(dir).map_err(io_at(dir))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io_at(dir))
}

/// Replaces whatever sits at `link` with a symlink to `target`. A real file
/// the user created is kept alongside, never deleted.
fn place_link(link: &Path, target: &Path) -> Result<(), InstallError> {
    match fs::symlink_metadata(link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::remove_file(link).map_err(io_at(link))?;
        }
        Ok(_) => {
            let kept = link.with_extension("iglu-replaced");
            fs::rename(link, &kept).map_err(io_at(link))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_at(link)(error)),
    }
    symlink(target, link).map_err(io_at(link))
}

/// Removes a link only if it still points into the secrets directory.
fn remove_stale(link: &Path, secrets: &Path) {
    if let Ok(target) = fs::read_link(link)
        && target.starts_with(secrets)
    {
        let _ = fs::remove_file(link);
    }
}

/// Empties the secrets directory, creating it on the first delivery of a boot.
fn empty_secrets_dir(secrets: &Path) -> Result<(), InstallError> {
    let entries = match fs::read_dir(secrets) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            for dir in [secrets.parent(), Some(secrets)].into_iter().flatten() {
                if !dir.exists() {
                    create_private_dir(dir)?;
                }
            }
            return Ok(());
        }
        Err(error) => return Err(io_at(secrets)(error)),
    };
    for entry in entries {
        let path = entry.map_err(io_at(secrets))?.path();
        let result = if fs::symlink_metadata(&path).map_err(io_at(&path))?.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        result.map_err(io_at(&path))?;
    }
    Ok(())
}

/// Where and how a bundle failed to parse, without the value at fault.
#[must_use]
pub fn describe(error: &serde_json::Error) -> String {
    let what = match error.classify() {
        serde_json::error::Category::Io => "unreadable input",
        serde_json::error::Category::Syntax => "malformed JSON",
        serde_json::error::Category::Data => "unexpected content",
        serde_json::error::Category::Eof => "input ended early",
    };
    format!("{what} at line {} column {}", error.line(), error.column())
}

/// Replaces the delivered secrets with a new bundle: the environment, Git
/// credentials, files and their links, and last the generation hosts read.
///
/// # Errors
///
/// When the bundle is unreadable or a file can't be written.
pub fn install(dirs: &Dirs, request: &[u8]) -> Result<(), InstallError> {
    let bundle = serde_json::from_slice::<InstallRequest>(request)
        .map_err(|e| InstallError::Bundle(describe(&e)))?
        .bundle;
    let home = dirs.home();

    let links_file = dirs.secret_links_file();
    let previous: Vec<HomePath> = fs::read(&links_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let plan = layout(&bundle, &previous);

    let secrets = dirs.secrets();
    empty_secrets_dir(&secrets)?;
    let files = dirs.secrets_files();
    create_private_dir(&files)?;

    let env: BTreeMap<&str, &str> = bundle
        .env
        .iter()
        .map(|(k, v)| (k.as_str(), v.expose()))
        .collect();
    write_private(&dirs.secrets_env(), &serde_json::to_vec(&env)?)?;

    let git: Vec<Stored> = bundle
        .git
        .iter()
        .map(|c| Stored {
            host: c.host.clone(),
            username: c.username.clone(),
            password: c.password.clone(),
        })
        .collect();
    write_private(&dirs.git_credentials(), &serde_json::to_vec(&git)?)?;

    for old in &plan.stale {
        remove_stale(&home.join(old.as_str()), &secrets);
    }
    for ((path, file), value) in plan.links.iter().zip(bundle.files.values()) {
        let target = files.join(file);
        write_private(&target, value.expose().as_bytes())?;
        for parent in path.parents() {
            let dir = home.join(parent);
            if !dir.exists() {
                create_private_dir(&dir)?;
            }
        }
        place_link(&home.join(path.as_str()), &target)?;
    }

    let state_dir = dirs.state();
    fs::create_dir_all(&state_dir).map_err(io_at(&state_dir))?;
    let linked: Vec<&HomePath> = plan.links.iter().map(|(path, _)| path).collect();
    write_atomic(&links_file, &serde_json::to_vec(&linked)?).map_err(io_at(&links_file))?;

    let generation = dirs.secrets_generation();
    write_atomic(&generation, bundle.generation.get().to_string().as_bytes())
        .map_err(io_at(&generation))
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
    fn a_malformed_bundle_is_described_without_its_values() {
        let request =
            br#"{"bundle": {"generation": "sk-very-secret", "env": {}, "files": {}, "git": []}}"#;
        let Err(error) = serde_json::from_slice::<InstallRequest>(request) else {
            panic!("a string generation is malformed");
        };
        assert!(
            error.to_string().contains("sk-very-secret"),
            "serde quotes values"
        );
        let described = describe(&error);
        assert!(!described.contains("sk-very-secret"), "{described}");
        assert!(described.contains("line 1"), "{described}");
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
