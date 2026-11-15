//! UniFFI surface of acc-ui for Swift (iOS/macOS) and Kotlin.
//!
//! The core (rendering what agents already write) and the opt-in agent
//! layer (`acc-ui-agent`: the `acc_ui` MCP server, the spec skill, the
//! submit tag) are both here; using only the core costs the agent nothing.
//!
//! JSON crosses the boundary as strings: specs go to the renderer as the
//! flat JSON [`acc_ui::normalize`] produces, which it decodes with its own
//! types. Everything else stays in the `acc-ui` crate.

use std::collections::HashMap;

use serde_json::{Map, Value};

uniffi::setup_scaffolding!();

/// Fields are `detail`, not `message`, which would clash with Kotlin's
/// `Throwable.message`.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AccUiError {
    #[error("{detail}")]
    Invalid { detail: String },
}

fn invalid(detail: impl Into<String>) -> AccUiError {
    AccUiError::Invalid {
        detail: detail.into(),
    }
}

/// A normalised spec and the problems found while making it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NormalizedSpec {
    /// The flat spec as JSON, or `None` when nothing could be rendered.
    pub spec_json: Option<String>,
    /// The spec's id.
    pub id: Option<String>,
    pub errors: Vec<String>,
}

impl From<acc_ui::Normalized> for NormalizedSpec {
    fn from(n: acc_ui::Normalized) -> Self {
        NormalizedSpec {
            id: n.id().map(str::to_owned),
            spec_json: n.spec_json(),
            errors: n.errors,
        }
    }
}

/// An answer found in a message.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Submission {
    pub id: String,
    /// The values as a JSON object.
    pub values_json: String,
    /// The message text outside the tag.
    pub rest: String,
}

impl From<acc_ui_agent::Submission> for Submission {
    fn from(s: acc_ui_agent::Submission) -> Self {
        Submission {
            id: s.id,
            values_json: Value::Object(s.values).to_string(),
            rest: s.rest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UiCallKind {
    Render,
    Ask,
}

/// Where an `acc_ui_ask` stands.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum AskState {
    Waiting,
    Answered { submission: Submission },
    Dismissed,
    TimedOut,
    Failed { detail: String },
}

/// A recognised `acc_ui` tool call.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct UiCall {
    pub kind: UiCallKind,
    pub spec: NormalizedSpec,
    /// `None` for renders.
    pub ask: Option<AskState>,
}

/// How to launch the MCP server; hand it to ACC as a session MCP server.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// The spec version, `acc-ui/1`.
#[uniffi::export]
pub fn spec_version() -> String {
    acc_ui::VERSION.to_owned()
}

/// Validates and flattens a spec given as JSON.
#[uniffi::export]
pub fn normalize(json: String) -> NormalizedSpec {
    acc_ui::normalize(&json).into()
}

/// Whether a fence with this info string carries a component (for a
/// placeholder while it streams).
#[uniffi::export]
pub fn is_component_fence(info: String) -> bool {
    acc_ui::is_component_fence(&info)
}

/// Parses a closed fence; `None` when it isn't a component fence.
#[uniffi::export]
pub fn parse_fence(info: String, body: String) -> Option<NormalizedSpec> {
    acc_ui::parse_fence(&info, &body).map(Into::into)
}

/// Recognises an `acc_ui` tool call from an item's tool name, input JSON and
/// output (`None` while running).
#[uniffi::export]
pub fn recognize_tool(name: String, input_json: String, output: Option<String>) -> Option<UiCall> {
    let input = serde_json::from_str(&input_json).unwrap_or(Value::String(input_json));
    let call = acc_ui_agent::recognize_tool(&name, &input, output.as_deref())?;
    Some(UiCall {
        kind: match call.kind {
            acc_ui_agent::UiCallKind::Render => UiCallKind::Render,
            acc_ui_agent::UiCallKind::Ask => UiCallKind::Ask,
        },
        spec: call.spec.into(),
        ask: call.ask.map(|a| match a {
            acc_ui_agent::AskState::Waiting => AskState::Waiting,
            acc_ui_agent::AskState::Answered(s) => AskState::Answered { submission: s.into() },
            acc_ui_agent::AskState::Dismissed => AskState::Dismissed,
            acc_ui_agent::AskState::TimedOut => AskState::TimedOut,
            acc_ui_agent::AskState::Failed(detail) => AskState::Failed { detail },
        }),
    })
}

fn object(values_json: &str) -> Result<Map<String, Value>, AccUiError> {
    match serde_json::from_str(values_json) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(invalid("values must be a JSON object")),
        Err(e) => Err(invalid(format!("values are not valid JSON: {e}"))),
    }
}

/// The user message that answers component `id` with `values_json`.
#[uniffi::export]
pub fn submit_prompt(id: String, values_json: String) -> Result<String, AccUiError> {
    Ok(acc_ui_agent::submit_prompt(&id, &object(&values_json)?))
}

/// Finds a submit tag in a message.
#[uniffi::export]
pub fn parse_submit(text: String) -> Option<Submission> {
    acc_ui_agent::parse_submit(&text).map(Into::into)
}

/// Whether `id` is a valid spec or session id.
#[uniffi::export]
pub fn valid_id(id: String) -> bool {
    acc_ui::valid_id(&id)
}

/// The JSON Schema of a spec.
#[uniffi::export]
pub fn schema_json() -> String {
    acc_ui::schema().to_string()
}

/// A few lines for a session's system prompt: markdown conventions that
/// render best. Optional; costs the agent about 90 tokens a turn.
#[uniffi::export]
pub fn conventions() -> String {
    acc_ui::conventions()
}

/// The `SKILL.md` of the `acc-ui-formatting` skill: the conventions in
/// detail, loaded only when the agent needs them.
#[uniffi::export]
pub fn conventions_skill() -> String {
    acc_ui::conventions_skill()
}

// Opt-in: agents writing specs themselves (acc-ui-agent).

/// The `SKILL.md` of the `acc-ui-components` skill, which teaches the spec.
#[uniffi::export]
pub fn agent_skill_markdown() -> String {
    acc_ui_agent::skill_markdown()
}

/// Short instructions about the spec and the `acc_ui` tools.
#[uniffi::export]
pub fn agent_instructions() -> String {
    acc_ui_agent::instructions()
}

/// The MCP server script.
#[uniffi::export]
pub fn server_script() -> String {
    acc_ui_agent::server_script()
}

/// Shell that installs the server on the host and prints its path.
#[uniffi::export]
pub fn setup_script() -> String {
    acc_ui_agent::setup_script()
}

/// The server launch for a session, from the path `setup_script` printed.
#[uniffi::export]
pub fn mcp_server(script_path: String, session: String) -> Result<McpServer, AccUiError> {
    let s = acc_ui_agent::mcp_server(&script_path, &session).map_err(invalid)?;
    Ok(McpServer {
        name: s.name,
        command: s.command,
        args: s.args,
        env: s.env.into_iter().collect(),
    })
}

/// Shell that answers the waiting `acc_ui_ask` for spec `id`: with
/// `values_json` (a JSON object), or dismissed when `None`.
#[uniffi::export]
pub fn answer_script(session: String, id: String, values_json: Option<String>) -> Result<String, AccUiError> {
    let answer = match values_json {
        Some(v) => acc_ui_agent::Answer::Values(object(&v)?),
        None => acc_ui_agent::Answer::Dismissed,
    };
    acc_ui_agent::answer_script(&session, &id, &answer).map_err(invalid)
}
