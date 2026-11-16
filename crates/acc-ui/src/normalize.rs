//! Validation and normalisation: an agent's spec in, a flat, canonical spec
//! out, plus every problem found.
//!
//! Normalising is lenient. A node that is broken (unknown type, missing
//! field, unsafe URL) is dropped with an error and the rest still renders.
//! Only a spec with nothing renderable left, or one that is too big, comes
//! back without a spec.
//!
//! The output is flat so renderers never recurse into untrusted depth:
//!
//! ```json
//! {"v":"acc-ui/1","id":"deploy","title":"Deploy","fallback":"...",
//!  "interactive":true,"root":0,
//!  "nodes":[{"type":"card","title":"Deploy","children":[1]},{"type":"badge","text":"ok"}]}
//! ```
//!
//! Nodes are in pre-order, so the root is always 0 and a child's index is
//! always greater than its parent's. Containers carry `children` as indices.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::spec::{Action, ChartKind, CheckState, FieldKind, Node, VERSION};

/// Largest spec accepted, in bytes of JSON.
pub const MAX_SPEC_BYTES: usize = 256 * 1024;
/// Most nodes in one spec; later nodes are dropped.
pub const MAX_NODES: usize = 400;
/// Deepest nesting of containers; deeper nodes are dropped.
pub const MAX_DEPTH: usize = 12;
/// Largest `html` node, in bytes.
pub const MAX_HTML_BYTES: usize = 128 * 1024;
/// Most rows in one table.
pub const MAX_TABLE_ROWS: usize = 500;
/// Most points in one chart, over all series.
pub const MAX_CHART_POINTS: usize = 5000;
/// Largest `output` text, in bytes; earlier output is dropped.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// Most lines in one `diff`, over all its files; later lines are dropped.
pub const MAX_DIFF_LINES: usize = 5000;
/// Most fields in one form, options in one select, chips in one row.
pub const MAX_CHOICES: usize = 50;
/// Longest fallback text generated from the nodes.
const MAX_FALLBACK_CHARS: usize = 4000;

/// A normalised spec and the problems found while making it.
#[derive(Debug, Clone, PartialEq)]
pub struct Normalized {
    /// The flat spec, or `None` when nothing could be rendered.
    pub spec: Option<Value>,
    /// One line per problem: `"/root/children/2: unknown type \"gauge\""`.
    pub errors: Vec<String>,
}

impl Normalized {
    /// The spec as compact JSON.
    pub fn spec_json(&self) -> Option<String> {
        self.spec.as_ref().map(Value::to_string)
    }

    /// The spec's id, when there is a spec.
    pub fn id(&self) -> Option<&str> {
        self.spec.as_ref()?.get("id")?.as_str()
    }

    fn failed(error: impl Into<String>) -> Self {
        Normalized {
            spec: None,
            errors: vec![error.into()],
        }
    }
}

/// Normalises a spec given as JSON text.
pub fn normalize(json: &str) -> Normalized {
    if json.len() > MAX_SPEC_BYTES {
        return Normalized::failed(format!(
            "spec is {} bytes; the limit is {MAX_SPEC_BYTES}",
            json.len()
        ));
    }
    match serde_json::from_str::<Value>(json) {
        Ok(v) => normalize_value(v),
        Err(e) => Normalized::failed(format!("not valid JSON: {e}")),
    }
}

/// Normalises a parsed spec. A bare node (an object with `type` but no
/// `root`) is taken as the root of a spec without id.
pub fn normalize_value(spec: Value) -> Normalized {
    normalize_with(spec, false)
}

/// Normalises a spec acc-ui made itself, which may hold `request` nodes and
/// `answer` actions. Specs from agents never come through here.
pub(crate) fn normalize_trusted(spec: Value) -> Normalized {
    normalize_with(spec, true)
}

fn normalize_with(spec: Value, trusted: bool) -> Normalized {
    let size = spec.to_string().len();
    if size > MAX_SPEC_BYTES {
        return Normalized::failed(format!("spec is {size} bytes; the limit is {MAX_SPEC_BYTES}"));
    }
    let Value::Object(mut obj) = spec else {
        return Normalized::failed("spec must be a JSON object");
    };
    if !obj.contains_key("root") && obj.contains_key("type") {
        obj = Map::from_iter([("root".to_owned(), Value::Object(obj))]);
    }
    match obj.get("v") {
        None => {}
        Some(Value::String(v)) if v == VERSION => {}
        Some(other) => {
            return Normalized::failed(format!("/v: expected \"{VERSION}\", got {other}"));
        }
    }
    let Some(root) = obj.remove("root") else {
        return Normalized::failed("/root: missing");
    };

    let mut w = Walker {
        trusted,
        ..Walker::default()
    };
    let root_index = w.node(root, 0, "/root".to_owned());
    if root_index.is_none() {
        w.errors.push("/root: no renderable node left".to_owned());
        return Normalized {
            spec: None,
            errors: w.errors,
        };
    }

    let id = match obj.get("id") {
        Some(Value::String(s)) if valid_id(s) => s.clone(),
        Some(other) => {
            let fixed = match other {
                Value::String(s) => sanitize_id(s),
                _ => None,
            };
            w.errors
                .push(format!("/id: {other} is not 1-64 of A-Z a-z 0-9 . _ -"));
            fixed.unwrap_or_else(|| content_id(&w.nodes))
        }
        None => content_id(&w.nodes),
    };
    let title = text(&obj, "title");
    let fallback = text(&obj, "fallback").unwrap_or_else(|| w.fallback(title.as_deref()));

    let mut out = Map::new();
    out.insert("v".to_owned(), VERSION.into());
    out.insert("id".to_owned(), id.into());
    if let Some(t) = title {
        out.insert("title".to_owned(), t.into());
    }
    out.insert("fallback".to_owned(), fallback.into());
    out.insert("interactive".to_owned(), w.interactive.into());
    out.insert("root".to_owned(), 0.into());
    out.insert("nodes".to_owned(), Value::Array(w.nodes));
    Normalized {
        spec: Some(Value::Object(out)),
        errors: w.errors,
    }
}

/// Whether `id` is a valid spec id: 1-64 of `A-Z a-z 0-9 . _ -`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && id != "."
        && id != ".."
}

fn sanitize_id(id: &str) -> Option<String> {
    let s: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    let s = s.trim_matches('-').to_owned();
    valid_id(&s).then_some(s)
}

fn content_id(nodes: &[Value]) -> String {
    let digest = Sha256::digest(Value::Array(nodes.to_vec()).to_string().as_bytes());
    let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("u-{hex}")
}

fn text(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[derive(Default)]
struct Walker {
    nodes: Vec<Value>,
    /// Fallback lines, indented by depth.
    lines: Vec<String>,
    errors: Vec<String>,
    interactive: bool,
    over_limit: bool,
    /// The spec is acc-ui's own: requests and answers are allowed.
    trusted: bool,
    /// Diff lines kept so far, against [`MAX_DIFF_LINES`].
    diff_lines: usize,
}

impl Walker {
    /// Adds a node and its children, returning its index, or `None` when it
    /// was dropped.
    fn node(&mut self, raw: Value, depth: usize, path: String) -> Option<usize> {
        if self.nodes.len() >= MAX_NODES {
            if !self.over_limit {
                self.over_limit = true;
                self.errors.push(format!(
                    "{path}: more than {MAX_NODES} nodes; the rest are dropped"
                ));
            }
            return None;
        }
        if depth > MAX_DEPTH {
            self.errors
                .push(format!("{path}: nested deeper than {MAX_DEPTH}; dropped"));
            return None;
        }
        let Value::Object(mut obj) = raw else {
            self.errors.push(format!("{path}: a node must be an object"));
            return None;
        };
        let ty = match obj.get("type").and_then(Value::as_str) {
            Some(t) => alias(t).to_owned(),
            None => {
                self.errors.push(format!("{path}: missing \"type\""));
                return None;
            }
        };
        if !Node::TYPES.contains(&ty.as_str()) {
            self.errors.push(format!(
                "{path}: unknown type {ty:?}; expected one of {}",
                Node::TYPES
                    .iter()
                    .filter(|t| !Node::INTERNAL_TYPES.contains(t))
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            return None;
        }
        obj.insert("type".to_owned(), ty.clone().into());
        let children = match obj.remove("children") {
            Some(Value::Array(c)) => c,
            Some(_) => {
                self.errors.push(format!("{path}/children: must be an array"));
                Vec::new()
            }
            None => Vec::new(),
        };
        desugar(&ty, &mut obj);
        let mut node: Node = match serde_json::from_value(Value::Object(obj)) {
            Ok(n) => n,
            Err(e) => {
                self.errors.push(format!("{path}: {e}"));
                return None;
            }
        };
        if let Err(e) = self.check(&mut node, &path) {
            self.errors.push(format!("{path}: {e}"));
            return None;
        }
        self.interactive |= node.is_input();

        let index = self.nodes.len();
        self.nodes.push(Value::Null);
        self.describe(&node, depth);
        let container = matches!(
            node,
            Node::Stack(_) | Node::Row(_) | Node::Card(_) | Node::Disclosure(_)
        );
        if !container && !children.is_empty() {
            self.errors
                .push(format!("{path}/children: a {ty} has no children; ignored"));
        }
        let mut ids = Vec::new();
        if container {
            for (i, child) in children.into_iter().enumerate() {
                if let Some(id) = self.node(child, depth + 1, format!("{path}/children/{i}")) {
                    ids.push(Value::from(id));
                }
            }
        }
        let mut value = match serde_json::to_value(&node) {
            Ok(v) => v,
            Err(e) => {
                self.errors.push(format!("{path}: {e}"));
                Value::Null
            }
        };
        if container && let Value::Object(o) = &mut value {
            o.insert("children".to_owned(), Value::Array(ids));
        }
        self.nodes[index] = value;
        Some(index)
    }

    /// Checks what the types can't say, fixing what it can. An error drops
    /// the node.
    fn check(&mut self, node: &mut Node, path: &str) -> Result<(), String> {
        match node {
            Node::Card(c) => {
                if let Some(a) = &c.action
                    && let Err(e) = check_action(self.trusted, a)
                {
                    self.errors.push(format!("{path}/action: {e}; dropped"));
                    c.action = None;
                }
            }
            Node::Table(t) => {
                if t.columns.is_empty() {
                    return Err("a table needs columns".into());
                }
                if t.rows.len() > MAX_TABLE_ROWS {
                    self.errors.push(format!(
                        "{path}/rows: {} rows; only the first {MAX_TABLE_ROWS} are kept",
                        t.rows.len()
                    ));
                    t.rows.truncate(MAX_TABLE_ROWS);
                }
                let n = t.columns.len();
                for row in &mut t.rows {
                    row.resize(n, String::new());
                }
            }
            Node::Progress(p) => {
                let max = p.max.filter(|m| m.is_finite() && *m > 0.0).unwrap_or(1.0);
                if !p.value.is_finite() {
                    return Err("value must be a number".into());
                }
                p.value = p.value.clamp(0.0, max);
                p.max = Some(max);
            }
            Node::Checklist(c) if c.items.is_empty() => return Err("a checklist needs items".into()),
            Node::Chart(c) => {
                c.series.retain(|s| !s.points.is_empty());
                if c.series.is_empty() {
                    return Err("a chart needs at least one series with points".into());
                }
                if c.kind == ChartKind::Pie && c.series.len() > 1 {
                    self.errors
                        .push(format!("{path}/series: a pie uses only the first series"));
                    c.series.truncate(1);
                }
                let mut budget = MAX_CHART_POINTS;
                let total: usize = c.series.iter().map(|s| s.points.len()).sum();
                if total > MAX_CHART_POINTS {
                    self.errors.push(format!(
                        "{path}/series: {total} points; only the first {MAX_CHART_POINTS} are kept"
                    ));
                }
                for s in &mut c.series {
                    s.points.retain(|p| p.y.is_finite());
                    s.points.truncate(budget);
                    budget -= s.points.len();
                }
            }
            Node::Mermaid(m) if m.code.trim().is_empty() => return Err("empty diagram".into()),
            Node::Image(i) => check_image_src(&i.src)?,
            Node::Html(h) => {
                if h.html.len() > MAX_HTML_BYTES {
                    return Err(format!(
                        "html is {} bytes; the limit is {MAX_HTML_BYTES}",
                        h.html.len()
                    ));
                }
                if h.html.trim().is_empty() {
                    return Err("empty html".into());
                }
            }
            Node::Button(b) => {
                if b.label.trim().is_empty() {
                    return Err("a button needs a label".into());
                }
                check_action(self.trusted, &b.action)?;
            }
            Node::Chips(c) => {
                let mut i = 0;
                c.options.retain(|chip| {
                    let r = check_action(self.trusted, &chip.action);
                    if let Err(e) = &r {
                        self.errors
                            .push(format!("{path}/options/{i}/action: {e}; dropped"));
                    }
                    i += 1;
                    r.is_ok() && !chip.label.trim().is_empty()
                });
                c.options.truncate(MAX_CHOICES);
                if c.options.is_empty() {
                    return Err("chips need at least one option".into());
                }
            }
            Node::Code(c) if c.text.is_empty() => return Err("empty code".into()),
            Node::Disclosure(d) if d.summary.trim().is_empty() => {
                return Err("a disclosure needs a summary".into());
            }
            Node::Output(o) => {
                if o.text.len() > MAX_OUTPUT_BYTES {
                    let mut cut = o.text.len() - MAX_OUTPUT_BYTES;
                    while !o.text.is_char_boundary(cut) {
                        cut += 1;
                    }
                    let dropped = o.text[..cut].lines().count();
                    o.text.drain(..cut);
                    let omitted = o.omitted_lines.unwrap_or(0);
                    o.omitted_lines =
                        Some(omitted.saturating_add(u32::try_from(dropped).unwrap_or(u32::MAX)));
                }
            }
            Node::Diff(d) => {
                if d.files.is_empty() {
                    return Err("a diff needs files".into());
                }
                for f in &mut d.files {
                    for h in &mut f.hunks {
                        let room = MAX_DIFF_LINES.saturating_sub(self.diff_lines);
                        if h.lines.len() > room {
                            h.lines.truncate(room);
                            if !self.over_limit {
                                self.errors.push(format!(
                                    "{path}: more than {MAX_DIFF_LINES} diff lines; the rest are dropped"
                                ));
                            }
                        }
                        self.diff_lines += h.lines.len();
                    }
                    f.hunks.retain(|h| !h.lines.is_empty());
                }
            }
            Node::Request(r) => {
                if !self.trusted {
                    return Err("only acc-ui makes requests".into());
                }
                let id = r.request_id.clone();
                r.options
                    .retain(|o| matches!(&o.action, Action::Answer { request_id, .. } if *request_id == id));
                if r.options.is_empty() && r.questions.is_empty() {
                    return Err("a request needs options or questions that answer it".into());
                }
            }
            Node::Form(f) => {
                if f.fields.is_empty() {
                    return Err("a form needs fields".into());
                }
                if f.fields.len() > MAX_CHOICES {
                    return Err(format!("more than {MAX_CHOICES} fields"));
                }
                let mut seen = std::collections::HashSet::new();
                for (i, field) in f.fields.iter_mut().enumerate() {
                    let fp = format!("fields/{i}");
                    if !valid_field_name(&field.name) {
                        return Err(format!(
                            "{fp}/name: {:?} is not 1-64 of A-Z a-z 0-9 _ -",
                            field.name
                        ));
                    }
                    if !seen.insert(field.name.clone()) {
                        return Err(format!("{fp}/name: {:?} is used twice", field.name));
                    }
                    match field.kind {
                        FieldKind::Select => {
                            let options = field.options.get_or_insert_default();
                            if options.is_empty() {
                                return Err(format!("{fp}: a select needs options"));
                            }
                            options.truncate(MAX_CHOICES);
                        }
                        FieldKind::Slider if field.min.is_none() || field.max.is_none() => {
                            return Err(format!("{fp}: a slider needs min and max"));
                        }
                        _ => {}
                    }
                    if let (Some(lo), Some(hi)) = (field.min, field.max)
                        && lo > hi
                    {
                        return Err(format!("{fp}: min is above max"));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Adds the node's fallback lines.
    fn describe(&mut self, node: &Node, depth: usize) {
        let pad = "  ".repeat(depth.saturating_sub(1));
        let mut push = |s: String| {
            if !s.trim().is_empty() {
                self.lines.push(format!("{pad}{s}"));
            }
        };
        match node {
            Node::Stack(_) | Node::Row(_) => {}
            Node::Card(c) => {
                let head = [c.icon.as_deref(), c.title.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ");
                match &c.subtitle {
                    Some(s) if !head.is_empty() => push(format!("{head}: {s}")),
                    Some(s) => push(s.clone()),
                    None => push(head),
                }
            }
            Node::Kv(kv) => kv
                .items
                .iter()
                .for_each(|i| push(format!("{}: {}", i.label, i.value))),
            Node::Table(t) => {
                push(
                    t.columns
                        .iter()
                        .map(|c| c.label.as_str())
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
                t.rows.iter().for_each(|r| push(r.join(" | ")));
            }
            Node::Badge(b) => push(format!("[{}]", b.text)),
            Node::Progress(p) => {
                let pct = (p.value / p.max.unwrap_or(1.0) * 100.0).round();
                push(format!("{} {pct}%", p.label.as_deref().unwrap_or("Progress")));
            }
            Node::Checklist(c) => c.items.iter().for_each(|i| {
                let mark = match i.state {
                    CheckState::Todo => "[ ]",
                    CheckState::Doing => "[~]",
                    CheckState::Done => "[x]",
                    CheckState::Failed => "[!]",
                    CheckState::Skipped => "[-]",
                };
                push(format!("{mark} {}", i.text));
            }),
            Node::Callout(c) => match &c.title {
                Some(t) => push(format!("{t}: {}", c.text)),
                None => push(c.text.clone()),
            },
            Node::File(f) => match f.line {
                Some(l) => push(format!("{}:{l}", f.path)),
                None => push(f.path.clone()),
            },
            Node::Pr(p) => {
                let num = p.number.map(|n| format!("#{n} ")).unwrap_or_default();
                let url = p.url.as_deref().map(|u| format!(" {u}")).unwrap_or_default();
                push(format!("PR {num}{}{url}", p.title));
            }
            Node::Commit(c) => {
                let sha: String = c.sha.chars().take(8).collect();
                push(format!("{sha} {}", c.title.as_deref().unwrap_or("")));
            }
            Node::Chart(c) => push(format!(
                "[chart{}]",
                c.title.as_deref().map(|t| format!(": {t}")).unwrap_or_default()
            )),
            Node::Mermaid(_) => push("[diagram]".to_owned()),
            Node::Image(i) => push(format!("[image: {}]", i.alt.as_deref().unwrap_or(&i.src))),
            Node::Markdown(m) => push(m.text.clone()),
            Node::Html(_) => push("[HTML]".to_owned()),
            Node::Code(c) => c.text.lines().for_each(|l| push(format!("    {l}"))),
            Node::Disclosure(d) => push(format!("▸ {}", d.summary)),
            Node::Output(o) => {
                let mut lines: Vec<&str> = o.text.lines().collect();
                let start = lines.len().saturating_sub(10);
                lines.drain(..start);
                lines.into_iter().for_each(|l| push(format!("    {l}")));
                if let Some(c) = o.exit_code {
                    push(format!("(exit {c})"));
                }
            }
            Node::Diff(d) => d.files.iter().for_each(|f| {
                push(format!("{} +{} −{}", f.path, f.added, f.removed));
            }),
            Node::Request(r) => {
                push(r.title.clone());
                if let Some(plan) = &r.plan {
                    push(plan.clone());
                }
                for q in &r.questions {
                    let choices: Vec<&str> = q.options.iter().map(|o| o.label.as_str()).collect();
                    push(if choices.is_empty() {
                        q.text.clone()
                    } else {
                        format!("{} ({})", q.text, choices.join(" / "))
                    });
                }
                push(
                    r.options
                        .iter()
                        .map(|o| format!("[{}]", o.label))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            Node::Button(b) => push(format!("[{}]", b.label)),
            Node::Chips(c) => push(
                c.options
                    .iter()
                    .map(|o| format!("[{}]", o.label))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            Node::Form(f) => f.fields.iter().for_each(|field| {
                push(format!("{}: ____", field.label.as_deref().unwrap_or(&field.name)));
            }),
        }
    }

    fn fallback(&self, title: Option<&str>) -> String {
        let mut s = String::new();
        if let Some(t) = title {
            s.push_str(t);
            s.push('\n');
        }
        s.push_str(&self.lines.join("\n"));
        if s.chars().count() > MAX_FALLBACK_CHARS {
            s = s.chars().take(MAX_FALLBACK_CHARS - 1).collect();
            s.push('…');
        }
        s.trim_end().to_owned()
    }
}

/// Other names models use for node types.
fn alias(ty: &str) -> &str {
    match ty {
        "vstack" | "column" | "col" | "list" => "stack",
        "hstack" => "row",
        "text" | "md" => "markdown",
        "keyvalue" | "key_value" | "kvs" => "kv",
        "chip" => "chips",
        "alert" | "note" => "callout",
        "diagram" => "mermaid",
        "img" => "image",
        other => other,
    }
}

fn valid_field_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn check_action(trusted: bool, a: &Action) -> Result<(), String> {
    match a {
        Action::Answer { .. } if !trusted => Err("only acc-ui answers requests".to_owned()),
        Action::OpenUrl { url } if !is_web_url(url) => Err(format!("{url:?} is not an http(s) URL")),
        Action::Prompt { text } | Action::Copy { text } if text.trim().is_empty() => {
            Err("empty text".to_owned())
        }
        Action::OpenFile { path, .. } if path.trim().is_empty() => Err("empty path".to_owned()),
        _ => Ok(()),
    }
}

fn is_web_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && url.len() > 8
}

fn check_image_src(src: &str) -> Result<(), String> {
    let lower = src.trim().to_ascii_lowercase();
    let ok = lower.starts_with("https://")
        || lower.starts_with("data:image/")
        || src.starts_with('/')
        || src.starts_with("~/")
        || src
            .strip_prefix("sha256:")
            .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "image src {:?} must be https:, data:image/, an absolute host path or sha256:<hex>",
            src.chars().take(80).collect::<String>()
        ))
    }
}

/// Rewrites the shorthands models write into the canonical form.
fn desugar(ty: &str, obj: &mut Map<String, Value>) {
    match ty {
        "card" | "button" => {
            if let Some(a) = obj.get_mut("action") {
                desugar_action(a);
            }
        }
        "chips" => {
            if let Some(Value::Array(options)) = obj.get_mut("options") {
                for o in options.iter_mut() {
                    if let Value::String(s) = o {
                        *o = json!({ "label": s, "action": { "type": "prompt", "text": s } });
                    } else if let Value::Object(m) = o {
                        let label = m.get("label").cloned().unwrap_or(Value::Null);
                        let action = m
                            .entry("action")
                            .or_insert_with(|| json!({ "type": "prompt", "text": label }));
                        desugar_action(action);
                    }
                }
            }
        }
        "kv" => {
            if let Some(Value::Object(map)) = obj.get("items") {
                let items: Vec<Value> = map
                    .iter()
                    .map(|(k, v)| json!({ "label": k, "value": v }))
                    .collect();
                obj.insert("items".to_owned(), Value::Array(items));
            }
            if let Some(Value::Array(items)) = obj.get_mut("items") {
                for item in items.iter_mut() {
                    if let Value::Object(m) = item {
                        rename(m, "key", "label");
                        stringify(m, "value");
                        stringify(m, "label");
                    }
                }
            }
        }
        "table" => desugar_table(obj),
        "chart" => desugar_chart(obj),
        "checklist" => {
            if let Some(Value::Array(items)) = obj.get_mut("items") {
                for item in items.iter_mut() {
                    if let Value::String(s) = item {
                        *item = json!({ "text": s, "state": "todo" });
                    } else if let Value::Object(m) = item {
                        rename(m, "label", "text");
                        rename(m, "status", "state");
                        let done = m.remove("done").or_else(|| m.remove("checked"));
                        if !m.contains_key("state") {
                            let state = if done.and_then(|d| d.as_bool()) == Some(true) {
                                "done"
                            } else {
                                "todo"
                            };
                            m.insert("state".to_owned(), state.into());
                        }
                    }
                }
            }
        }
        "progress" => {
            let value = obj.get("value").and_then(Value::as_f64);
            if !obj.contains_key("max")
                && let Some(v) = value
                && v > 1.0
            {
                obj.insert("max".to_owned(), 100.into());
            }
        }
        "mermaid" => {
            rename(obj, "source", "code");
            rename(obj, "text", "code");
        }
        "markdown" | "callout" => {
            rename(obj, "content", "text");
            rename(obj, "markdown", "text");
        }
        "badge" => rename(obj, "label", "text"),
        "image" => rename(obj, "url", "src"),
        "html" => rename(obj, "content", "html"),
        "form" => {
            if let Some(Value::Array(fields)) = obj.get_mut("fields") {
                for f in fields.iter_mut() {
                    if let Value::Object(m) = f {
                        rename(m, "type", "kind");
                        rename(m, "id", "name");
                        if let Some(Value::Array(options)) = m.get_mut("options") {
                            for o in options.iter_mut() {
                                desugar_choice(o);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn desugar_action(a: &mut Value) {
    if let Value::String(s) = a {
        *a = json!({ "type": "prompt", "text": s });
    }
}

fn desugar_choice(o: &mut Value) {
    match o {
        Value::String(s) => *o = json!({ "label": s, "value": s }),
        Value::Number(_) | Value::Bool(_) => {
            let s = o.to_string();
            *o = json!({ "label": s, "value": s });
        }
        Value::Object(m) => {
            stringify(m, "value");
            stringify(m, "label");
            if !m.contains_key("value")
                && let Some(l) = m.get("label").cloned()
            {
                m.insert("value".to_owned(), l);
            }
            if !m.contains_key("label")
                && let Some(v) = m.get("value").cloned()
            {
                m.insert("label".to_owned(), v);
            }
        }
        _ => {}
    }
}

fn desugar_table(obj: &mut Map<String, Value>) {
    rename(obj, "headers", "columns");
    let mut keys: Vec<String> = Vec::new();
    if let Some(Value::Array(cols)) = obj.get_mut("columns") {
        for c in cols.iter_mut() {
            match c {
                Value::String(s) => {
                    keys.push(s.clone());
                    *c = json!({ "label": s });
                }
                Value::Object(m) => {
                    let key = m.get("key").and_then(Value::as_str).map(str::to_owned);
                    rename(m, "title", "label");
                    if !m.contains_key("label")
                        && let Some(k) = &key
                    {
                        m.insert("label".to_owned(), k.clone().into());
                    }
                    let label = m.get("label").and_then(Value::as_str).unwrap_or_default();
                    keys.push(key.unwrap_or_else(|| label.to_owned()));
                    m.remove("key");
                }
                _ => {}
            }
        }
    }
    if let Some(Value::Array(rows)) = obj.get_mut("rows") {
        for row in rows.iter_mut() {
            if let Value::Object(m) = row {
                *row = Value::Array(
                    keys.iter()
                        .map(|k| m.get(k).cloned().unwrap_or(Value::Null))
                        .collect(),
                );
            }
            if let Value::Array(cells) = row {
                for cell in cells.iter_mut() {
                    *cell = Value::String(scalar_text(cell));
                }
            }
        }
    }
}

fn desugar_chart(obj: &mut Map<String, Value>) {
    rename(obj, "chart", "kind");
    if !obj.contains_key("series")
        && let Some(points) = obj.remove("points").or_else(|| obj.remove("data"))
    {
        obj.insert("series".to_owned(), json!([{ "points": points }]));
    }
    let labels: Option<Vec<Value>> = obj.remove("labels").and_then(|l| match l {
        Value::Array(a) => Some(a),
        _ => None,
    });
    if let Some(Value::Array(series)) = obj.get_mut("series") {
        for s in series.iter_mut() {
            let Value::Object(m) = s else { continue };
            rename(m, "data", "points");
            rename(m, "values", "points");
            rename(m, "label", "name");
            let Some(Value::Array(points)) = m.get_mut("points") else {
                continue;
            };
            for (i, p) in points.iter_mut().enumerate() {
                let x_at = |i: usize| {
                    labels
                        .as_ref()
                        .and_then(|l| l.get(i).cloned())
                        .unwrap_or_else(|| Value::from(i + 1))
                };
                match p {
                    Value::Array(pair) if pair.len() == 2 => {
                        *p = json!({ "x": pair[0], "y": number(&pair[1]) });
                    }
                    Value::Number(_) | Value::String(_) => {
                        *p = json!({ "x": x_at(i), "y": number(p) });
                    }
                    Value::Object(pm) => {
                        rename(pm, "label", "x");
                        rename(pm, "value", "y");
                        if let Some(y) = pm.get("y") {
                            let y = number(y);
                            pm.insert("y".to_owned(), y);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn number(v: &Value) -> Value {
    match v {
        Value::String(s) => s.trim().parse::<f64>().map(Value::from).unwrap_or(Value::Null),
        other => other.clone(),
    }
}

fn scalar_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn stringify(m: &mut Map<String, Value>, key: &str) {
    if let Some(v) = m.get_mut(key)
        && !v.is_string()
    {
        *v = Value::String(scalar_text(v));
    }
}

fn rename(m: &mut Map<String, Value>, from: &str, to: &str) {
    if !m.contains_key(to)
        && let Some(v) = m.remove(from)
    {
        m.insert(to.to_owned(), v);
    }
}
