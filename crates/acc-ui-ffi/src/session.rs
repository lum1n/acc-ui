//! The session model over FFI: a [`SessionView`] object fed ACC's event
//! JSON, and its regions as typed records.
//!
//! The records mirror `acc_ui::session` one to one, with counts and
//! indices as `u32` (UniFFI has no `usize`) and a tool's free-form input as
//! JSON text. Names that would be too generic in an app's namespace carry
//! a `Session` or `Item` prefix.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use acc_ui::session::{self as s, event as e};

use crate::{AccUiError, NormalizedSpec, invalid};

fn n(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

fn list<T: Clone, U: From<T>>(v: &[T]) -> Vec<U> {
    v.iter().cloned().map(Into::into).collect()
}

/// Generates a fieldless enum mirroring a core one, with `From`.
macro_rules! mirror_enum {
    ($(#[$m:meta])* $name:ident <= $core:path { $($v:ident),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
        pub enum $name { $($v),* }

        impl From<$core> for $name {
            fn from(v: $core) -> Self {
                use $core as C;
                match v { $(C::$v => Self::$v),* }
            }
        }
    };
}

mirror_enum!(
    AskKind
        <= e::AskKind {
            Permission,
            Question,
            Auth,
            Unknown
        }
);
mirror_enum!(
    /// What choosing an option does.
    OptionKind <= e::OptionKind { AllowOnce, AllowAlways, RejectOnce, RejectAlways, Other }
);
mirror_enum!(
    ItemKind
        <= e::ItemKind {
            UserMessage,
            AgentMessage,
            Reasoning,
            ToolCall,
            CommandExecution,
            FileChange,
            Subagent,
            Unknown,
        }
);
mirror_enum!(
    ItemStatus
        <= e::ItemStatus {
            Running,
            Completed,
            Failed
        }
);
mirror_enum!(
    ToolKind
        <= e::ToolKind {
            Shell,
            Edit,
            Read,
            Fetch,
            Mcp,
            Other
        }
);
mirror_enum!(
    FileChangeKind
        <= e::FileChangeKind {
            Add,
            Update,
            Delete,
            Rename
        }
);
mirror_enum!(LineTag <= e::LineTag { Context, Add, Remove });
mirror_enum!(
    PlanStatus
        <= e::PlanStatus {
            InProgress,
            Completed,
            Pending
        }
);
mirror_enum!(
    /// The verb of an [`Activity`]; apps word and localise it.
    ActivityKind <= s::ActivityKind {
        Thinking, Writing, Running, Reading, Searching, Editing, Fetching, Planning, CallingTool,
        Delegating, WaitingForSubagents, Working,
    }
);
mirror_enum!(
    /// How a command went; `Unknown` when it finished without an exit code.
    CommandOutcome <= s::CommandOutcome { Running, Succeeded, Failed, Unknown }
);
mirror_enum!(
    TurnOutcome
        <= s::TurnOutcome {
            Running,
            Done,
            Failed,
            Interrupted,
            Waiting
        }
);

/// What a session is doing.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RunState {
    Idle,
    Working {
        progress: Option<u8>,
    },
    Blocked {
        kind: AskKind,
    },
    Done,
    /// The turn ended, but background work will start another.
    Waiting,
    Error,
    Exited,
    Unknown,
}

impl From<e::RunState> for RunState {
    fn from(v: e::RunState) -> Self {
        match v {
            e::RunState::Idle => Self::Idle,
            e::RunState::Working { progress } => Self::Working { progress },
            e::RunState::Blocked { kind } => Self::Blocked { kind: kind.into() },
            e::RunState::Done => Self::Done,
            e::RunState::Waiting => Self::Waiting,
            e::RunState::Error => Self::Error,
            e::RunState::Exited => Self::Exited,
            e::RunState::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// A running total for the session.
    pub cost_usd: Option<f64>,
    pub context_used: Option<u64>,
    pub context_window: Option<u64>,
}

impl From<e::Usage> for Usage {
    fn from(u: e::Usage) -> Self {
        Self {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_read_tokens: u.cache_read_tokens,
            cache_write_tokens: u.cache_write_tokens,
            cost_usd: u.cost_usd,
            context_used: u.context_used,
            context_window: u.context_window,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QuotaWindow {
    pub id: String,
    /// 0-100.
    pub used_percent: Option<f64>,
    pub window_minutes: Option<u64>,
    /// Seconds since the Unix epoch.
    pub resets_at: Option<u64>,
    pub status: Option<String>,
}

impl From<e::QuotaWindow> for QuotaWindow {
    fn from(w: e::QuotaWindow) -> Self {
        Self {
            id: w.id,
            used_percent: w.used_percent,
            window_minutes: w.window_minutes,
            resets_at: w.resets_at,
            status: w.status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Quota {
    pub plan: Option<String>,
    pub windows: Vec<QuotaWindow>,
}

impl From<e::Quota> for Quota {
    fn from(q: e::Quota) -> Self {
        Self {
            plan: q.plan,
            windows: list(&q.windows),
        }
    }
}

/// Who, what and how much: the top of a session screen.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SessionHeader {
    pub harness: Option<String>,
    pub native_id: Option<String>,
    pub model: Option<String>,
    pub title: Option<String>,
    pub state: RunState,
    pub state_message: Option<String>,
    pub usage: Option<Usage>,
    pub quota: Option<Quota>,
    /// The working directory; paths under it are shown relative.
    pub root: Option<String>,
    pub exited: bool,
    pub exit_code: Option<i32>,
    pub turns: u32,
}

impl From<s::Header> for SessionHeader {
    fn from(h: s::Header) -> Self {
        Self {
            harness: h.harness,
            native_id: h.native_id,
            model: h.model,
            title: h.title,
            state: h.state.into(),
            state_message: h.state_message,
            usage: h.usage.map(Into::into),
            quota: h.quota.map(Into::into),
            root: h.root,
            exited: h.exited,
            exit_code: h.exit_code,
            turns: n(h.turns),
        }
    }
}

/// One thing the agent is doing right now.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Activity {
    pub item_id: String,
    /// The subagent item it runs under, if not the main agent's.
    pub parent: Option<String>,
    pub kind: ActivityKind,
    /// What it acts on, short; empty when there is nothing to say.
    pub target: String,
    /// The English verb, for apps that don't localise.
    pub label: String,
    pub since_ms: Option<u64>,
}

impl From<s::Activity> for Activity {
    fn from(a: s::Activity) -> Self {
        Self {
            item_id: a.item_id,
            parent: a.parent,
            label: a.kind.label().to_owned(),
            kind: a.kind.into(),
            target: a.target,
            since_ms: a.since_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlanEntry {
    pub content: String,
    pub status: PlanStatus,
    pub priority: Option<String>,
}

impl From<e::PlanEntry> for PlanEntry {
    fn from(p: e::PlanEntry) -> Self {
        Self {
            content: p.content,
            status: p.status.into(),
            priority: p.priority,
        }
    }
}

/// The agent's to-do list, with its progress.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlanView {
    pub entries: Vec<PlanEntry>,
    pub explanation: Option<String>,
    pub completed: u32,
    pub total: u32,
    /// The entry in progress, or the first pending one.
    pub current: Option<u32>,
}

impl From<s::PlanView> for PlanView {
    fn from(p: s::PlanView) -> Self {
        Self {
            entries: list(&p.entries),
            explanation: p.explanation,
            completed: n(p.completed),
            total: n(p.total),
            current: p.current.map(n),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AskOption {
    pub id: String,
    pub label: String,
    pub kind: Option<OptionKind>,
}

impl From<e::AskOption> for AskOption {
    fn from(o: e::AskOption) -> Self {
        Self {
            id: o.id,
            label: o.label,
            kind: o.kind.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiffLine {
    pub tag: LineTag,
    pub text: String,
}

/// A hunk; `old_start` and `new_start` are 0 when the position is unknown
/// (Claude's edits), so show it as a snippet.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

impl From<e::Hunk> for Hunk {
    fn from(h: e::Hunk) -> Self {
        Self {
            old_start: h.old_start,
            old_lines: h.old_lines,
            new_start: h.new_start,
            new_lines: h.new_lines,
            lines: h
                .lines
                .into_iter()
                .map(|l| DiffLine {
                    tag: l.tag.into(),
                    text: l.text,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: FileChangeKind,
    pub hunks: Vec<Hunk>,
    pub added: u32,
    pub removed: u32,
}

impl From<e::FileDiff> for FileDiff {
    fn from(f: e::FileDiff) -> Self {
        Self {
            path: f.path,
            old_path: f.old_path,
            kind: f.kind.into(),
            hunks: list(&f.hunks),
            added: f.added,
            removed: f.removed,
        }
    }
}

/// An item's harness-independent detail.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ItemDetail {
    /// A message or reasoning; its text is on the item.
    Text,
    Tool {
        tool: String,
        kind: ToolKind,
        title: String,
        /// The tool's input as JSON.
        input_json: String,
        output: Option<String>,
        paths: Vec<String>,
    },
    Command {
        tool: String,
        command: String,
        cwd: Option<String>,
        output: String,
        exit_code: Option<i32>,
    },
    FileChange {
        tool: String,
        files: Vec<FileDiff>,
    },
    Subagent {
        tool: String,
        agent: Option<String>,
        description: String,
        prompt: Option<String>,
        child: Option<String>,
        status: Option<String>,
        output: Option<String>,
        model: Option<String>,
        background: bool,
    },
    Unknown,
}

impl From<e::Detail> for ItemDetail {
    fn from(d: e::Detail) -> Self {
        match d {
            e::Detail::Text => Self::Text,
            e::Detail::Tool {
                tool,
                kind,
                title,
                input,
                output,
                paths,
            } => Self::Tool {
                tool,
                kind: kind.into(),
                title,
                input_json: input.to_string(),
                output,
                paths,
            },
            e::Detail::Command {
                tool,
                command,
                cwd,
                output,
                exit_code,
            } => Self::Command {
                tool,
                command,
                cwd,
                output,
                exit_code,
            },
            e::Detail::FileChange { tool, files } => Self::FileChange {
                tool,
                files: list(&files),
            },
            e::Detail::Subagent {
                tool,
                agent,
                description,
                prompt,
                child,
                status,
                output,
                model,
                background,
            } => Self::Subagent {
                tool,
                agent,
                description,
                prompt,
                child,
                status,
                output,
                model,
                background,
            },
            e::Detail::Unknown => Self::Unknown,
        }
    }
}

/// An approval or question waiting on the user.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Ask {
    pub request_id: String,
    pub kind: AskKind,
    pub title: String,
    pub options: Vec<AskOption>,
    pub item_id: Option<String>,
    /// The tool call to approve.
    pub subject: Option<ItemDetail>,
    pub turn: u32,
}

impl From<s::Ask> for Ask {
    fn from(a: s::Ask) -> Self {
        Self {
            request_id: a.request_id,
            kind: a.kind.into(),
            title: a.title,
            options: list(&a.options),
            item_id: a.item_id,
            subject: a.subject.map(Into::into),
            turn: n(a.turn),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FileEdit {
    pub item_id: String,
    pub turn: u32,
    pub kind: FileChangeKind,
    pub added: u32,
    pub removed: u32,
    pub hunks: Vec<Hunk>,
}

impl From<s::FileEdit> for FileEdit {
    fn from(f: s::FileEdit) -> Self {
        Self {
            item_id: f.item_id,
            turn: n(f.turn),
            kind: f.kind.into(),
            added: f.added,
            removed: f.removed,
            hunks: list(&f.hunks),
        }
    }
}

/// A file the session read or changed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FileEntry {
    pub path: String,
    /// Relative to the root when under it.
    pub display: String,
    pub old_path: Option<String>,
    /// The net change; `None` when only read.
    pub change: Option<FileChangeKind>,
    pub added: u32,
    pub removed: u32,
    pub reads: u32,
    pub edits: Vec<FileEdit>,
    pub turns: Vec<u32>,
}

impl From<s::FileEntry> for FileEntry {
    fn from(f: s::FileEntry) -> Self {
        Self {
            path: f.path,
            display: f.display,
            old_path: f.old_path,
            change: f.change.map(Into::into),
            added: f.added,
            removed: f.removed,
            reads: f.reads,
            edits: list(&f.edits),
            turns: f.turns.into_iter().map(n).collect(),
        }
    }
}

/// A shell command the agent ran.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CommandEntry {
    pub item_id: String,
    pub parent: Option<String>,
    pub turn: u32,
    /// As the harness ran it, shell wrapper included.
    pub command: String,
    /// What the agent asked for, unwrapped.
    pub display: String,
    /// `display` on one line, cut, for a list row.
    pub headline: String,
    pub cwd: Option<String>,
    pub status: ItemStatus,
    pub exit_code: Option<i32>,
    pub outcome: CommandOutcome,
    /// The last lines of output.
    pub output_tail: String,
    pub output_lines: u32,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
}

impl From<s::CommandEntry> for CommandEntry {
    fn from(c: s::CommandEntry) -> Self {
        Self {
            item_id: c.item_id,
            parent: c.parent,
            turn: n(c.turn),
            command: c.command,
            display: c.display,
            headline: c.headline,
            cwd: c.cwd,
            status: c.status.into(),
            exit_code: c.exit_code,
            outcome: c.outcome.into(),
            output_tail: c.output_tail,
            output_lines: n(c.output_lines),
            started_at_ms: c.started_at_ms,
            completed_at_ms: c.completed_at_ms,
        }
    }
}

/// A run of another agent.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SubagentEntry {
    pub item_id: String,
    pub parent: Option<String>,
    pub turn: u32,
    pub agent: Option<String>,
    pub description: String,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub status: ItemStatus,
    pub harness_status: Option<String>,
    pub background: bool,
    /// Its final report, or latest summary while it runs.
    pub report: Option<String>,
    pub items: u32,
    pub commands: u32,
    pub edits: u32,
}

impl From<s::SubagentEntry> for SubagentEntry {
    fn from(a: s::SubagentEntry) -> Self {
        Self {
            item_id: a.item_id,
            parent: a.parent,
            turn: n(a.turn),
            agent: a.agent,
            description: a.description,
            prompt: a.prompt,
            model: a.model,
            status: a.status.into(),
            harness_status: a.harness_status,
            background: a.background,
            report: a.report,
            items: a.items,
            commands: a.commands,
            edits: a.edits,
        }
    }
}

/// One count in a [`Digest`]: a chip in a turn's digest row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DigestPart {
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
    /// Show it as a warning when `failed` is above 0.
    Ran {
        commands: u32,
        failed: u32,
    },
    Subagents {
        count: u32,
    },
    Tools {
        count: u32,
    },
}

impl From<s::DigestPart> for DigestPart {
    fn from(p: s::DigestPart) -> Self {
        use s::DigestPart as P;
        match p {
            P::Edited {
                files,
                created,
                deleted,
                added,
                removed,
            } => Self::Edited {
                files,
                created,
                deleted,
                added,
                removed,
            },
            P::Read { files } => Self::Read { files },
            P::Ran { commands, failed } => Self::Ran { commands, failed },
            P::Subagents { count } => Self::Subagents { count },
            P::Tools { count } => Self::Tools { count },
        }
    }
}

/// What a turn did, counted.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Digest {
    pub files_changed: u32,
    pub files_created: u32,
    pub files_deleted: u32,
    pub lines_added: u32,
    pub lines_removed: u32,
    pub files_read: u32,
    pub commands: u32,
    pub commands_failed: u32,
    pub subagents: u32,
    pub tools: u32,
    /// The counts worth showing, in reading order.
    pub parts: Vec<DigestPart>,
    /// `parts` as one English line.
    pub summary: String,
}

impl From<s::Digest> for Digest {
    fn from(d: s::Digest) -> Self {
        Self {
            parts: d.parts().into_iter().map(Into::into).collect(),
            summary: d.summary(),
            files_changed: d.files_changed,
            files_created: d.files_created,
            files_deleted: d.files_deleted,
            lines_added: d.lines_added,
            lines_removed: d.lines_removed,
            files_read: d.files_read,
            commands: d.commands,
            commands_failed: d.commands_failed,
            subagents: d.subagents,
            tools: d.tools,
        }
    }
}

/// One turn: a prompt and everything up to its end.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Turn {
    pub index: u32,
    /// `None` for turns the harness started itself.
    pub prompt: Option<String>,
    /// What the agent said last in the turn.
    pub narrative: Option<String>,
    pub narrative_item: Option<String>,
    pub items: Vec<String>,
    /// Items folded under the prompt: all but prompts and the narrative.
    pub steps: u32,
    pub digest: Digest,
    pub outcome: TurnOutcome,
    pub open: bool,
    pub started_at_ms: Option<u64>,
    pub ended_at_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    /// The turn's unified diff, where the harness sends one.
    pub diff: Option<String>,
}

impl From<s::Turn> for Turn {
    fn from(t: s::Turn) -> Self {
        Self {
            duration_ms: t.duration_ms(),
            index: n(t.index),
            prompt: t.prompt,
            narrative: t.narrative,
            narrative_item: t.narrative_item,
            items: t.items,
            steps: t.steps,
            digest: t.digest.into(),
            outcome: t.outcome.into(),
            open: t.open,
            started_at_ms: t.started_at_ms,
            ended_at_ms: t.ended_at_ms,
            diff: t.diff,
        }
    }
}

/// An item as the session keeps it. A command's full output isn't here;
/// [`CommandEntry`] has its tail.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
    pub parent: Option<String>,
    pub turn: u32,
    pub status: ItemStatus,
    /// Message or reasoning text.
    pub text: String,
    pub detail: ItemDetail,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
}

impl From<s::ItemView> for Item {
    fn from(i: s::ItemView) -> Self {
        Self {
            id: i.id,
            kind: i.kind.into(),
            parent: i.parent,
            turn: n(i.turn),
            status: i.status.into(),
            text: i.text,
            detail: i.detail.into(),
            started_at_ms: i.started_at_ms,
            completed_at_ms: i.completed_at_ms,
        }
    }
}

/// What one call changed, so an app redraws only that. Ids are item ids;
/// files are keyed by path, turns by index.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SessionChanges {
    pub header: bool,
    pub now: bool,
    pub plan: bool,
    pub asks: bool,
    pub notices: bool,
    pub files: Vec<String>,
    pub commands: Vec<String>,
    pub subagents: Vec<String>,
    pub turns: Vec<u32>,
    pub items: Vec<String>,
}

impl From<s::Changes> for SessionChanges {
    fn from(c: s::Changes) -> Self {
        Self {
            header: c.header,
            now: c.now,
            plan: c.plan,
            asks: c.asks,
            notices: c.notices,
            files: c.files,
            commands: c.commands,
            subagents: c.subagents,
            turns: c.turns.into_iter().map(n).collect(),
            items: c.items,
        }
    }
}

/// Every region at once.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SessionSnapshot {
    pub header: SessionHeader,
    pub now: Vec<Activity>,
    pub plan: Option<PlanView>,
    pub asks: Vec<Ask>,
    pub files: Vec<FileEntry>,
    pub commands: Vec<CommandEntry>,
    pub subagents: Vec<SubagentEntry>,
    pub turns: Vec<Turn>,
    pub notices: Vec<String>,
    pub items: Vec<Item>,
}

impl From<s::Snapshot> for SessionSnapshot {
    fn from(v: s::Snapshot) -> Self {
        Self {
            header: v.header.into(),
            now: list(&v.now),
            plan: v.plan.map(Into::into),
            asks: list(&v.asks),
            files: list(&v.files),
            commands: list(&v.commands),
            subagents: list(&v.subagents),
            turns: list(&v.turns),
            notices: v.notices,
            items: list(&v.items),
        }
    }
}

/// A session's live state, built from its ACC events. Feed it every event
/// in order; it is safe to share between threads.
#[derive(Debug, Default, uniffi::Object)]
pub struct SessionView {
    inner: Mutex<acc_ui::SessionView>,
}

impl SessionView {
    fn lock(&self) -> MutexGuard<'_, acc_ui::SessionView> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[uniffi::export]
impl SessionView {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Rebuilds a session from ACC's transcript JSON (a session snapshot),
    /// for sessions opened after they started.
    #[uniffi::constructor]
    pub fn from_transcript(json: String) -> Result<Arc<Self>, AccUiError> {
        let view = acc_ui::SessionView::from_transcript(&json)
            .map_err(|e| invalid(format!("not a transcript: {e}")))?;
        Ok(Arc::new(Self {
            inner: Mutex::new(view),
        }))
    }

    /// Sets the session's working directory. Harnesses don't always say
    /// (Claude Code never does), so set it from how the session started.
    pub fn set_root(&self, root: String) {
        self.lock().set_root(root);
    }

    /// Applies one event as ACC serialises it. Unknown event types change
    /// nothing; JSON that isn't an event is an error.
    pub fn apply_json(&self, json: String) -> Result<SessionChanges, AccUiError> {
        self.lock()
            .apply_json(&json)
            .map(Into::into)
            .map_err(|e| invalid(format!("not an ACC event: {e}")))
    }

    /// Records that the user answered request `request_id`.
    pub fn answered(&self, request_id: String) -> SessionChanges {
        self.lock().answered(&request_id).into()
    }

    pub fn header(&self) -> SessionHeader {
        self.lock().header().clone().into()
    }

    /// What the agent and its subagents are doing right now, oldest first.
    pub fn now(&self) -> Vec<Activity> {
        list(&self.lock().now())
    }

    pub fn plan(&self) -> Option<PlanView> {
        self.lock().plan().cloned().map(Into::into)
    }

    pub fn asks(&self) -> Vec<Ask> {
        list(self.lock().asks())
    }

    pub fn ask(&self, request_id: String) -> Option<Ask> {
        self.lock().ask(&request_id).cloned().map(Into::into)
    }

    pub fn files(&self) -> Vec<FileEntry> {
        list(self.lock().files())
    }

    pub fn file(&self, path: String) -> Option<FileEntry> {
        self.lock().file(&path).cloned().map(Into::into)
    }

    pub fn commands(&self) -> Vec<CommandEntry> {
        list(self.lock().commands())
    }

    pub fn command(&self, item_id: String) -> Option<CommandEntry> {
        self.lock().command(&item_id).cloned().map(Into::into)
    }

    pub fn subagents(&self) -> Vec<SubagentEntry> {
        list(self.lock().subagents())
    }

    pub fn subagent(&self, item_id: String) -> Option<SubagentEntry> {
        self.lock().subagent(&item_id).cloned().map(Into::into)
    }

    pub fn turns(&self) -> Vec<Turn> {
        list(self.lock().turns())
    }

    pub fn turn(&self, index: u32) -> Option<Turn> {
        let index = usize::try_from(index).ok()?;
        self.lock().turn(index).cloned().map(Into::into)
    }

    pub fn notices(&self) -> Vec<String> {
        self.lock().notices().to_vec()
    }

    pub fn items(&self) -> Vec<Item> {
        list(self.lock().items())
    }

    pub fn item(&self, id: String) -> Option<Item> {
        self.lock().item(&id).cloned().map(Into::into)
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        self.lock().snapshot().into()
    }

    /// An item as a spec to render: a message's markdown as blocks, a
    /// command with its output, a diff, a folded reasoning or tool call.
    pub fn item_spec(&self, id: String) -> Option<NormalizedSpec> {
        self.lock().item_spec(&id).map(Into::into)
    }

    /// A waiting ask as a spec: a `request` node whose options answer it.
    pub fn ask_spec(&self, request_id: String) -> Option<NormalizedSpec> {
        let view = self.lock();
        let ask = view.ask(&request_id)?;
        Some(view.ask_spec(ask).into())
    }
}

/// What a link points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LinkKind {
    File,
    Pr,
    Commit,
    Url,
}

/// One link in a text, with its range as UTF-8 bytes (Rust, Swift's
/// `String.utf8`) and as UTF-16 code units (`NSRange`, Kotlin and Java
/// strings).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Link {
    pub start: u32,
    pub end: u32,
    pub start_utf16: u32,
    pub end_utf16: u32,
    pub kind: LinkKind,
    /// The URL, or the file's path (relative to the root when under it).
    pub target: String,
    /// What a markdown link shows; `None` for bare links.
    pub label: Option<String>,
    pub line: Option<u32>,
    /// `owner/name`, for pull requests and commits.
    pub repo: Option<String>,
    pub number: Option<u64>,
    pub sha: Option<String>,
}

fn utf16_at(text: &str, byte: usize) -> u32 {
    n(text.get(..byte).map_or(0, |s| s.encode_utf16().count()))
}

/// An agent's markdown as a spec: tables, checklists, callouts, diagrams
/// and code as blocks, the rest as markdown.
#[uniffi::export]
pub fn markdown_blocks(text: String) -> NormalizedSpec {
    acc_ui::markdown_blocks(&text).into()
}

/// The files, pull requests, commits and URLs in `text`, in order. `root`
/// is the session's working directory.
#[uniffi::export]
pub fn detect_links(text: String, root: Option<String>) -> Vec<Link> {
    acc_ui::detect_links(&text, root.as_deref())
        .into_iter()
        .map(|l| Link {
            start: n(l.start),
            end: n(l.end),
            start_utf16: utf16_at(&text, l.start),
            end_utf16: utf16_at(&text, l.end),
            kind: match l.kind {
                acc_ui::LinkKind::File => LinkKind::File,
                acc_ui::LinkKind::Pr => LinkKind::Pr,
                acc_ui::LinkKind::Commit => LinkKind::Commit,
                acc_ui::LinkKind::Url => LinkKind::Url,
            },
            target: l.target,
            label: l.label,
            line: l.line,
            repo: l.repo,
            number: l.number,
            sha: l.sha,
        })
        .collect()
}
