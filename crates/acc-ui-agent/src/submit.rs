//! The submit tag: how an answer to a component reaches the agent as a user
//! message when no `acc_ui_ask` call is waiting for it.
//!
//! ```text
//! <acc-ui-submit id="deploy-target">{"env":"prod"}</acc-ui-submit>
//! ```

use serde_json::{Map, Value};

const OPEN: &str = "<acc-ui-submit id=\"";
const CLOSE: &str = "</acc-ui-submit>";

/// An answer found in a message.
#[derive(Debug, Clone, PartialEq)]
pub struct Submission {
    /// The component's spec id.
    pub id: String,
    /// The submitted values.
    pub values: Map<String, Value>,
    /// The message text outside the tag, trimmed.
    pub rest: String,
}

/// The message that answers component `id` with `values`.
pub fn submit_prompt(id: &str, values: &Map<String, Value>) -> String {
    format!("{OPEN}{}\">{}{CLOSE}", attr(id), json_in_tag(values))
}

/// Finds the first submit tag in `text`.
pub fn parse_submit(text: &str) -> Option<Submission> {
    let start = text.find(OPEN)?;
    let after_open = &text[start + OPEN.len()..];
    let id_end = after_open.find("\">")?;
    let id = unattr(&after_open[..id_end]);
    let body_start = start + OPEN.len() + id_end + 2;
    let body_len = text[body_start..].find(CLOSE)?;
    let body = &text[body_start..body_start + body_len];
    let values = match serde_json::from_str::<Value>(body.trim()).ok()? {
        Value::Object(m) => m,
        _ => return None,
    };
    let after = &text[body_start + body_len + CLOSE.len()..];
    let rest = [text[..start].trim(), after.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Some(Submission { id, values, rest })
}

/// Compact JSON that can't close the tag early.
pub(crate) fn json_in_tag(values: &Map<String, Value>) -> String {
    Value::Object(values.clone()).to_string().replace("</", "<\\/")
}

fn attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

fn unattr(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}
