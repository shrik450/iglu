//! Projects: what a person works on, and how its workspaces start.
//!
//! Every workspace belongs to one project. A project with a repository gives
//! its workspaces a checkout; one without, such as each person's built-in
//! `general`, gives them only a home directory.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::column::{
    BROWSER_DEBUG_PORT, BROWSER_PORT_PRIVATE, ColumnKind, ColumnSpec, ColumnTemplate, ColumnWidth,
    name_templates,
};
use crate::label::{ProjectName, WorkspaceName};
use crate::parse::serde_text;
use crate::port::GuestPort;
use crate::repo::{BranchName, Checkout, RepoUrl};

/// Whether iglu made the project or the person did. The built-in one can't
/// be deleted, so everyone always has somewhere to start a workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "snake_case"))]
pub enum Origin {
    Builtin,
    Added,
}

serde_text!(Origin, "project origin", "unknown");

/// The columns a project's workspaces open with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(try_from = "Vec<ColumnTemplate>", into = "Vec<ColumnTemplate>")]
pub struct Opening(Vec<ColumnTemplate>);

impl Opening {
    pub const MAX_COLUMNS: usize = 16;

    /// One shell, which is what a new project opens with.
    #[must_use]
    pub fn shell() -> Self {
        Self(vec![ColumnTemplate {
            name: None,
            kind: ColumnKind::Shell,
            width: ColumnWidth::Half,
        }])
    }

    #[must_use]
    pub fn templates(&self) -> &[ColumnTemplate] {
        &self.0
    }

    /// The named columns a new workspace gets.
    #[must_use]
    pub fn columns(&self) -> Vec<ColumnSpec> {
        name_templates(&self.0)
    }
}

impl TryFrom<Vec<ColumnTemplate>> for Opening {
    type Error = ParseError;

    fn try_from(templates: Vec<ColumnTemplate>) -> Result<Self, Self::Error> {
        if templates.len() > Self::MAX_COLUMNS {
            return Err(ParseError::new(
                "opening columns",
                "may have at most 16 columns",
            ));
        }
        Ok(Self(templates))
    }
}

impl From<Opening> for Vec<ColumnTemplate> {
    fn from(opening: Opening) -> Self {
        opening.0
    }
}

/// Ports a project's workspaces publish as previews when they're created.
/// At most 16, each once, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(try_from = "Vec<GuestPort>", into = "Vec<GuestPort>")]
pub struct PreviewPorts(#[cfg_attr(feature = "ts", ts(type = "Array<number>"))] Vec<GuestPort>);

impl PreviewPorts {
    pub const MAX: usize = 16;

    #[must_use]
    pub fn ports(&self) -> &[GuestPort] {
        &self.0
    }
}

impl TryFrom<Vec<GuestPort>> for PreviewPorts {
    type Error = ParseError;

    fn try_from(mut ports: Vec<GuestPort>) -> Result<Self, Self::Error> {
        ports.sort_unstable();
        ports.dedup();
        if ports.len() > Self::MAX {
            return Err(ParseError::new(
                "preview ports",
                "may have at most 16 ports",
            ));
        }
        if ports.iter().any(|p| p.get() == BROWSER_DEBUG_PORT) {
            return Err(ParseError::new("preview ports", BROWSER_PORT_PRIVATE));
        }
        Ok(Self(ports))
    }
}

impl From<PreviewPorts> for Vec<GuestPort> {
    fn from(ports: PreviewPorts) -> Self {
        ports.0
    }
}

/// The built-in project's name, until its owner renames it.
///
/// # Panics
///
/// Never: the name is a valid project name.
#[must_use]
pub fn general() -> ProjectName {
    "general".parse().expect("'general' is a project name")
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckoutError {
    #[error("the project has no repository, so its workspaces have no branch")]
    NoRepository,
}

/// The checkout a new workspace in a project gets: on its own branch, named
/// after the workspace unless one is given.
///
/// # Errors
///
/// When a branch is asked for in a project without a repository.
///
/// # Panics
///
/// Never: a workspace name is always a valid branch name.
pub fn checkout(
    repo: Option<&RepoUrl>,
    workspace: &WorkspaceName,
    branch: Option<BranchName>,
    base: Option<BranchName>,
) -> Result<Option<Checkout>, CheckoutError> {
    match repo {
        None if branch.is_some() || base.is_some() => Err(CheckoutError::NoRepository),
        None => Ok(None),
        Some(repo) => Ok(Some(Checkout {
            repo: repo.clone(),
            branch: branch.unwrap_or_else(|| {
                workspace
                    .as_str()
                    .parse()
                    .expect("workspace names are valid branch names")
            }),
            base,
        })),
    }
}

/// A name for a project made from a repository: the repository's directory
/// name as a label, numbered when it's taken.
///
/// # Panics
///
/// Never: the stem is a valid label, and some numbered name is always free.
#[must_use]
pub fn suggest_name<'a>(
    repo: &RepoUrl,
    taken: impl IntoIterator<Item = &'a ProjectName>,
) -> ProjectName {
    let mut stem = String::new();
    for c in repo.checkout_dir().as_str().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || (c.is_ascii_digit() && !stem.is_empty()) {
            stem.push(c);
        } else if !stem.is_empty() && !stem.ends_with('-') {
            stem.push('-');
        }
    }
    stem.truncate(50);
    let stem = stem
        .trim_end_matches('-')
        .parse()
        .unwrap_or_else(|_| "project".parse().expect("'project' is a project name"));
    unique(&stem, taken)
}

/// `name`, or the first free numbered version of it.
///
/// # Panics
///
/// Never: some numbered name is always free.
#[must_use]
pub fn unique<'a>(
    name: &ProjectName,
    taken: impl IntoIterator<Item = &'a ProjectName>,
) -> ProjectName {
    let taken: HashSet<&str> = taken.into_iter().map(ProjectName::as_str).collect();
    let stem: String = name.as_str().chars().take(50).collect();
    std::iter::once(name.clone())
        .chain(
            (2..=u32::MAX)
                .filter_map(|n| format!("{}-{n}", stem.trim_end_matches('-')).parse().ok()),
        )
        .find(|candidate: &ProjectName| !taken.contains(candidate.as_str()))
        .expect("some numbered name is always free")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(url: &str) -> RepoUrl {
        url.parse().expect("a valid repository")
    }

    fn name(n: &str) -> ProjectName {
        n.parse().expect("a valid project name")
    }

    #[test]
    fn preview_ports_are_unique_ordered_and_bounded() {
        let port = |p: u16| GuestPort::try_from(p).expect("a port");
        let ports =
            PreviewPorts::try_from(vec![port(5173), port(3000), port(5173)]).expect("ports");
        assert_eq!(ports.ports(), [port(3000), port(5173)]);
        assert!(PreviewPorts::try_from((1..=17).map(port).collect::<Vec<_>>()).is_err());
        // Anyone with the link would drive the workspace's browser.
        assert!(PreviewPorts::try_from(vec![port(3000), port(BROWSER_DEBUG_PORT)]).is_err());
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        assert_eq!(unique(&general(), []), general());
        assert_eq!(unique(&general(), &[general()]), name("general-2"));
    }

    #[test]
    fn a_workspace_branches_after_its_name_unless_told() {
        let ws: WorkspaceName = "fix-login".parse().expect("a workspace name");
        let app = repo("https://github.com/you/app.git");
        let made = checkout(Some(&app), &ws, None, None).expect("a checkout");
        assert_eq!(
            made.map(|c| c.branch.as_str().to_owned()),
            Some("fix-login".into())
        );
        let main: BranchName = "main".parse().expect("a branch");
        let told = checkout(Some(&app), &ws, Some(main.clone()), Some(main))
            .expect("a checkout")
            .expect("with a repository");
        assert_eq!(
            (
                told.branch.as_str(),
                told.base.as_ref().map(BranchName::as_str)
            ),
            ("main", Some("main"))
        );
    }

    #[test]
    fn a_project_without_a_repository_has_no_branches() {
        let ws: WorkspaceName = "notes".parse().expect("a workspace name");
        assert_eq!(checkout(None, &ws, None, None), Ok(None));
        let branch: BranchName = "main".parse().expect("a branch");
        assert_eq!(
            checkout(None, &ws, Some(branch.clone()), None),
            Err(CheckoutError::NoRepository)
        );
        assert_eq!(
            checkout(None, &ws, None, Some(branch)),
            Err(CheckoutError::NoRepository)
        );
    }

    #[test]
    fn names_come_from_the_repository_and_stay_unique() {
        assert_eq!(
            suggest_name(&repo("https://github.com/you/iglu.git"), []),
            name("iglu")
        );
        assert_eq!(
            suggest_name(&repo("git@github.com:acme/My_Shop.git"), []),
            name("my-shop")
        );
        let taken = [name("iglu"), name("iglu-2")];
        assert_eq!(
            suggest_name(&repo("https://github.com/you/iglu.git"), &taken),
            name("iglu-3")
        );
        assert_eq!(
            suggest_name(&repo("https://github.com/you/2048.git"), []),
            name("project")
        );
    }

    #[test]
    fn openings_are_bounded_and_named() {
        let many = vec![Opening::shell().templates()[0].clone(); Opening::MAX_COLUMNS + 1];
        assert!(Opening::try_from(many).is_err());
        let two = Opening::try_from(vec![Opening::shell().templates()[0].clone(); 2])
            .expect("two shells");
        let names: Vec<String> = two
            .columns()
            .into_iter()
            .map(|c| c.name.as_str().to_owned())
            .collect();
        assert_eq!(names, ["shell", "shell-2"]);
    }
}
