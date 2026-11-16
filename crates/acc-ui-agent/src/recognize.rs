//! Recognising `acc_ui` tool calls in a transcript, whatever the harness
//! calls them: `mcp__acc_ui__acc_ui_render` (Claude), `acc_ui/acc_ui_render`
//! (Codex), or the bare name or a title around it (ACP agents).

use serde_json::Value;

use crate::host::{ASK_TOOL, RENDER_TOOL, result_marker};
use crate::submit::{Submission, parse_submit};
use acc_ui::{Normalized, normalize, normalize_value};

/// Which tool was called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCallKind {
    /// `acc_ui_render`: shown, returns at once.
    Render,
    /// `acc_ui_ask`: shown, and the call waits for the user's answer.
    Ask,
}

/// Where an `acc_ui_ask` stands, read from the tool's output.
#[derive(Debug, Clone, PartialEq)]
pub enum AskState {
    /// No output yet: the call is waiting. Show the inputs.
    Waiting,
    /// The user answered.
    Answered(Submission),
    /// The user closed it without answering.
    Dismissed,
    /// The call gave up waiting. The user can still answer; the answer then
    /// goes as a submit prompt.
    TimedOut,
    /// The call failed (bad id, spec as a string...). The text says why.
    Failed(String),
}

/// A recognised `acc_ui` tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct UiCall {
    pub kind: UiCallKind,
    /// The spec to render, with any errors in it.
    pub spec: Normalized,
    /// For [`UiCallKind::Ask`], the state of the answer. `None` for renders.
    pub ask: Option<AskState>,
}

/// Recognises a tool call by its name, input and output (`None` while it
/// runs). `None` when it isn't an `acc_ui` tool.
pub fn recognize_tool(name: &str, input: &Value, output: Option<&str>) -> Option<UiCall> {
    let kind = if has_tool(name, ASK_TOOL) {
        UiCallKind::Ask
    } else if has_tool(name, RENDER_TOOL) {
        UiCallKind::Render
    } else {
        return None;
    };
    let spec = match spec_of(input) {
        SpecIn::Value(v) => normalize_value(v.clone()),
        SpecIn::Text(s) => normalize(s),
        SpecIn::Missing => Normalized {
            spec: None,
            errors: vec!["the call has no spec".to_owned()],
        },
    };
    let ask = (kind == UiCallKind::Ask).then(|| ask_state(output));
    Some(UiCall { kind, spec, ask })
}

enum SpecIn<'a> {
    Value(&'a Value),
    Text(&'a str),
    Missing,
}

fn spec_of(input: &Value) -> SpecIn<'_> {
    match input.get("spec") {
        Some(Value::String(s)) => SpecIn::Text(s),
        Some(v) => SpecIn::Value(v),
        // Some harnesses hand over the arguments as a JSON string.
        None if input.is_string() => match input.as_str() {
            Some(s) => SpecIn::Text(s),
            None => SpecIn::Missing,
        },
        None if input.get("root").is_some() => SpecIn::Value(input),
        None => SpecIn::Missing,
    }
}

fn ask_state(output: Option<&str>) -> AskState {
    let Some(out) = output.map(str::trim).filter(|o| !o.is_empty()) else {
        return AskState::Waiting;
    };
    if out.starts_with(result_marker::ANSWERED)
        && let Some(s) = parse_submit(out)
    {
        return AskState::Answered(s);
    }
    // Claude hands over an answer's `structuredContent` instead of its text.
    if let Ok(Value::Object(o)) = serde_json::from_str::<Value>(out)
        && let (Some(Value::String(id)), Some(Value::Object(values))) = (o.get("id"), o.get("values"))
    {
        return AskState::Answered(Submission {
            id: id.clone(),
            values: values.clone(),
            rest: String::new(),
        });
    }
    if out.starts_with(result_marker::DISMISSED) {
        return AskState::Dismissed;
    }
    if out.starts_with(result_marker::TIMED_OUT) {
        return AskState::TimedOut;
    }
    AskState::Failed(out.to_owned())
}

/// Whether `name` contains `tool` as a whole word: preceded by the start,
/// `_` or a non-alphanumeric, and followed by the end or a character that
/// can't continue an identifier.
fn has_tool(name: &str, tool: &str) -> bool {
    name.match_indices(tool).any(|(i, _)| {
        let before = name[..i].chars().next_back();
        let after = name[i + tool.len()..].chars().next();
        before.is_none_or(|c| !c.is_ascii_alphanumeric())
            && after.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
    })
}
