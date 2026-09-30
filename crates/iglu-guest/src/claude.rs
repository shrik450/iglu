//! Claude Code hook events, as Claude Code writes them to a hook's stdin,
//! and what each one says about whether the session needs its user.

use iglu_domain::attention::{AttentionState, Summary};
use serde::Deserialize;

/// The parts of a hook event iglu uses. Claude Code sends more fields;
/// they're ignored, and events iglu doesn't know are [`HookEvent::Other`].
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "hook_event_name")]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit {
        #[serde(default)]
        prompt: String,
    },
    PreToolUse(ToolUse),
    PostToolUse(ToolUse),
    Notification {
        #[serde(default)]
        message: String,
        #[serde(default)]
        notification_type: NotificationType,
    },
    Stop {
        #[serde(default)]
        last_assistant_message: String,
    },
    SessionEnd,
    #[serde(other)]
    Other,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct ToolUse {
    pub tool_name: String,
    #[serde(default)]
    pub tool_input: ToolInput,
}

#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ToolInput {
    /// What the model says the call is for; Bash and a few other tools have it.
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationType {
    /// A tool call is waiting for the user's permission.
    PermissionPrompt,
    /// An MCP server is asking the user for input.
    ElicitationDialog,
    /// Claude has been waiting for a new prompt for a while. The `Stop` that
    /// came before already said the session is done.
    IdlePrompt,
    AuthSuccess,
    /// A kind this version doesn't know. Treated as needing the user, since
    /// missing a prompt costs more than an extra alert.
    #[default]
    #[serde(other)]
    Other,
}

/// Parses a hook's stdin. Anything unparseable is `None`: a status update
/// must never break the agent's hook chain.
#[must_use]
pub fn parse(input: &str) -> Option<HookEvent> {
    serde_json::from_str(input).ok()
}

/// What an event means for the session's attention state, or `None` when it
/// changes nothing.
#[must_use]
pub fn attention(event: &HookEvent) -> Option<(AttentionState, Summary)> {
    match event {
        HookEvent::SessionStart => Some((AttentionState::Idle, Summary::sanitize(""))),
        HookEvent::UserPromptSubmit { prompt } => {
            Some((AttentionState::Working, Summary::sanitize(prompt)))
        }
        HookEvent::PreToolUse(tool) | HookEvent::PostToolUse(tool) => {
            let summary = match &tool.tool_input.description {
                Some(description) => format!("{}: {description}", tool.tool_name),
                None => tool.tool_name.clone(),
            };
            Some((AttentionState::Working, Summary::sanitize(&summary)))
        }
        HookEvent::Notification {
            message,
            notification_type,
        } => match notification_type {
            NotificationType::PermissionPrompt
            | NotificationType::ElicitationDialog
            | NotificationType::Other => {
                let message = if message.is_empty() {
                    "needs your attention"
                } else {
                    message
                };
                Some((AttentionState::Waiting, Summary::sanitize(message)))
            }
            NotificationType::IdlePrompt | NotificationType::AuthSuccess => None,
        },
        HookEvent::Stop {
            last_assistant_message,
        } => Some((
            AttentionState::Done,
            Summary::sanitize(last_assistant_message),
        )),
        HookEvent::SessionEnd => Some((AttentionState::Exited, Summary::sanitize(""))),
        HookEvent::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Payloads captured from Claude Code 2.1.284, with paths and IDs shortened.
    const SESSION_START: &str = r#"{"session_id":"s1","transcript_path":"/home/dev/.claude/projects/p/s1.jsonl","cwd":"/home/dev/app","hook_event_name":"SessionStart","source":"startup"}"#;
    const PROMPT: &str = r#"{"session_id":"s1","transcript_path":"/home/dev/.claude/projects/p/s1.jsonl","cwd":"/home/dev/app","prompt_id":"p1","permission_mode":"default","hook_event_name":"UserPromptSubmit","prompt":"Run the shell command 'echo probe' with your Bash tool.\nThen reply done."}"#;
    const PRE_TOOL: &str = r#"{"session_id":"s1","transcript_path":"/home/dev/.claude/projects/p/s1.jsonl","cwd":"/home/dev/app","prompt_id":"p1","permission_mode":"default","effort":{"level":"medium"},"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"echo probe","description":"Print the word probe"},"tool_use_id":"t1"}"#;
    const POST_TOOL: &str = r#"{"session_id":"s1","transcript_path":"/home/dev/.claude/projects/p/s1.jsonl","cwd":"/home/dev/app","prompt_id":"p1","permission_mode":"default","effort":{"level":"medium"},"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"echo probe","description":"Print the word probe"},"tool_response":{"stdout":"probe","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false},"tool_use_id":"t1","duration_ms":107}"#;
    const STOP: &str = r#"{"session_id":"s1","transcript_path":"/home/dev/.claude/projects/p/s1.jsonl","cwd":"/home/dev/app","prompt_id":"p1","permission_mode":"default","effort":{"level":"medium"},"hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"The command ran and printed probe.\n\nAnything else?","background_tasks":[],"session_crons":[]}"#;
    const IDLE: &str = r#"{"session_id":"s1","transcript_path":"/home/dev/.claude/projects/p/s1.jsonl","cwd":"/home/dev/app","scratchpad_dir":"/tmp/claude-1000/p/s1/scratchpad","prompt_id":"p1","hook_event_name":"Notification","message":"Claude is waiting for your input","notification_type":"idle_prompt"}"#;
    const SESSION_END: &str = r#"{"session_id":"s1","transcript_path":"/home/dev/.claude/projects/p/s1.jsonl","cwd":"/home/dev/app","prompt_id":"p1","hook_event_name":"SessionEnd","reason":"other"}"#;
    // Documented, not captured: permission prompts need an interactive approval.
    const PERMISSION: &str = r#"{"session_id":"s1","transcript_path":"/home/dev/.claude/projects/p/s1.jsonl","cwd":"/home/dev/app","hook_event_name":"Notification","message":"Claude needs your permission to use Bash","notification_type":"permission_prompt"}"#;

    fn state_of(input: &str) -> Option<(AttentionState, String)> {
        let event = parse(input).expect("a real payload parses");
        attention(&event).map(|(state, summary)| (state, summary.as_str().to_owned()))
    }

    #[test]
    fn a_turn_goes_idle_working_done_exited() {
        assert_eq!(
            state_of(SESSION_START),
            Some((AttentionState::Idle, String::new()))
        );
        assert_eq!(
            state_of(PROMPT),
            Some((
                AttentionState::Working,
                "Run the shell command 'echo probe' with your Bash tool.".into()
            ))
        );
        let tool = Some((AttentionState::Working, "Bash: Print the word probe".into()));
        assert_eq!(state_of(PRE_TOOL), tool);
        assert_eq!(state_of(POST_TOOL), tool);
        assert_eq!(
            state_of(STOP),
            Some((
                AttentionState::Done,
                "The command ran and printed probe.".into()
            ))
        );
        assert_eq!(
            state_of(SESSION_END),
            Some((AttentionState::Exited, String::new()))
        );
    }

    #[test]
    fn an_idle_prompt_leaves_a_done_session_done() {
        assert_eq!(state_of(IDLE), None);
    }

    #[test]
    fn a_permission_prompt_needs_the_user() {
        assert_eq!(
            state_of(PERMISSION),
            Some((
                AttentionState::Waiting,
                "Claude needs your permission to use Bash".into()
            ))
        );
    }

    #[test]
    fn unknown_events_and_notifications() {
        assert_eq!(
            state_of(r#"{"hook_event_name":"PreCompact","trigger":"auto"}"#),
            None
        );
        assert_eq!(
            state_of(r#"{"hook_event_name":"Notification","notification_type":"something_new"}"#),
            Some((AttentionState::Waiting, "needs your attention".into()))
        );
        assert_eq!(parse("not json"), None);
    }
}
