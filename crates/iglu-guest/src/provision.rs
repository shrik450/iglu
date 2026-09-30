//! Cloning a workspace's repository and checking out its branch.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use iglu_domain::repo::{BranchName, RepoUrl};
use serde::{Deserialize, Serialize};

use crate::paths;
use crate::status::write_atomic;

/// What the guest records about its workspace, for tools that need the checkout.
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub checkout: PathBuf,
}

#[derive(Debug)]
pub struct Spec {
    pub repo: RepoUrl,
    pub branch: BranchName,
    pub base: Option<BranchName>,
}

/// How to get onto the branch once the repository is present.
#[derive(Debug, PartialEq, Eq)]
pub enum Checkout {
    /// The branch exists on the remote: track it.
    Track,
    /// A new branch, starting from the given remote-tracking ref.
    Create { from: String },
}

#[must_use]
pub fn checkout(branch_on_remote: bool, base: Option<&BranchName>) -> Checkout {
    if branch_on_remote {
        Checkout::Track
    } else {
        Checkout::Create {
            from: base.map_or_else(|| "origin/HEAD".to_owned(), |b| format!("origin/{b}")),
        }
    }
}

#[must_use]
pub fn switch_args(branch: &BranchName, checkout: &Checkout) -> Vec<String> {
    match checkout {
        Checkout::Track => vec!["switch".into(), "--".into(), branch.to_string()],
        Checkout::Create { from } => vec![
            "switch".into(),
            "--no-track".into(),
            "-c".into(),
            branch.to_string(),
            from.clone(),
        ],
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    #[error("git {0} failed: {1}")]
    Git(String, String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("HOME isn't set")]
    NoHome,
}

fn git(dir: Option<&Path>, args: &[String]) -> Result<std::process::Output, ProvisionError> {
    let mut command = Command::new("git");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "https:ssh")
        .env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new",
        );
    Ok(command.output()?)
}

fn git_ok(dir: Option<&Path>, args: &[String]) -> Result<(), ProvisionError> {
    let output = git(dir, args)?;
    if output.status.success() {
        Ok(())
    } else {
        let what = args.first().cloned().unwrap_or_default();
        Err(ProvisionError::Git(
            what,
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

/// Idempotent: an existing checkout is fetched instead of cloned, and a
/// branch already checked out stays as it is.
///
/// # Errors
///
/// When Git fails or the checkout can't be recorded.
pub fn run(spec: &Spec) -> Result<(), ProvisionError> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or(ProvisionError::NoHome)?);
    let dir = home.join(spec.repo.checkout_dir().as_str());
    if dir.join(".git").exists() {
        git_ok(
            Some(&dir),
            &["fetch".into(), "--prune".into(), "origin".into()],
        )?;
    } else {
        git_ok(
            None,
            &[
                "clone".into(),
                "--origin".into(),
                "origin".into(),
                "--".into(),
                spec.repo.to_string(),
                dir.display().to_string(),
            ],
        )?;
    }

    let current = git(
        Some(&dir),
        &[
            "symbolic-ref".into(),
            "--quiet".into(),
            "--short".into(),
            "HEAD".into(),
        ],
    )?;
    let on_branch = current.status.success()
        && String::from_utf8_lossy(&current.stdout).trim() == spec.branch.as_str();
    if !on_branch {
        let remote_ref = format!("refs/remotes/origin/{}", spec.branch);
        let exists = git(
            Some(&dir),
            &[
                "show-ref".into(),
                "--verify".into(),
                "--quiet".into(),
                remote_ref,
            ],
        )?
        .status
        .success();
        let local_ref = format!("refs/heads/{}", spec.branch);
        let local = git(
            Some(&dir),
            &[
                "show-ref".into(),
                "--verify".into(),
                "--quiet".into(),
                local_ref,
            ],
        )?
        .status
        .success();
        let plan = if local {
            Checkout::Track
        } else {
            checkout(exists, spec.base.as_ref())
        };
        git_ok(Some(&dir), &switch_args(&spec.branch, &plan))?;
    }

    let state = home.join(paths::STATE_DIR);
    fs::create_dir_all(&state)?;
    let record =
        serde_json::to_vec(&WorkspaceRecord { checkout: dir }).map_err(std::io::Error::other)?;
    write_atomic(&home.join(paths::WORKSPACE_FILE), &record)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str) -> BranchName {
        name.parse().expect("valid branch")
    }

    #[test]
    fn existing_remote_branches_are_tracked() {
        assert_eq!(checkout(true, Some(&branch("main"))), Checkout::Track);
        assert_eq!(
            switch_args(&branch("feat"), &Checkout::Track),
            ["switch", "--", "feat"]
        );
    }

    #[test]
    fn new_branches_start_from_the_base_or_the_default_branch() {
        assert_eq!(
            checkout(false, None),
            Checkout::Create {
                from: "origin/HEAD".into()
            }
        );
        assert_eq!(
            checkout(false, Some(&branch("develop"))),
            Checkout::Create {
                from: "origin/develop".into()
            }
        );
        assert_eq!(
            switch_args(
                &branch("feat"),
                &Checkout::Create {
                    from: "origin/HEAD".into()
                }
            ),
            ["switch", "--no-track", "-c", "feat", "origin/HEAD"]
        );
    }
}
