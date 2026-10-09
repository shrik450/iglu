//! Where the workspace's checkout stands, from `git status`.

use iglu_domain::guest::GitReport;

use crate::paths::Dirs;
use crate::provision::WorkspaceRecord;

/// Reads porcelain v2 output. Unknown lines are ignored.
#[must_use]
pub fn parse(output: &str) -> GitReport {
    let mut status = GitReport::default();
    for line in output.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            status.branch = (head != "(detached)").then(|| head.to_owned());
        } else if let Some(upstream) = line.strip_prefix("# branch.upstream ") {
            status.upstream = Some(upstream.to_owned());
        } else if let Some(counts) = line.strip_prefix("# branch.ab ") {
            let mut parts = counts.split_whitespace();
            status.ahead = parts
                .next()
                .and_then(|a| a.strip_prefix('+'))
                .and_then(|a| a.parse().ok())
                .unwrap_or(0);
            status.behind = parts
                .next()
                .and_then(|b| b.strip_prefix('-'))
                .and_then(|b| b.parse().ok())
                .unwrap_or(0);
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            status.changed += 1;
        } else if line.starts_with("u ") {
            status.conflicted += 1;
        } else if line.starts_with("? ") {
            status.untracked += 1;
        }
    }
    status
}

/// The checkout's status, or `None` for a workspace without a repository
/// or when Git can't say.
#[must_use]
pub fn state(dirs: &Dirs) -> Option<GitReport> {
    let record: WorkspaceRecord =
        serde_json::from_slice(&std::fs::read(dirs.workspace_file()).ok()?).ok()?;
    // Only the workspace's own clone: a home directory without one may sit
    // inside some other repository, as local workspaces' homes do.
    if !record.checkout.join(".git").exists() {
        return None;
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&record.checkout)
        .args(["status", "--porcelain=v2", "--branch"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_v2_counts() {
        let output = "# branch.oid 1234\n# branch.head fix-login\n# branch.upstream origin/fix-login\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 a b src/main.rs\n2 R. N... 100644 100644 100644 a b R100 new.rs\told.rs\nu UU N... 1 2 3 4 a b c conflict.rs\n? notes.txt\n";
        assert_eq!(
            parse(output),
            GitReport {
                branch: Some("fix-login".into()),
                upstream: Some("origin/fix-login".into()),
                ahead: 2,
                behind: 1,
                changed: 2,
                untracked: 1,
                conflicted: 1,
            }
        );
        assert_eq!(parse("# branch.head (detached)\n").branch, None);
    }
}
