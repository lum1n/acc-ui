//! ACC's events as acc-ui reads them: its own tolerant copy of the wire
//! format, so acc-ui doesn't depend on `acc-core` and the two release
//! separately.
//!
//! Every field has a default and unknown event types, item kinds and detail
//! types parse as `Unknown`, so a newer ACC never breaks an older acc-ui.
//! Only what the session model reads is kept; `native` payloads are ignored
//! except a permission request's, which holds its questions and plan.
//! The recorded sessions in `tests/sessions` catch drift.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One ACC event (`acc_core::event::Event`, serialised).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    SessionStarted {
        #[serde(default)]
        harness: String,
        #[serde(default)]
        native_id: String,
        #[serde(default)]
        model: Option<String>,
    },
    StateChanged {
        #[serde(default)]
        state: RunState,
        #[serde(default)]
        message: Option<String>,
        /// When ACC observed it, ms since the Unix epoch.
        #[serde(default)]
        observed_at_ms: Option<u64>,
    },
    ItemStarted {
        id: String,
        #[serde(default)]
        kind: ItemKind,
        #[serde(default)]
        parent: Option<String>,
        #[serde(default)]
        detail: Detail,
        #[serde(default)]
        started_at_ms: Option<u64>,
        /// A user message's images, from ACC's refs in `native.images`.
        #[serde(default, rename = "native", deserialize_with = "native_images")]
        images: Vec<ImageRef>,
    },
    ItemDelta {
        id: String,
        #[serde(default)]
        text: String,
    },
    ItemUpdated {
        id: String,
        #[serde(default)]
        detail: Detail,
    },
    ItemCompleted {
        id: String,
        #[serde(default)]
        status: ItemStatus,
        #[serde(default)]
        detail: Detail,
        #[serde(default)]
        completed_at_ms: Option<u64>,
    },
    PermissionRequested {
        request_id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        options: Vec<AskOption>,
        #[serde(default)]
        kind: AskKind,
        #[serde(default)]
        item_id: Option<String>,
        #[serde(default)]
        detail: Option<Detail>,
        /// The harness's own request: questions and plans live here.
        #[serde(default)]
        native: Value,
    },
    PermissionResolved {
        request_id: String,
    },
    Usage(Usage),
    Quota(Quota),
    TitleChanged {
        #[serde(default)]
        title: String,
    },
    Notice {
        #[serde(default)]
        message: String,
    },
    Plan(Plan),
    TurnDiff {
        #[serde(default)]
        turn_id: Option<String>,
        #[serde(default)]
        diff: String,
    },
    TurnCompleted,
    Exited {
        #[serde(default)]
        code: Option<i32>,
    },
    /// Anything else: palette and option changes, panels, native frames,
    /// and event types newer than this acc-ui.
    #[serde(other)]
    Unknown,
}

/// One image a message carried, by reference (`acc_core::image::ImageRef`).
/// The bytes are not in events: an app shows an image it sent itself by
/// `sha256`, or reads `path` from the host.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// Decoded size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Lowercase hex SHA-256 of the bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The image's file on the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl ImageRef {
    /// An image node `src` for it: the path, else `sha256:<hex>`.
    pub fn src(&self) -> Option<String> {
        self.path
            .clone()
            .or_else(|| self.sha256.as_ref().map(|h| format!("sha256:{h}")))
    }
}

/// `native.images`, never failing the event: anything malformed is left out.
fn native_images<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<ImageRef>, D::Error> {
    let native = Value::deserialize(d)?;
    Ok(native
        .get("images")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| ImageRef::deserialize(v).ok())
                .filter(|r| *r != ImageRef::default())
                .collect()
        })
        .unwrap_or_default())
}

/// What a session is doing (`acc_core::state::RunState`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunState {
    #[default]
    Idle,
    Working {
        #[serde(default)]
        progress: Option<u8>,
    },
    Blocked {
        #[serde(default)]
        kind: AskKind,
    },
    Done,
    /// The turn ended but background work (a background subagent) will
    /// start another one on its own.
    Waiting,
    Error,
    Exited,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    UserMessage,
    AgentMessage,
    Reasoning,
    ToolCall,
    CommandExecution,
    FileChange,
    Subagent,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    #[default]
    Running,
    Completed,
    Failed,
}

/// An item's harness-independent view (`acc_core::item::ItemDetail`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Detail {
    /// A message or reasoning; the content arrives as deltas.
    #[default]
    Text,
    Tool {
        #[serde(default)]
        tool: String,
        #[serde(default)]
        kind: ToolKind,
        /// One line on what the tool acts on (path, pattern, URL).
        #[serde(default)]
        title: String,
        #[serde(default)]
        input: Value,
        #[serde(default)]
        output: Option<String>,
        #[serde(default)]
        paths: Vec<String>,
    },
    Command {
        #[serde(default)]
        tool: String,
        #[serde(default)]
        command: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        output: String,
        #[serde(default)]
        exit_code: Option<i32>,
    },
    FileChange {
        #[serde(default)]
        tool: String,
        #[serde(default)]
        files: Vec<FileDiff>,
    },
    Subagent {
        #[serde(default)]
        tool: String,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        description: String,
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        child: Option<String>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        output: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        background: bool,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Shell,
    Edit,
    Read,
    Fetch,
    Mcp,
    #[default]
    #[serde(other)]
    Other,
}

/// One file's change (`acc_core::item::FileDiff`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    #[serde(default)]
    pub kind: FileChangeKind,
    #[serde(default)]
    pub hunks: Vec<Hunk>,
    #[serde(default)]
    pub added: u32,
    #[serde(default)]
    pub removed: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Add,
    Delete,
    Rename,
    #[default]
    #[serde(other)]
    Update,
}

/// A region of a diff. Line numbers are 1-based; 0 means unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    #[serde(default)]
    pub old_start: u32,
    #[serde(default)]
    pub old_lines: u32,
    #[serde(default)]
    pub new_start: u32,
    #[serde(default)]
    pub new_lines: u32,
    #[serde(default)]
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    #[serde(default)]
    pub tag: LineTag,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineTag {
    #[default]
    Context,
    Add,
    Remove,
}

/// Why a session waits on the user (`acc_core::state::BlockKind`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    #[default]
    Permission,
    Question,
    Auth,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskOption {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<OptionKind>,
}

/// What choosing an [`AskOption`] does, so apps can tell the approving
/// option from the declining one and mark the ones that stick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    #[serde(other)]
    Other,
}

impl OptionKind {
    pub fn allows(self) -> bool {
        matches!(self, Self::AllowOnce | Self::AllowAlways)
    }

    pub fn rejects(self) -> bool {
        matches!(self, Self::RejectOnce | Self::RejectAlways)
    }

    /// Whether the choice also applies to later requests like this one.
    pub fn persists(self) -> bool {
        matches!(self, Self::AllowAlways | Self::RejectAlways)
    }

    /// The ACC name, `allow_once` and so on; `None` for [`Self::Other`].
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Self::AllowOnce => Some("allow_once"),
            Self::AllowAlways => Some("allow_always"),
            Self::RejectOnce => Some("reject_once"),
            Self::RejectAlways => Some("reject_always"),
            Self::Other => None,
        }
    }
}

/// The harness's usage report. Cost is a running total everywhere; tokens
/// are per turn on some harnesses and running totals on others, so they're
/// shown as reported, never summed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Quota {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default)]
    pub windows: Vec<QuotaWindow>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QuotaWindow {
    #[serde(default)]
    pub id: String,
    /// 0-100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<u64>,
    /// Seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// The agent's to-do list for the main conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub entries: Vec<PlanEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub status: PlanStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    InProgress,
    Completed,
    #[default]
    #[serde(other)]
    Pending,
}
