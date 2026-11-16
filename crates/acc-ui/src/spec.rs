//! The `acc-ui/1` spec as agents write it: a tree of components under one
//! root.
//!
//! These types are the single source of truth. The JSON Schema
//! ([`crate::schema`]), the skill's component catalog
//! ([`crate::skill_markdown`]) and validation ([`crate::normalize`]) are all
//! derived from them, so doc comments here are what the model reads.
//!
//! Only the canonical form is described. [`crate::normalize`] also accepts a
//! few shorthands models tend to write (an action as a bare string, chart
//! points as `[x, y]`, table cells as numbers) and rewrites them first.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Version tag every spec carries.
pub const VERSION: &str = "acc-ui/1";

/// A component for the user: a tree of nodes under `root`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "acc-ui/1 spec")]
pub struct Spec {
    /// Always `"acc-ui/1"`.
    pub v: String,
    /// Stable id for this component, 1-64 of `A-Z a-z 0-9 . _ -`. Required for
    /// `acc_ui_ask`, where the answer is keyed by it; make it unique in the
    /// session (`deploy-target-2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^[A-Za-z0-9._-]{1,64}$"))]
    pub id: Option<String>,
    /// Short heading shown above the component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Plain text shown where the component can't be rendered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    pub root: Node,
}

/// One component. `type` picks which.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Node {
    // Layout
    /// Children top to bottom.
    Stack(Stack),
    /// Children side by side.
    Row(Row),
    /// A titled box around its children.
    Card(Card),

    // Data
    /// Label/value pairs.
    Kv(Kv),
    /// Rows and columns of text.
    Table(Table),

    // Status
    /// A small coloured label.
    Badge(Badge),
    /// A progress bar.
    Progress(Progress),
    /// Steps with a state each.
    Checklist(Checklist),
    /// A highlighted note, warning or error.
    Callout(Callout),

    // Links
    /// A file on the host, opened on tap.
    File(File),
    /// A pull request.
    Pr(Pr),
    /// A commit.
    Commit(Commit),

    // Content
    /// A line, bar, area, point or pie chart.
    Chart(Chart),
    /// A Mermaid diagram.
    Mermaid(Mermaid),
    /// An image.
    Image(Image),
    /// Markdown text.
    Markdown(Markdown),
    /// Self-contained HTML in a sandbox: no network, no navigation.
    Html(Html),
    /// Source code or other preformatted text.
    Code(Code),
    /// A summary line whose children show when the user opens it.
    Disclosure(Disclosure),

    // Made by acc-ui from what the agent did; kept out of the schema
    // agents see.
    /// A command's output.
    #[schemars(skip)]
    Output(Output),
    /// Changes to files.
    #[schemars(skip)]
    Diff(Diff),
    /// An approval or question from the harness, answered through ACC.
    /// Only acc-ui makes these: specs from agents can't, so an agent can't
    /// dress up a prompt that answers a real request.
    #[schemars(skip)]
    Request(Request),

    // Inputs
    /// A button that runs an action when tapped.
    Button(Button),
    /// Quick-reply chips; each runs its action when tapped.
    Chips(Chips),
    /// Input fields with a submit button.
    Form(Form),
}

impl Node {
    /// The `type` tag of each node kind, in catalog order.
    pub const TYPES: &[&str] = &[
        "stack",
        "row",
        "card",
        "kv",
        "table",
        "badge",
        "progress",
        "checklist",
        "callout",
        "file",
        "pr",
        "commit",
        "chart",
        "mermaid",
        "image",
        "markdown",
        "html",
        "code",
        "disclosure",
        "output",
        "diff",
        "request",
        "button",
        "chips",
        "form",
    ];

    /// Types made only by acc-ui, left out of what agents are told.
    pub const INTERNAL_TYPES: &[&str] = &["output", "diff", "request"];

    /// Child nodes of a container.
    pub fn children(&self) -> &[Node] {
        match self {
            Node::Stack(n) => &n.children,
            Node::Row(n) => &n.children,
            Node::Card(n) => &n.children,
            Node::Disclosure(n) => &n.children,
            _ => &[],
        }
    }

    /// Whether the node takes input from the user.
    pub fn is_input(&self) -> bool {
        matches!(
            self,
            Node::Button(_) | Node::Chips(_) | Node::Form(_) | Node::Request(_)
        ) || matches!(self, Node::Card(c) if c.action.is_some())
    }
}

/// Spacing between children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Gap {
    None,
    Small,
    Medium,
    Large,
}

/// Colour role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
    Accent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stack {
    #[serde(default)]
    pub children: Vec<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<Gap>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Row {
    #[serde(default)]
    pub children: Vec<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<Gap>,
    /// Wrap onto more lines when the row is too narrow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Card {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// One emoji shown before the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(default)]
    pub children: Vec<Node>,
    /// Runs when the card is tapped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Kv {
    pub items: Vec<KvItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KvItem {
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Table {
    pub columns: Vec<Column>,
    /// One array of cells per row, in column order. Cells may hold inline
    /// markdown (`code`, **bold**, links).
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Column {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Badge {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Progress {
    /// Progress so far, from 0 to `max`.
    pub value: f64,
    /// Defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Checklist {
    pub items: Vec<CheckItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CheckItem {
    pub text: String,
    pub state: CheckState,
    /// A second line under the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Todo,
    Doing,
    Done,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Callout {
    /// Markdown.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct File {
    /// Absolute path, or relative to the session's working directory.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// What the file is, shown under the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pr {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// `owner/name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PrState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Commit {
    pub sha: String,
    /// First line of the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Chart {
    /// A `pie` uses the first series, with each point's `x` as a slice label.
    pub kind: ChartKind,
    pub series: Vec<Series>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_label: Option<String>,
    /// Stack bar and area series instead of overlapping them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stacked: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Line,
    Bar,
    Area,
    Point,
    Pie,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Series {
    /// Legend label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub points: Vec<Point>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Point {
    /// A number, a category label, or an ISO 8601 date.
    pub x: XValue,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum XValue {
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Mermaid {
    /// Mermaid source: flowchart, sequence, state, class or ER diagram.
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Image {
    /// An `https:` URL, a `data:image/...` URL, or an absolute path on the host.
    pub src: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Markdown {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Code {
    pub text: String,
    /// The fence's language (`rust`, `sh`, `diff`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Disclosure {
    /// The line shown while it's closed.
    pub summary: String,
    #[serde(default)]
    pub children: Vec<Node>,
    /// Starts open. Defaults to closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Output {
    /// The output's last lines, as the terminal showed them.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Lines before `text` that were left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omitted_lines: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Diff {
    pub files: Vec<DiffFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DiffFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub change: FileChange,
    #[serde(default)]
    pub added: u32,
    #[serde(default)]
    pub removed: u32,
    #[serde(default)]
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileChange {
    Add,
    Update,
    Delete,
    Rename,
}

/// A region of a diff. Line numbers are 1-based; 0 means unknown (an edit
/// given as old and new snippets, without its place in the file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DiffHunk {
    #[serde(default)]
    pub old_start: u32,
    #[serde(default)]
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DiffLine {
    pub tag: LineTag,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineTag {
    Context,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Request {
    /// ACC's id for the request.
    pub request_id: String,
    pub kind: RequestKind,
    pub title: String,
    /// What it's about: the command, the files and their changes, or the
    /// tool and its target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub options: Vec<RequestOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    Permission,
    Question,
    Auth,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RequestOption {
    pub label: String,
    /// Runs [`Action::Answer`] with this request and option.
    pub action: Action,
    /// Allows or rejects, once or always, when the harness says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<OptionRole>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptionRole {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Html {
    /// A full document or a fragment. Inline all CSS, JS and images: the
    /// sandbox fetches nothing and opens links only after the user confirms.
    pub html: String,
    /// Preferred height in points; the content's own height is used if unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Button {
    pub label: String,
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<ButtonStyle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Chips {
    pub options: Vec<Chip>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Chip {
    pub label: String,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Form {
    pub fields: Vec<Field>,
    /// Defaults to "Submit".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_label: Option<String>,
}

/// A form input. Its value is submitted under `name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Field {
    /// Key of the value in the submitted object: `A-Z a-z 0-9 _ -`.
    #[schemars(regex(pattern = r"^[A-Za-z0-9_-]{1,64}$"))]
    pub name: String,
    pub kind: FieldKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Help text under the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Initial value: a string, number or boolean matching `kind`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// The choices of a `select`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<Choice>>,
    /// Lower bound of a `number` or `slider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Upper bound of a `number` or `slider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// A `select` that allows several choices; its value is an array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiple: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// One line of text.
    Text,
    /// Several lines of text.
    Textarea,
    Number,
    /// On/off; the value is a boolean.
    Toggle,
    /// One of `options` (several with `multiple`).
    Select,
    /// An ISO 8601 date, `YYYY-MM-DD`.
    Date,
    /// A number between `min` and `max`.
    Slider,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Choice {
    pub label: String,
    /// Submitted value; defaults to the label.
    #[serde(default)]
    pub value: String,
}

/// What a tap does. Nothing runs without the user tapping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Send `text` as the user's next message.
    Prompt { text: String },
    /// Answer this component: with `values`, merged over any form's values.
    /// Answers an `acc_ui_ask`, or comes back as the user's next message.
    Submit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        values: Option<Map<String, Value>>,
    },
    /// Open a web page after the user confirms.
    OpenUrl { url: String },
    /// Open a file on the host.
    OpenFile {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line: Option<u32>,
    },
    /// Copy `text` to the clipboard.
    Copy { text: String },
    /// Answer a harness request through ACC. Only acc-ui makes these.
    #[schemars(skip)]
    Answer { request_id: String, option_id: String },
}
