//! The session's regions as apps read them. Each is a plain record, so it
//! serialises as is (golden tests, JSON over FFI) and maps one to one onto
//! UniFFI records later.

use serde::Serialize;
use serde::ser::{SerializeStruct, Serializer};

use super::ask::Question;
use super::command::{CommandKind, Diagnostics, TestCounts};
use super::event::{
    AskKind, AskOption, Detail, FileChangeKind, Hunk, ImageRef, ItemKind, ItemStatus, PlanEntry, PlanStatus,
    Quota, RunState, Usage,
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
    /// Running tests.
    Testing,
    /// Building, compiling or type checking.
    Building,
    /// Running a linter.
    Checking,
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
            Self::Testing => "Testing",
            Self::Building => "Building",
            Self::Checking => "Checking",
        }
    }

    /// The English verb for something finished, for summaries.
    pub fn past_label(self) -> &'static str {
        match self {
            Self::Thinking => "Thought",
            Self::Writing => "Wrote",
            Self::Running => "Ran",
            Self::Reading => "Read",
            Self::Searching => "Searched",
            Self::Editing => "Edited",
            Self::Fetching => "Fetched",
            Self::Planning => "Planned",
            Self::CallingTool => "Called",
            Self::Delegating => "Delegated",
            Self::WaitingForSubagents => "Waited for subagents",
            Self::Working => "Worked",
            Self::Testing => "Tested",
            Self::Building => "Built",
            Self::Checking => "Checked",
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
    /// What a question asks, one entry per question; empty for approvals.
    /// Answer them with an [`AnswerDraft`](super::AnswerDraft). When empty,
    /// a question still takes free text as its option id.
    pub questions: Vec<Question>,
    /// The plan to approve (Markdown) when the agent asks to leave plan
    /// mode; the options then approve or reject the plan.
    pub plan: Option<String>,
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
    /// `display` on one line for a list row: its first non-empty line, cut
    /// at [`HEADLINE_CHARS`], with `…` when anything was left out.
    pub headline: String,
    pub cwd: Option<String>,
    pub status: ItemStatus,
    pub exit_code: Option<i32>,
    /// Whether it worked, as far as anyone can tell.
    pub outcome: CommandOutcome,
    /// `outcome` wasn't taken from an exit code: it was read from the
    /// output (a test summary, a compiler's errors; `cargo test | tail`
    /// exits with `tail`'s code), or it is a Claude Code command that
    /// completed, which Claude only reports for a zero exit.
    pub outcome_inferred: bool,
    /// What kind of work it does; [`CommandKind::Other`] unless every part
    /// of the command was understood.
    pub kind: CommandKind,
    /// The program behind `kind`, with its subcommand where that matters:
    /// `cargo test`, `pytest`, `git commit`, `rg`.
    pub tool: Option<String>,
    /// Files it showed (`cat`, `head`, `sed -n`), as absolute paths. Only
    /// paths it could resolve: relative ones need a `cwd` or root.
    pub reads: Vec<String>,
    /// A test run's counts, from the runner's summary.
    pub tests: Option<TestCounts>,
    /// A build's or linter's errors and warnings.
    pub diagnostics: Option<Diagnostics>,
    /// The last lines of output (see [`OUTPUT_TAIL_LINES`]).
    pub output_tail: String,
    pub output_lines: usize,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
}

/// Lines of output kept in [`CommandEntry::output_tail`].
pub const OUTPUT_TAIL_LINES: usize = 40;

/// Longest [`CommandEntry::headline`], in characters.
pub const HEADLINE_CHARS: usize = 120;

/// How a command went.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutcome {
    #[default]
    Running,
    /// Exited 0.
    Succeeded,
    /// Exited non-zero, or the harness reported it failed.
    Failed,
    /// Finished, but the harness gave no exit code (Claude Code never
    /// does), so success can't be told from output alone.
    Unknown,
}

impl CommandOutcome {
    /// The outcome from the exit code, corrected by what the output says
    /// (`verdict`), and whether that correction decided it. Failed tests
    /// fail a command that exited 0 (output piped through `tail`); a
    /// passing summary only counts when there is no exit code. Without
    /// either, `zero_if_completed` (the harness marks non-zero exits as
    /// failed and this was one simple command) makes it a success.
    pub(crate) fn judge(
        status: ItemStatus,
        exit_code: Option<i32>,
        verdict: Option<bool>,
        zero_if_completed: bool,
    ) -> (Self, bool) {
        let plain = Self::of(status, exit_code);
        let judged = match (plain, verdict) {
            (Self::Succeeded | Self::Unknown, Some(false)) => Self::Failed,
            (Self::Unknown, Some(true)) => Self::Succeeded,
            (Self::Unknown, None) if zero_if_completed => Self::Succeeded,
            (o, _) => o,
        };
        (judged, judged != plain)
    }

    pub(crate) fn of(status: ItemStatus, exit_code: Option<i32>) -> Self {
        match (status, exit_code) {
            (ItemStatus::Failed, _) | (_, Some(1..) | Some(..=-1)) => Self::Failed,
            (ItemStatus::Running, _) => Self::Running,
            (ItemStatus::Completed, Some(0)) => Self::Succeeded,
            (ItemStatus::Completed, None) => Self::Unknown,
        }
    }
}

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
    /// them. Empty for a prompt that was only images.
    pub prompt: Option<String>,
    /// Images sent with the prompt.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub prompt_images: Vec<ImageRef>,
    /// What the agent said last in this turn: its answer once the turn
    /// ends, its latest message while it runs. Earlier messages and
    /// reasoning are in `items`, for apps that fold them away.
    pub narrative: Option<String>,
    pub narrative_item: Option<String>,
    /// The main conversation's items in this turn, in order. Reasoning
    /// joins once it has text (Codex sends empty reasoning when summaries
    /// are off), so every item here has something to show.
    pub items: Vec<String>,
    /// Items an app folds away under the prompt: everything in `items`
    /// but user messages and the narrative.
    pub steps: u32,
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
/// Subagents' work in the turn counts too. It serialises with its
/// [`parts`](Self::parts) and [`summary`](Self::summary) alongside the counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// The counts of the turn's last test run that printed a summary.
    pub tests: Option<TestCounts>,
}

impl Digest {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The counts worth showing, in reading order: one chip each in a
    /// digest row. Empty when the turn did nothing countable.
    pub fn parts(&self) -> Vec<DigestPart> {
        let mut parts = Vec::new();
        if self.files_changed > 0 {
            parts.push(DigestPart::Edited {
                files: self.files_changed,
                created: self.files_created,
                deleted: self.files_deleted,
                added: self.lines_added,
                removed: self.lines_removed,
            });
        }
        if self.files_read > 0 {
            parts.push(DigestPart::Read {
                files: self.files_read,
            });
        }
        if self.commands > 0 {
            parts.push(DigestPart::Ran {
                commands: self.commands,
                failed: self.commands_failed,
            });
        }
        if let Some(t) = self.tests {
            parts.push(DigestPart::Tests {
                passed: t.passed,
                failed: t.failed,
                skipped: t.skipped,
            });
        }
        if self.subagents > 0 {
            parts.push(DigestPart::Subagents {
                count: self.subagents,
            });
        }
        if self.tools > 0 {
            parts.push(DigestPart::Tools { count: self.tools });
        }
        parts
    }

    /// An English line such as `Edited 3 files (+19 −3) · read 2 files ·
    /// ran 3 commands, 1 failed · 1 subagent`. Apps that localise build
    /// their own from [`parts`](Self::parts).
    pub fn summary(&self) -> String {
        let mut s = self
            .parts()
            .iter()
            .map(DigestPart::label)
            .collect::<Vec<_>>()
            .join(" · ");
        if let Some(c) = s.get(..1) {
            s = c.to_uppercase() + &s[1..];
        }
        s
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("Digest", 13)?;
        s.serialize_field("files_changed", &self.files_changed)?;
        s.serialize_field("files_created", &self.files_created)?;
        s.serialize_field("files_deleted", &self.files_deleted)?;
        s.serialize_field("lines_added", &self.lines_added)?;
        s.serialize_field("lines_removed", &self.lines_removed)?;
        s.serialize_field("files_read", &self.files_read)?;
        s.serialize_field("commands", &self.commands)?;
        s.serialize_field("commands_failed", &self.commands_failed)?;
        s.serialize_field("subagents", &self.subagents)?;
        s.serialize_field("tools", &self.tools)?;
        s.serialize_field("tests", &self.tests)?;
        s.serialize_field("parts", &self.parts())?;
        s.serialize_field("summary", &self.summary())?;
        s.end()
    }
}

/// One count in a [`Digest`], with what an app needs to word and colour it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DigestPart {
    /// Files changed, `created` and `deleted` among them, and lines.
    Edited {
        files: u32,
        created: u32,
        deleted: u32,
        added: u32,
        removed: u32,
    },
    Read {
        files: u32,
    },
    /// Commands run; `failed` ones are worth a warning colour.
    Ran {
        commands: u32,
        failed: u32,
    },
    /// The last test run's counts; `failed` ones are worth a warning colour.
    Tests {
        passed: u32,
        failed: u32,
        skipped: u32,
    },
    Subagents {
        count: u32,
    },
    /// Other tool calls: searches, fetches, MCP tools.
    Tools {
        count: u32,
    },
}

impl DigestPart {
    /// The part in English, lower case: `edited 3 files (+19 −3)`.
    pub fn label(&self) -> String {
        let n =
            |count: u32, one: &str, many: &str| format!("{count} {}", if count == 1 { one } else { many });
        match *self {
            Self::Edited {
                files,
                added,
                removed,
                ..
            } => format!("edited {} (+{added} −{removed})", n(files, "file", "files")),
            Self::Read { files } => format!("read {}", n(files, "file", "files")),
            Self::Ran { commands, failed } => {
                let mut s = format!("ran {}", n(commands, "command", "commands"));
                if failed > 0 {
                    s.push_str(&format!(", {failed} failed"));
                }
                s
            }
            Self::Tests { passed, failed, .. } => match (passed, failed) {
                (_, 0) => format!("{} passed", n(passed, "test", "tests")),
                (0, _) => format!("{} failed", n(failed, "test", "tests")),
                _ => format!("{} failed, {passed} passed", n(failed, "test", "tests")),
            },
            Self::Subagents { count } => n(count, "subagent", "subagents"),
            Self::Tools { count } => n(count, "other tool", "other tools"),
        }
    }

    /// Whether it reports something that went wrong.
    pub fn is_warning(&self) -> bool {
        matches!(
            self,
            Self::Ran { failed: 1.., .. } | Self::Tests { failed: 1.., .. }
        )
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
    /// Images it carries: a prompt's attachments, an image a tool viewed
    /// or generated.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
}
