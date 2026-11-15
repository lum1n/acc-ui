//! The session's regions as apps read them. Each is a plain record, so it
//! serialises as is (golden tests, JSON over FFI) and maps one to one onto
//! UniFFI records later.

use serde::Serialize;

use super::event::{
    AskKind, AskOption, Detail, FileChangeKind, Hunk, ItemKind, ItemStatus, PlanEntry, PlanStatus, Quota,
    RunState, Usage,
};

/// Who, what and how much: the top of a session screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Header {
    pub harness: Option<String>,
    /// The harness's own session id, for resume.
    pub native_id: Option<String>,
    pub model: Option<String>,
    /// The harness's title for the session, or the first prompt's first
    /// line until it names one.
    pub title: Option<String>,
    pub state: RunState,
    pub state_message: Option<String>,
    /// The latest usage report, as the harness gave it.
    pub usage: Option<Usage>,
    pub quota: Option<Quota>,
    /// The session's working directory: what the client set, or the first
    /// command's `cwd`. File paths under it are shown relative to it.
    pub root: Option<String>,
    pub exited: bool,
    pub exit_code: Option<i32>,
    /// Turns so far, the open one included.
    pub turns: usize,
}

/// One thing the agent is doing right now, from a running item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Activity {
    pub item_id: String,
    /// The subagent item this runs under, when it isn't the main agent's.
    pub parent: Option<String>,
    pub kind: ActivityKind,
    /// What it acts on, short: a command's first line, a relative path, a
    /// subagent's task. Empty when there is nothing to say (thinking).
    pub target: String,
    pub since_ms: Option<u64>,
}

/// The verb of an [`Activity`]; apps word and localise it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Thinking,
    Writing,
    Running,
    Reading,
    Searching,
    Editing,
    Fetching,
    Planning,
    CallingTool,
    Delegating,
    WaitingForSubagents,
    Working,
}

impl ActivityKind {
    /// An English verb, for apps that don't localise.
    pub fn label(self) -> &'static str {
        match self {
            Self::Thinking => "Thinking",
            Self::Writing => "Writing",
            Self::Running => "Running",
            Self::Reading => "Reading",
            Self::Searching => "Searching",
            Self::Editing => "Editing",
            Self::Fetching => "Fetching",
            Self::Planning => "Planning",
            Self::CallingTool => "Calling",
            Self::Delegating => "Delegating",
            Self::WaitingForSubagents => "Waiting for subagents",
            Self::Working => "Working",
        }
    }
}

/// The agent's to-do list, with its progress.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PlanView {
    pub entries: Vec<PlanEntry>,
    pub explanation: Option<String>,
    pub completed: usize,
    pub total: usize,
    /// The entry in progress, or the first pending one.
    pub current: Option<usize>,
}

impl PlanView {
    pub(crate) fn new(entries: Vec<PlanEntry>, explanation: Option<String>) -> Self {
        let completed = entries
            .iter()
            .filter(|e| e.status == PlanStatus::Completed)
            .count();
        let current = entries
            .iter()
            .position(|e| e.status == PlanStatus::InProgress)
            .or_else(|| entries.iter().position(|e| e.status == PlanStatus::Pending));
        Self {
            total: entries.len(),
            entries,
            explanation,
            completed,
            current,
        }
    }
}

/// An approval or question waiting on the user.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ask {
    pub request_id: String,
    pub kind: AskKind,
    pub title: String,
    pub options: Vec<AskOption>,
    /// The item it is about, when the harness says.
    pub item_id: Option<String>,
    /// The tool call to approve (its command, file changes or arguments).
    pub subject: Option<Detail>,
    pub turn: usize,
}

/// A file the session read or changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileEntry {
    pub path: String,
    /// `path` relative to the session's root, or `path` when outside it.
    pub display: String,
    pub old_path: Option<String>,
    /// The net change: `add` if the session created it, `delete` if it
    /// removed it last, `update` otherwise. `None` when only read.
    pub change: Option<FileChangeKind>,
    pub added: u32,
    pub removed: u32,
    pub reads: u32,
    pub edits: Vec<FileEdit>,
    /// Turns that read or changed it, in order.
    pub turns: Vec<usize>,
}

/// One change to a file, from one file-change item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileEdit {
    pub item_id: String,
    pub turn: usize,
    pub kind: FileChangeKind,
    pub added: u32,
    pub removed: u32,
    pub hunks: Vec<Hunk>,
}

/// A shell command the agent ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandEntry {
    pub item_id: String,
    pub parent: Option<String>,
    pub turn: usize,
    /// As the harness ran it, shell wrapper included.
    pub command: String,
    /// What the agent asked for: `zsh -lc '...'` unwrapped.
    pub display: String,
    pub cwd: Option<String>,
    pub status: ItemStatus,
    pub exit_code: Option<i32>,
    /// The last lines of output (see [`OUTPUT_TAIL_LINES`]).
    pub output_tail: String,
    pub output_lines: usize,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
}

/// Lines of output kept in [`CommandEntry::output_tail`].
pub const OUTPUT_TAIL_LINES: usize = 40;

/// A run of another agent and what it did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubagentEntry {
    pub item_id: String,
    /// The subagent this one runs under, for nested runs.
    pub parent: Option<String>,
    pub turn: usize,
    pub agent: Option<String>,
    pub description: String,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub status: ItemStatus,
    /// The harness's own status word.
    pub harness_status: Option<String>,
    pub background: bool,
    /// Its final report, or its latest summary while it runs.
    pub report: Option<String>,
    /// Items of its conversation so far.
    pub items: u32,
    pub commands: u32,
    pub edits: u32,
}

/// One turn: a prompt and everything up to the turn's end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Turn {
    pub index: usize,
    /// The user's prompt; `None` for turns the harness started itself (a
    /// background subagent reporting back) or that began before acc-ui saw
    /// them.
    pub prompt: Option<String>,
    /// What the agent said last in this turn: its answer once the turn
    /// ends, its latest message while it runs. Earlier messages and
    /// reasoning are in `items`, for apps that fold them away.
    pub narrative: Option<String>,
    pub narrative_item: Option<String>,
    /// The main conversation's items in this turn, in order.
    pub items: Vec<String>,
    pub digest: Digest,
    pub outcome: TurnOutcome,
    pub open: bool,
    /// Best known start and end, ms since the Unix epoch, from the times
    /// ACC reports (state observations, item timings).
    pub started_at_ms: Option<u64>,
    pub ended_at_ms: Option<u64>,
    /// The turn's aggregated unified diff, where the harness sends one.
    pub diff: Option<String>,
}

impl Turn {
    pub fn duration_ms(&self) -> Option<u64> {
        Some(self.ended_at_ms?.saturating_sub(self.started_at_ms?))
    }
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    #[default]
    Running,
    Done,
    /// The harness reported an error.
    Failed,
    /// A new prompt or the end of the process cut it short.
    Interrupted,
    /// It ended with background work (a subagent) that starts another turn.
    Waiting,
}

/// What a turn did, counted, for a one-line summary under its prompt.
/// Subagents' work in the turn counts too.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Digest {
    /// Distinct files changed, of them created and deleted.
    pub files_changed: u32,
    pub files_created: u32,
    pub files_deleted: u32,
    pub lines_added: u32,
    pub lines_removed: u32,
    /// Distinct files read through read tools.
    pub files_read: u32,
    pub commands: u32,
    /// Commands that exited non-zero or failed.
    pub commands_failed: u32,
    pub subagents: u32,
    /// Other tool calls (searches, fetches, MCP tools).
    pub tools: u32,
}

impl Digest {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// An English line such as `Edited 3 files (+19 −3) · read 2 files ·
    /// ran 3 commands, 1 failed · 1 subagent`. Apps that localise build
    /// their own from the fields.
    pub fn summary(&self) -> String {
        let n =
            |count: u32, one: &str, many: &str| format!("{count} {}", if count == 1 { one } else { many });
        let mut parts = Vec::new();
        if self.files_changed > 0 {
            parts.push(format!(
                "Edited {} (+{} −{})",
                n(self.files_changed, "file", "files"),
                self.lines_added,
                self.lines_removed
            ));
        }
        if self.files_read > 0 {
            parts.push(format!("read {}", n(self.files_read, "file", "files")));
        }
        if self.commands > 0 {
            let mut s = format!("ran {}", n(self.commands, "command", "commands"));
            if self.commands_failed > 0 {
                s.push_str(&format!(", {} failed", self.commands_failed));
            }
            parts.push(s);
        }
        if self.subagents > 0 {
            parts.push(n(self.subagents, "subagent", "subagents"));
        }
        if self.tools > 0 {
            parts.push(n(self.tools, "other tool", "other tools"));
        }
        let mut s = parts.join(" · ");
        if let Some(c) = s.get(..1) {
            s = c.to_uppercase() + &s[1..];
        }
        s
    }
}

/// An item as the session keeps it: identity, place and text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemView {
    pub id: String,
    pub kind: ItemKind,
    pub parent: Option<String>,
    pub turn: usize,
    pub status: ItemStatus,
    /// Message or reasoning text, accumulated from deltas.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip)]
    pub detail: Detail,
    #[serde(skip)]
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
}
