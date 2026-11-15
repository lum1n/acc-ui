//! `acc-ui`: agent transcripts as native UI.
//!
//! ACC turns each harness's output into typed items. acc-ui turns those
//! items, and the markdown agents write, into components (cards, tables,
//! checklists, diagrams, charts, callouts, file/PR/commit links) that apps
//! render natively. The agent's context is untouched: acc-ui only reads what
//! the agent already produced.
//!
//! - [`spec`]: the components, as an `acc-ui/1` tree.
//! - [`normalize`]: validates a tree and flattens it into what renderers
//!   take, with limits and a plain-text fallback.
//! - [`session`]: the session model. ACC events in; header, now, plan,
//!   needs-you, files, commands, subagents and turns out.
//! - [`blocks`]: an agent's markdown as blocks (tables, checklists,
//!   callouts, diagrams, code), the rest left as markdown.
//! - [`links`]: files, PRs, commits and URLs in agent text.
//! - [`fence`]: fences that hold components (` ```mermaid `, ` ```chart `).
//! - [`conventions`]: optional text that nudges agents toward markdown
//!   that renders best.
//!
//! Agents writing specs themselves (the `acc_ui` MCP server, a spec skill)
//! is opt-in and lives in the `acc-ui-agent` crate.

pub mod blocks;
pub mod conventions;
pub mod fence;
pub mod links;
pub mod normalize;
pub mod schema;
pub mod session;
pub mod spec;

pub use blocks::markdown_blocks;
pub use conventions::{conventions, conventions_skill};
pub use fence::{is_component_fence, parse_fence};
pub use links::{Link, LinkKind, detect_links};
pub use normalize::{Normalized, normalize, normalize_value, valid_id};
pub use schema::schema;
pub use session::{Changes, SessionView, Snapshot};
pub use spec::VERSION;
