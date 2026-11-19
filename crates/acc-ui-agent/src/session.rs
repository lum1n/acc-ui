//! The agent layer in a [`SessionView`]: which items are `acc_ui` calls,
//! what they show, which asks wait for the user, and how to answer one.
//!
//! The core session model treats these calls as ordinary tool items (its
//! [`SessionView::item_spec`] shows their raw output): core stays free of
//! the agent layer, and the calls' names and answer states are this
//! crate's. An app that enabled the agent layer takes [`item_spec`] here
//! instead, which shows a call's own spec.

use acc_ui::Normalized;
use serde_json::{Map, Value};

use acc_ui::SessionView;
use acc_ui::session::ItemView;
use acc_ui::session::event::{Detail, ItemStatus};

use crate::host::{Answer, answer_script};
use crate::recognize::{AskState, UiCall, UiCallKind, recognize_tool};
use crate::submit::submit_prompt;

/// One `acc_ui_render` or `acc_ui_ask` call in a session.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentCall {
    pub item_id: String,
    /// The turn it was made in.
    pub turn: usize,
    /// Still running: an ask whose answer the server waits for.
    pub running: bool,
    pub call: UiCall,
}

impl AgentCall {
    /// The spec's id, which answers name.
    pub fn spec_id(&self) -> Option<&str> {
        self.call.spec.spec.as_ref()?.get("id")?.as_str()
    }

    /// Whether the user can still answer it: an ask that waits, or one
    /// that timed out (its answer then goes as a prompt).
    pub fn awaits_answer(&self) -> bool {
        self.call.kind == UiCallKind::Ask
            && matches!(self.call.ask, Some(AskState::Waiting | AskState::TimedOut))
            && self.spec_id().is_some()
    }

    /// How to deliver the user's `values` (`None` to dismiss): the answer
    /// script while the call waits, or a submit prompt for the next turn
    /// once it timed out. `session` is the id given to
    /// [`crate::mcp_server`]. `None` when it doesn't take an answer, or a
    /// dismissal that no call waits for.
    pub fn answer(
        &self,
        session: &str,
        values: Option<Map<String, Value>>,
    ) -> Option<Result<Delivery, String>> {
        if !self.awaits_answer() {
            return None;
        }
        let id = self.spec_id()?;
        if self.running && self.call.ask == Some(AskState::Waiting) {
            let answer = values.map_or(Answer::Dismissed, Answer::Values);
            return Some(answer_script(session, id, &answer).map(Delivery::Script));
        }
        values.map(|v| Ok(Delivery::Prompt(submit_prompt(id, &v))))
    }
}

/// How an answer reaches the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// Shell to run on the host: the waiting call returns with the answer.
    Script(String),
    /// A user message to send as the next prompt.
    Prompt(String),
}

/// The `acc_ui` call item `id` is, if it is one.
pub fn agent_call(view: &SessionView, id: &str) -> Option<AgentCall> {
    of_item(view.item(id)?)
}

/// Item `id` as a spec, like [`SessionView::item_spec`], except that an
/// `acc_ui` call shows the spec it carries (normalised as untrusted, so it
/// holds no requests). A call whose spec didn't survive normalising shows
/// as the tool call it is. An ask's inputs answer through
/// [`AgentCall::answer`], and only while [`AgentCall::awaits_answer`].
pub fn item_spec(view: &SessionView, id: &str) -> Option<Normalized> {
    match agent_call(view, id) {
        Some(call) if call.call.spec.spec.is_some() => Some(call.call.spec),
        _ => view.item_spec(id),
    }
}

/// Every `acc_ui` call in the session, in order.
pub fn agent_calls(view: &SessionView) -> Vec<AgentCall> {
    view.items().iter().filter_map(of_item).collect()
}

/// The asks the user can answer now, oldest first.
pub fn open_agent_asks(view: &SessionView) -> Vec<AgentCall> {
    agent_calls(view)
        .into_iter()
        .filter(AgentCall::awaits_answer)
        .collect()
}

fn of_item(item: &ItemView) -> Option<AgentCall> {
    let Detail::Tool {
        tool, input, output, ..
    } = &item.detail
    else {
        return None;
    };
    let running = item.status == ItemStatus::Running;
    // Output only counts once the call returned.
    let output = if running { None } else { output.as_deref() };
    let mut call = recognize_tool(tool, input, output)?;
    if !running && call.ask == Some(AskState::Waiting) {
        // Ended without a result: the server is gone.
        call.ask = Some(if item.status == ItemStatus::Failed {
            AskState::Failed(String::new())
        } else {
            AskState::TimedOut
        });
    }
    Some(AgentCall {
        item_id: item.id.clone(),
        turn: item.turn,
        running,
        call,
    })
}
