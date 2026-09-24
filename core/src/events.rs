//! Hook protocol: the stdin JSON the agent sends, the stdout JSON we answer.
//! See spec §5 (measured against Claude Code 2.1.117).

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    PreToolUse,
    PostToolUse,
    SessionStart,
    UserPromptSubmit,
    PreCompact,
}

impl Event {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "PreToolUse" => Some(Self::PreToolUse),
            "PostToolUse" => Some(Self::PostToolUse),
            "SessionStart" => Some(Self::SessionStart),
            "UserPromptSubmit" => Some(Self::UserPromptSubmit),
            "PreCompact" => Some(Self::PreCompact),
            _ => None,
        }
    }
}

/// Everything an event may carry. Only the fields each event actually uses are set;
/// the rest are `None`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HookEvent {
    pub session_id: String,
    pub cwd: String,
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Value,
    pub tool_use_id: Option<String>,
    /// SessionStart only: `startup` | `clear` | `compact` | `resume`.
    pub source: Option<String>,
    /// UserPromptSubmit only: the user's message text.
    pub prompt: Option<String>,
    /// PreCompact only: `manual` | `auto`.
    pub trigger: Option<String>,
}

impl HookEvent {
    pub fn read(stdin: &mut dyn std::io::Read) -> std::io::Result<Self> {
        let mut s = String::new();
        stdin.read_to_string(&mut s)?;
        Ok(serde_json::from_str(&s)?)
    }
}

/// The stdout contract. Stdout is written last, and is always complete valid JSON.
pub enum Output {
    /// No decision to communicate — the universal failure shape, too.
    Passthrough,
    /// A rewritten `tool_input` for the upcoming tool call.
    UpdatedInput(Value),
    /// Extra context appended to the agent's context window.
    Context(Event, String),
}

impl Output {
    pub fn json(&self) -> Value {
        match self {
            Output::Passthrough => Value::Object(serde_json::Map::new()),
            Output::UpdatedInput(input) => specific("PreToolUse", serde_json::json!({
                "updatedInput": input
            })),
            Output::Context(ev, ctx) => {
                specific(&ev.name(), serde_json::json!({ "additionalContext": ctx }))
            }
        }
    }

    /// Serialize for writing to stdout.
    pub fn to_string(&self) -> String {
        serde_json::to_string(&self.json()).unwrap_or_else(|_| "{}".to_string())
    }
}

/// `{"hookSpecificOutput":{"hookEventName":..., ...body}}`.
fn specific(event: &str, body: Value) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("hookEventName".to_string(), Value::String(event.to_string()));
    out.extend(body.as_object().cloned().unwrap_or_default());
    let mut top = serde_json::Map::new();
    top.insert("hookSpecificOutput".to_string(), Value::Object(out));
    Value::Object(top)
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::PreToolUse => "PreToolUse",
            Event::PostToolUse => "PostToolUse",
            Event::SessionStart => "SessionStart",
            Event::UserPromptSubmit => "UserPromptSubmit",
            Event::PreCompact => "PreCompact",
        }
    }
}
