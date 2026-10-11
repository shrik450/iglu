//! Claude Code hook events, as Claude Code writes them to a hook's stdin,
//! and what each one says about whether the session needs its user.

use iglu_domain::attention::{AttentionState, Summary, ThreadKey};
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

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    session_id: Option<String>,
}

/// The conversation an event belongs to: Claude Code's session ID, or the
/// session's own thread when the event doesn't carry a usable one.
#[must_use]
pub fn thread(input: &str) -> ThreadKey {
    serde_json::from_str::<Envelope>(input)
        .ok()
        .and_then(|envelope| envelope.session_id)
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(ThreadKey::session)
}

/// What a conversation is about: the prompt that starts its first turn.
#[must_use]
pub fn title(event: &HookEvent) -> Option<Summary> {
    match event {
        HookEvent::UserPromptSubmit { prompt } => Some(Summary::sanitize(prompt)),
        HookEvent::SessionStart
        | HookEvent::PreToolUse(_)
        | HookEvent::PostToolUse(_)
        | HookEvent::Notification { .. }
        | HookEvent::Stop { .. }
        | HookEvent::SessionEnd
        | HookEvent::Other => None,
    }
}

/// What an event does to its conversation's thread.
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Set(AttentionState, Summary),
    /// The conversation is over, so its thread goes.
    Ended,
}

/// What an event means for its conversation's thread, or `None` when it
/// changes nothing.
#[must_use]
pub fn attention(event: &HookEvent) -> Option<Change> {
    let set = |state, summary: &str| Some(Change::Set(state, Summary::sanitize(summary)));
    match event {
        HookEvent::SessionStart => set(AttentionState::Idle, ""),
        HookEvent::UserPromptSubmit { prompt } => set(AttentionState::Working, prompt),
        HookEvent::PreToolUse(tool) | HookEvent::PostToolUse(tool) => {
            let summary = match &tool.tool_input.description {
                Some(description) => format!("{}: {description}", tool.tool_name),
                None => tool.tool_name.clone(),
            };
            set(AttentionState::Working, &summary)
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
                set(AttentionState::Waiting, message)
            }
            NotificationType::IdlePrompt | NotificationType::AuthSuccess => None,
        },
        HookEvent::Stop {
            last_assistant_message,
        } => set(AttentionState::Done, last_assistant_message),
        HookEvent::SessionEnd => Some(Change::Ended),
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
        match attention(&event)? {
            Change::Set(state, summary) => Some((state, summary.as_str().to_owned())),
            Change::Ended => None,
        }
    }

    #[test]
    fn a_turn_goes_idle_working_done_and_ends() {
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
        let end = parse(SESSION_END).expect("a real payload parses");
        assert_eq!(attention(&end), Some(Change::Ended));
    }

    #[test]
    fn events_name_their_conversation_and_prompts_title_it() {
        assert_eq!(thread(PROMPT).as_str(), "s1");
        assert_eq!(
            thread(r#"{"hook_event_name":"Stop","session_id":"../etc"}"#),
            ThreadKey::session()
        );
        assert_eq!(thread("not json"), ThreadKey::session());
        let prompt = parse(PROMPT).expect("a real payload parses");
        assert_eq!(
            title(&prompt).map(|t| t.as_str().to_owned()).as_deref(),
            Some("Run the shell command 'echo probe' with your Bash tool.")
        );
        assert_eq!(title(&parse(STOP).expect("a real payload parses")), None);
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
