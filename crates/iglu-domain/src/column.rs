//! A workspace's columns: the terminal sessions it opens, what each runs,
//! and how wide each shows in the console's strip.
//!
//! iglud keeps the columns as intent. The guest's sessions are what exists;
//! a column whose session ended stays until someone closes it.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::label::AgentName;
use crate::parse::text_type;
use crate::terminal::SessionName;

/// One argument of a command. Passed to the program as it is: never joined
/// into a string or read by a shell.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct Arg(String);

impl Arg {
    pub const MAX_BYTES: usize = 16 * 1024;

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Arg {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > Self::MAX_BYTES {
            return Err(ParseError::new("argument", "must be at most 16 KiB"));
        }
        if s.contains('\0') {
            return Err(ParseError::new("argument", "may not contain NUL"));
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

text_type!(Arg);

/// A program and its arguments.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "Array<string>"))]
#[serde(try_from = "Vec<Arg>", into = "Vec<Arg>")]
pub struct Argv(Vec<Arg>);

impl Argv {
    pub const MAX_ARGS: usize = 64;

    /// # Panics
    ///
    /// Never: an `Argv` can only be made with a program.
    #[must_use]
    pub fn program(&self) -> &Arg {
        self.0.first().expect("an argv always has a program")
    }

    #[must_use]
    pub fn args(&self) -> &[Arg] {
        &self.0
    }

    /// This command with one more argument at the end.
    ///
    /// # Errors
    ///
    /// When that would make it too long.
    pub fn with(&self, arg: Arg) -> Result<Self, ParseError> {
        let mut args = self.0.clone();
        args.push(arg);
        Self::try_from(args)
    }
}

impl TryFrom<Vec<Arg>> for Argv {
    type Error = ParseError;

    fn try_from(args: Vec<Arg>) -> Result<Self, Self::Error> {
        if args.is_empty() {
            return Err(ParseError::new("command", "needs a program"));
        }
        if args.len() > Self::MAX_ARGS {
            return Err(ParseError::new("command", "has more than 64 arguments"));
        }
        if args[0].as_str().is_empty() {
            return Err(ParseError::new("command", "needs a program"));
        }
        Ok(Self(args))
    }
}

impl From<Argv> for Vec<Arg> {
    fn from(argv: Argv) -> Self {
        argv.0
    }
}

/// How much of the strip a column takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "kebab-case"))]
pub enum ColumnWidth {
    Third,
    Half,
    TwoThirds,
    Full,
}

impl ColumnWidth {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Third => "third",
            Self::Half => "half",
            Self::TwoThirds => "two-thirds",
            Self::Full => "full",
        }
    }
}

impl FromStr for ColumnWidth {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "third" => Ok(Self::Third),
            "half" => Ok(Self::Half),
            "two-thirds" => Ok(Self::TwoThirds),
            "full" => Ok(Self::Full),
            _ => Err(ParseError::new(
                "column width",
                "expected third, half, two-thirds or full",
            )),
        }
    }
}

/// What a column runs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(tag = "kind", rename_all = "snake_case")
)]
pub enum ColumnKind {
    /// The workspace user's login shell.
    Shell,
    /// One of the environment's agents.
    Agent { agent: AgentName },
    /// A long-running command, such as a dev server.
    Server { command: Argv },
}

impl ColumnKind {
    /// What a column of this kind is called when nobody names it.
    #[must_use]
    pub fn default_name(&self) -> &str {
        match self {
            Self::Shell => "shell",
            Self::Agent { agent } => agent.as_str(),
            Self::Server { .. } => "server",
        }
    }
}

/// A column as the person set it up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ColumnSpec {
    pub name: SessionName,
    pub kind: ColumnKind,
    pub width: ColumnWidth,
}

/// A column in a project's opening layout, before it has a name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ColumnTemplate {
    /// Defaults to the kind's own name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<SessionName>,
    pub kind: ColumnKind,
    pub width: ColumnWidth,
}

/// Whether a column's session is there, compared with what was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "snake_case"))]
pub enum ColumnState {
    /// Asked for, and its session is open.
    Open,
    /// Asked for, but its session ended: its program exited, or it was
    /// closed from inside. Restart opens it again.
    Ended,
    /// A session nobody asked iglu for, such as one opened by hand inside
    /// the workspace. It shows as a shell column at the end.
    Adopted,
}

/// Puts what was asked for next to what's open: every asked-for column in
/// order, then the open sessions nobody asked for, in name order. Each comes
/// with its attached-client count.
#[must_use]
pub fn join(
    asked: &[SessionName],
    open: &[(SessionName, u32)],
) -> Vec<(SessionName, ColumnState, u32)> {
    let clients = |name: &SessionName| open.iter().find(|(n, _)| n == name).map(|(_, c)| *c);
    let mut joined: Vec<(SessionName, ColumnState, u32)> = asked
        .iter()
        .map(|name| match clients(name) {
            Some(count) => (name.clone(), ColumnState::Open, count),
            None => (name.clone(), ColumnState::Ended, 0),
        })
        .collect();
    let mut adopted: Vec<&(SessionName, u32)> = open
        .iter()
        .filter(|(name, _)| !asked.contains(name))
        .collect();
    adopted.sort_by(|a, b| a.0.cmp(&b.0));
    joined.extend(
        adopted
            .into_iter()
            .map(|(name, count)| (name.clone(), ColumnState::Adopted, *count)),
    );
    joined
}

/// A layout that doesn't name each column exactly once.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a layout names every column once, no more and no fewer")]
pub struct LayoutMismatch;

/// Reorders and resizes `columns` to `layout`.
///
/// # Errors
///
/// When the layout leaves a column out, names one twice, or names one that
/// doesn't exist.
pub fn arrange(
    columns: &[ColumnSpec],
    layout: &[(SessionName, ColumnWidth)],
) -> Result<Vec<ColumnSpec>, LayoutMismatch> {
    if layout.len() != columns.len() {
        return Err(LayoutMismatch);
    }
    let mut seen = std::collections::HashSet::new();
    layout
        .iter()
        .map(|(name, width)| {
            if !seen.insert(name) {
                return Err(LayoutMismatch);
            }
            columns
                .iter()
                .find(|column| &column.name == name)
                .map(|column| ColumnSpec {
                    width: *width,
                    ..column.clone()
                })
                .ok_or(LayoutMismatch)
        })
        .collect()
}

/// The first of `base`, `base-2`, `base-3`… that isn't taken.
///
/// # Panics
///
/// Never: `base` is a session name or a valid prefix of one, and only the
/// suffixes are tried.
#[must_use]
pub fn free_name<'a>(base: &str, taken: impl IntoIterator<Item = &'a SessionName>) -> SessionName {
    let taken: std::collections::HashSet<&str> =
        taken.into_iter().map(SessionName::as_str).collect();
    let stem: String = base.chars().take(SessionName::MAX_LEN - 4).collect();
    std::iter::once(stem.clone())
        .chain((2..=u32::MAX).map(|n| format!("{stem}-{n}")))
        .filter_map(|candidate| candidate.parse::<SessionName>().ok())
        .find(|candidate| !taken.contains(candidate.as_str()))
        .expect("some numbered name is always free")
}

/// Names a project's templates for a new workspace, in order.
#[must_use]
pub fn name_templates(templates: &[ColumnTemplate]) -> Vec<ColumnSpec> {
    let mut specs: Vec<ColumnSpec> = Vec::with_capacity(templates.len());
    for template in templates {
        let base = template
            .name
            .as_ref()
            .map_or_else(|| template.kind.default_name(), SessionName::as_str);
        let name = free_name(base, specs.iter().map(|spec| &spec.name));
        specs.push(ColumnSpec {
            name,
            kind: template.kind.clone(),
            width: template.width,
        });
    }
    specs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Result<Argv, ParseError> {
        Argv::try_from(
            args.iter()
                .map(|a| a.parse::<Arg>())
                .collect::<Result<Vec<_>, _>>()?,
        )
    }

    #[test]
    fn arguments_are_data_with_bounds() {
        let tricky = "line one\nline 'two' $HOME; rm -rf / \\ ✓";
        assert_eq!(
            tricky.parse::<Arg>().map(|a| a.to_string()).as_deref(),
            Ok(tricky)
        );
        assert!("nul\0".parse::<Arg>().is_err());
        assert!("x".repeat(Arg::MAX_BYTES + 1).parse::<Arg>().is_err());
        assert!(argv(&[]).is_err());
        assert!(argv(&[""]).is_err());
        assert!(argv(&["x"; 65]).is_err());
        let ok = argv(&["just", "dev"]).expect("a command");
        assert_eq!(ok.program().as_str(), "just");
        let parsed: Argv = serde_json::from_str(r#"["pnpm","dev"]"#).expect("parses");
        assert_eq!(parsed.args().len(), 2);
        assert!(serde_json::from_str::<Argv>("[]").is_err());
    }

    #[test]
    fn kinds_and_widths_round_trip_as_the_console_writes_them() {
        let kind: ColumnKind =
            serde_json::from_str(r#"{"kind":"server","command":["just","dev"]}"#).expect("parses");
        assert!(matches!(kind, ColumnKind::Server { .. }));
        let width: ColumnWidth = serde_json::from_str(r#""two-thirds""#).expect("parses");
        assert_eq!(width, ColumnWidth::TwoThirds);
        assert_eq!(
            "two-thirds".parse::<ColumnWidth>(),
            Ok(ColumnWidth::TwoThirds)
        );
        assert!(
            serde_json::from_str::<ColumnKind>(r#"{"kind":"agent","agent":"Bad Name"}"#).is_err()
        );
    }

    #[test]
    fn names_count_up_from_the_base() {
        let taken: Vec<SessionName> = ["claude", "claude-2", "shell"]
            .iter()
            .map(|n| n.parse().expect("name"))
            .collect();
        assert_eq!(free_name("claude", &taken).as_str(), "claude-3");
        assert_eq!(free_name("dev", &taken).as_str(), "dev");
        let long = "a".repeat(40);
        assert!(free_name(&long, &taken).as_str().len() <= SessionName::MAX_LEN);
    }

    fn spec(name: &str) -> ColumnSpec {
        ColumnSpec {
            name: name.parse().expect("name"),
            kind: ColumnKind::Shell,
            width: ColumnWidth::Half,
        }
    }

    #[test]
    fn joining_marks_ended_and_adopted_sessions() {
        let n = |s: &str| -> SessionName { s.parse().expect("name") };
        let joined = join(
            &[n("claude"), n("dev")],
            &[(n("tmux"), 1), (n("claude"), 2), (n("a-hand"), 0)],
        );
        let states: Vec<(&str, ColumnState, u32)> = joined
            .iter()
            .map(|(name, state, c)| (name.as_str(), *state, *c))
            .collect();
        assert_eq!(
            states,
            [
                ("claude", ColumnState::Open, 2),
                ("dev", ColumnState::Ended, 0),
                ("a-hand", ColumnState::Adopted, 0),
                ("tmux", ColumnState::Adopted, 1),
            ]
        );
    }

    #[test]
    fn a_layout_must_name_every_column_once() {
        let columns = [spec("a"), spec("b")];
        let n = |s: &str| -> SessionName { s.parse().expect("name") };
        let arranged = arrange(
            &columns,
            &[(n("b"), ColumnWidth::Full), (n("a"), ColumnWidth::Third)],
        )
        .expect("a valid layout");
        assert_eq!(
            arranged.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(arranged[0].width, ColumnWidth::Full);
        assert_eq!(
            arrange(&columns, &[(n("a"), ColumnWidth::Half)]),
            Err(LayoutMismatch)
        );
        assert_eq!(
            arrange(
                &columns,
                &[(n("a"), ColumnWidth::Half), (n("a"), ColumnWidth::Half)]
            ),
            Err(LayoutMismatch)
        );
        assert_eq!(
            arrange(
                &columns,
                &[(n("a"), ColumnWidth::Half), (n("c"), ColumnWidth::Half)]
            ),
            Err(LayoutMismatch)
        );
    }

    #[test]
    fn templates_get_distinct_names_in_order() {
        let claude: AgentName = "claude".parse().expect("agent");
        let templates = vec![
            ColumnTemplate {
                name: None,
                kind: ColumnKind::Agent {
                    agent: claude.clone(),
                },
                width: ColumnWidth::TwoThirds,
            },
            ColumnTemplate {
                name: Some("dev".parse().expect("name")),
                kind: ColumnKind::Server {
                    command: argv(&["just", "dev"]).expect("argv"),
                },
                width: ColumnWidth::Third,
            },
            ColumnTemplate {
                name: None,
                kind: ColumnKind::Agent { agent: claude },
                width: ColumnWidth::Half,
            },
            ColumnTemplate {
                name: None,
                kind: ColumnKind::Shell,
                width: ColumnWidth::Half,
            },
        ];
        let names: Vec<String> = name_templates(&templates)
            .into_iter()
            .map(|s| s.name.to_string())
            .collect();
        assert_eq!(names, ["claude", "dev", "claude-2", "shell"]);
    }
}
