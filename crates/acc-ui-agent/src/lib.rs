//! Opt-in: agents that write `acc-ui/1` specs themselves.
//!
//! The core `acc-ui` crate turns what agents already produce into UI and
//! costs the agent nothing. This crate is for clients that also want agents
//! to make components on purpose (a chart of benchmark numbers, a form to
//! fill in), and accept the context it costs:
//!
//! - [`skill_markdown`] and [`instructions`] teach the spec;
//! - [`host`] holds the `acc_ui` MCP server (`acc_ui_render`, `acc_ui_ask`)
//!   with the scripts that install it and deliver answers;
//! - [`recognize_tool`] finds those calls in a transcript, and [`session`]
//!   in a `SessionView`, with how to answer a waiting ask;
//! - [`submit_prompt`] answers a component with a user message when no
//!   `acc_ui_ask` call waits.

pub mod host;
pub mod recognize;
pub mod session;
pub mod skill;
pub mod submit;

pub use host::{Answer, McpServer, answer_script, mcp_server, server_script, setup_script};
pub use recognize::{AskState, UiCall, UiCallKind, recognize_tool};
pub use session::{AgentCall, Delivery, agent_call, agent_calls, open_agent_asks};
pub use skill::{instructions, skill_markdown};
pub use submit::{Submission, parse_submit, submit_prompt};
