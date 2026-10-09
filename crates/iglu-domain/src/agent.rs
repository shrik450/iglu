//! Agents an environment declares: what starts one, how it takes a prompt,
//! and how it reports whether it needs the person.
//!
//! Environments declare agents in Nix (`iglu.agents.<name>`), so the binary
//! and its attention hooks ship in the same image. hostd reads them from the
//! image's manifest when it builds the environment.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseError;
use crate::column::{Arg, Argv};
use crate::label::AgentName;
use crate::parse::text_type;

/// How an agent takes the prompt a workspace starts it with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "kebab-case"))]
pub enum PromptStyle {
    /// As one more argument: `claude "<prompt>"`.
    Argument,
    /// It doesn't take one.
    None,
}

/// How an agent reports its attention state to `iglu-status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename_all = "kebab-case"))]
pub enum AttentionAdapter {
    /// Claude Code's hooks, which the workspace module installs: working,
    /// waiting and done.
    ClaudeHooks,
    /// Whatever the agent or a wrapper reports with `iglu-status set`.
    StatusCommand,
}

/// The most agents an environment can declare.
pub const MAX_AGENTS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AgentSpec {
    pub name: AgentName,
    pub command: Argv,
    pub prompt: PromptStyle,
    pub attention: AttentionAdapter,
}

/// What a workspace asks its first agent to do. Passed as one argument,
/// never through a shell.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct Prompt(Arg);

impl Prompt {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl FromStr for Prompt {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(ParseError::new("prompt", "is empty"));
        }
        s.parse::<Arg>()
            .map(Self)
            .map_err(|_| ParseError::new("prompt", "must be at most 16 KiB without NUL"))
    }
}

impl fmt::Display for Prompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

text_type!(Prompt);

/// Why an agent can't run as asked.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AgentError {
    #[error("the environment has no agent called {0}")]
    Unknown(AgentName),
    #[error("{0} doesn't take a prompt")]
    TakesNoPrompt(AgentName),
    #[error("{0}'s command is too long with the prompt")]
    TooLong(AgentName),
}

/// Finds an agent by name among the ones an environment declares.
///
/// # Errors
///
/// When the environment doesn't declare it.
pub fn find<'a>(agents: &'a [AgentSpec], name: &AgentName) -> Result<&'a AgentSpec, AgentError> {
    agents
        .iter()
        .find(|agent| &agent.name == name)
        .ok_or_else(|| AgentError::Unknown(name.clone()))
}

/// The command that starts an agent, with its prompt if it has one.
///
/// # Errors
///
/// When there's a prompt the agent can't take.
pub fn command(agent: &AgentSpec, prompt: Option<&Prompt>) -> Result<Argv, AgentError> {
    match (prompt, agent.prompt) {
        (None, _) => Ok(agent.command.clone()),
        (Some(prompt), PromptStyle::Argument) => agent
            .command
            .with(prompt.0.clone())
            .map_err(|_| AgentError::TooLong(agent.name.clone())),
        (Some(_), PromptStyle::None) => Err(AgentError::TakesNoPrompt(agent.name.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> AgentSpec {
        AgentSpec {
            name: "claude".parse().expect("name"),
            command: serde_json::from_str(r#"["claude"]"#).expect("argv"),
            prompt: PromptStyle::Argument,
            attention: AttentionAdapter::ClaudeHooks,
        }
    }

    #[test]
    fn a_prompt_is_one_more_argument_exactly() {
        let prompt: Prompt = "fix the 'flaky' test;\nthen push $BRANCH"
            .parse()
            .expect("prompt");
        let started = command(&claude(), Some(&prompt)).expect("argv");
        let words: Vec<&str> = started.args().iter().map(Arg::as_str).collect();
        assert_eq!(
            words,
            ["claude", "fix the 'flaky' test;\nthen push $BRANCH"]
        );
        assert_eq!(command(&claude(), None).expect("argv").args().len(), 1);
    }

    #[test]
    fn agents_that_take_no_prompt_refuse_one() {
        let silent = AgentSpec {
            prompt: PromptStyle::None,
            ..claude()
        };
        let prompt: Prompt = "hello".parse().expect("prompt");
        assert!(matches!(
            command(&silent, Some(&prompt)),
            Err(AgentError::TakesNoPrompt(_))
        ));
    }

    #[test]
    fn unknown_agents_and_empty_prompts_are_refused() {
        let agents = [claude()];
        assert!(find(&agents, &"codex".parse().expect("name")).is_err());
        assert!(find(&agents, &"claude".parse().expect("name")).is_ok());
        assert!("   ".parse::<Prompt>().is_err());
    }
}
