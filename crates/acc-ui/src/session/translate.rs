//! Items and asks as `acc-ui/1` specs, for apps that show them as blocks
//! (a transcript view, an expanded step, an approval card).
//!
//! Item specs hold agent text, so they're normalised like any agent spec:
//! no requests or answers can come from them. Only [`SessionView::ask_spec`],
//! built from ACC's own request, makes a `request` node.

use serde_json::{Value, json};

use super::event::{
    AskKind, Detail, FileChangeKind, ImageRef, ItemKind, ItemStatus, LineTag, OptionKind, ToolKind,
};
use super::{Ask, SessionView, display_path, first_line, unwrap_shell};
use crate::blocks::blocks;
use crate::normalize::{Normalized, normalize_trusted, normalize_value, valid_id};

/// Most of a tool's output shown in an item spec, in characters.
const CODE_CHARS: usize = 8000;
/// Most files listed from a search's output.
const LISTED_FILES: usize = 100;

impl SessionView {
    /// Item `id` as a spec: messages as blocks, reasoning, commands, file
    /// changes, tool calls and subagent runs as a `disclosure` whose summary
    /// says what happened and whose children show it. `None` for items
    /// with nothing to show: reasoning without text, unknown details.
    pub fn item_spec(&self, id: &str) -> Option<Normalized> {
        let item = self.item(id)?;
        let root = self.header.root.as_deref();
        let node = match (&item.detail, item.kind) {
            (Detail::Text, ItemKind::UserMessage) if item.images.is_empty() => {
                json!({ "type": "markdown", "text": item.text })
            }
            (Detail::Text, ItemKind::UserMessage) => {
                let mut children = image_nodes(&item.images);
                if !item.text.is_empty() {
                    children.insert(0, json!({ "type": "markdown", "text": item.text }));
                }
                json!({ "type": "stack", "children": children })
            }
            // Codex sends reasoning without text when summaries are off.
            (Detail::Text, ItemKind::Reasoning) if item.text.trim().is_empty() => return None,
            (Detail::Text, ItemKind::Reasoning) => json!({
                "type": "disclosure",
                "summary": if item.status == ItemStatus::Running { "Thinking" } else { "Thought" },
                "children": blocks(&item.text),
            }),
            (Detail::Text, _) => json!({ "type": "stack", "children": blocks(&item.text) }),
            (
                Detail::Command {
                    command, exit_code, ..
                },
                _,
            ) => {
                let shown = unwrap_shell(command);
                let mut children = Vec::new();
                if shown.lines().count() > 1 {
                    children.push(json!({ "type": "code", "text": shown, "language": "sh" }));
                }
                let output = self.commands().iter().find(|c| c.item_id == item.id);
                if let Some(c) = output {
                    let mut out = json!({ "type": "output", "text": c.output_tail });
                    if let Some(code) = exit_code {
                        out["exit_code"] = (*code).into();
                    }
                    let omitted = c.output_lines.saturating_sub(c.output_tail.lines().count());
                    if omitted > 0 {
                        out["omitted_lines"] = omitted.into();
                    }
                    if !c.output_tail.is_empty() || exit_code.is_some() {
                        children.push(out);
                    }
                }
                disclosure(format!("$ {}", first_line(&shown).unwrap_or_default()), children)
            }
            (Detail::FileChange { files, .. }, _) => {
                let summary = files
                    .iter()
                    .map(|f| format!("{} +{} −{}", display_path(root, &f.path), f.added, f.removed))
                    .collect::<Vec<_>>()
                    .join(", ");
                let verb = match files.as_slice() {
                    [f] if f.kind == FileChangeKind::Add => "Created",
                    [f] if f.kind == FileChangeKind::Delete => "Deleted",
                    _ => "Edited",
                };
                let diff = json!({ "type": "diff", "files": files.iter().map(|f| diff_file(root, f)).collect::<Vec<_>>() });
                disclosure(format!("{verb} {summary}"), vec![diff])
            }
            (
                Detail::Subagent {
                    agent,
                    description,
                    output,
                    ..
                },
                _,
            ) => {
                let who = agent.as_deref().map(|a| format!("{a}: ")).unwrap_or_default();
                let children = output.as_deref().map(blocks).unwrap_or_default();
                disclosure(format!("{who}{description}"), children)
            }
            (
                Detail::Tool {
                    tool,
                    kind,
                    input,
                    output,
                    paths,
                    ..
                },
                _,
            ) => {
                let activity = self.activity(item);
                let summary = match &activity {
                    Some(a) if !a.target.is_empty() => format!("{} {}", a.kind.past_label(), a.target),
                    Some(a) => a.kind.past_label().to_owned(),
                    None => "Tool call".to_owned(),
                };
                let output = output.as_deref().map(str::trim_end).filter(|o| !o.is_empty());
                let read_path = (*kind == ToolKind::Read)
                    .then(|| {
                        input
                            .get("file_path")
                            .and_then(Value::as_str)
                            .or(paths.first().map(String::as_str))
                    })
                    .flatten()
                    .filter(|p| !p.is_empty());
                let children = match output {
                    _ if !item.images.is_empty() => image_nodes(&item.images),
                    _ if read_path.is_some() => {
                        read_nodes(root, read_path.unwrap_or_default(), input, output)
                    }
                    Some(o) if lists_files(tool, input) => file_list_nodes(root, o),
                    Some(o) => vec![json!({ "type": "code", "text": tail_chars(o, CODE_CHARS) })],
                    None => Vec::new(),
                };
                disclosure(summary, children)
            }
            (Detail::Unknown, _) => return None,
        };
        let mut spec = json!({ "root": node });
        if valid_id(&item.id) {
            spec["id"] = item.id.clone().into();
        }
        Some(normalize_value(spec))
    }

    /// A pending ask as a spec: one `request` node whose options answer it.
    pub fn ask_spec(&self, ask: &Ask) -> Normalized {
        let root = self.header.root.as_deref();
        let kind = match ask.kind {
            AskKind::Question => "question",
            AskKind::Auth => "auth",
            AskKind::Permission | AskKind::Unknown => "permission",
        };
        let options: Vec<Value> = ask
            .options
            .iter()
            .map(|o| {
                let mut opt = json!({
                    "label": if o.label.is_empty() { &o.id } else { &o.label },
                    "action": { "type": "answer", "request_id": ask.request_id, "option_id": o.id },
                });
                if let Some(role) = o.kind.and_then(OptionKind::as_str) {
                    opt["role"] = role.into();
                }
                opt
            })
            .collect();
        let mut request = json!({
            "type": "request",
            "request_id": ask.request_id,
            "kind": kind,
            "title": ask.title,
            "options": options,
        });
        let detail = match &ask.subject {
            Some(Detail::Command { command, .. }) => Some(unwrap_shell(command)),
            Some(Detail::FileChange { files, .. }) => Some(
                files
                    .iter()
                    .map(|f| format!("{} +{} −{}", display_path(root, &f.path), f.added, f.removed))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Some(Detail::Tool { tool, title, .. }) if !title.is_empty() => {
                Some(format!("{tool} {}", display_path(root, title)))
            }
            _ => None,
        };
        if let Some(d) = detail.filter(|d| !d.is_empty()) {
            request["detail"] = d.into();
        }
        if let Some(plan) = &ask.plan {
            request["plan"] = plan.as_str().into();
        }
        if !ask.questions.is_empty() {
            request["questions"] = ask
                .questions
                .iter()
                .map(|q| {
                    json!({
                        "text": q.text,
                        "header": q.header,
                        "options": q.options.iter()
                            .map(|o| json!({ "label": o.label, "description": o.description }))
                            .collect::<Vec<_>>(),
                        "multi_select": q.multi_select,
                        "allows_text": q.allows_text,
                        "secret": q.secret,
                        "required": q.required,
                    })
                })
                .collect::<Vec<_>>()
                .into();
        }
        normalize_trusted(json!({ "root": request }))
    }
}

/// An `image` node per ref that has a `src`.
fn image_nodes(images: &[ImageRef]) -> Vec<Value> {
    images
        .iter()
        .filter_map(|i| {
            let src = i.src()?;
            let alt = i
                .path
                .as_deref()
                .and_then(|p| p.rsplit('/').next())
                .unwrap_or("Image");
            Some(json!({ "type": "image", "src": src, "alt": alt }))
        })
        .collect()
}

fn disclosure(summary: String, children: Vec<Value>) -> Value {
    json!({ "type": "disclosure", "summary": summary, "children": children })
}

fn diff_file(root: Option<&str>, f: &super::event::FileDiff) -> Value {
    let change = match f.kind {
        FileChangeKind::Add => "add",
        FileChangeKind::Delete => "delete",
        FileChangeKind::Rename => "rename",
        FileChangeKind::Update => "update",
    };
    let hunks: Vec<Value> = f
        .hunks
        .iter()
        .map(|h| {
            json!({
                "old_start": h.old_start,
                "new_start": h.new_start,
                "lines": h.lines.iter().map(|l| json!({
                    "tag": match l.tag { LineTag::Add => "add", LineTag::Remove => "remove", LineTag::Context => "context" },
                    "text": l.text,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut file = json!({
        "path": display_path(root, &f.path),
        "change": change,
        "added": f.added,
        "removed": f.removed,
        "hunks": hunks,
    });
    if let Some(old) = &f.old_path {
        file["old_path"] = display_path(root, old).into();
    }
    file
}

/// A file read: the file (at its first line when the read started
/// further in), then what was read, as code in the file's language.
fn read_nodes(root: Option<&str>, path: &str, input: &Value, output: Option<&str>) -> Vec<Value> {
    let mut file = json!({ "type": "file", "path": display_path(root, path) });
    if let Some(line) = input
        .get("offset")
        .and_then(Value::as_u64)
        .filter(|l| *l > 0)
        .and_then(|l| u32::try_from(l).ok())
    {
        file["line"] = line.into();
    }
    let mut nodes = vec![file];
    if let Some(o) = output {
        let text = strip_line_numbers(o).unwrap_or_else(|| o.to_owned());
        let mut code = json!({ "type": "code", "text": head_chars(&text, CODE_CHARS) });
        if let Some(lang) = language(path) {
            code["language"] = lang.into();
        }
        nodes.push(code);
    }
    nodes
}

/// Claude's Read output without its `cat -n` style numbers (`     1\t…`
/// or `     1→…`), when every line up to an optional trailing
/// `<system-reminder>` has one. `None` when it isn't numbered.
fn strip_line_numbers(out: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut rest = "";
    let mut at = 0;
    for l in out.split('\n') {
        let t = l.trim_start();
        let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let after = &t[digits..];
        let body = (digits > 0)
            .then(|| after.strip_prefix('\t').or_else(|| after.strip_prefix('→')))
            .flatten();
        let Some(b) = body else {
            rest = &out[at..];
            break;
        };
        lines.push(b);
        at += l.len() + 1;
    }
    let rest = rest.trim();
    (!lines.is_empty() && (rest.is_empty() || rest.starts_with("<system-reminder>")))
        .then(|| lines.join("\n"))
}

/// Whether a search tool's output is a list of paths: Claude's Glob, and
/// Grep in its default `files_with_matches` mode.
fn lists_files(tool: &str, input: &Value) -> bool {
    match tool {
        "Glob" => true,
        "Grep" => matches!(
            input.get("output_mode").and_then(Value::as_str),
            None | Some("files_with_matches")
        ),
        _ => false,
    }
}

/// A `file` node per path in a search's output, the first
/// [`LISTED_FILES`], then a note of how many more there were.
fn file_list_nodes(root: Option<&str>, out: &str) -> Vec<Value> {
    let paths: Vec<&str> = out
        .lines()
        .map(str::trim)
        // `Found 3 files`, `No files found`, a truncation note.
        .filter(|l| l.starts_with('/') || (!l.is_empty() && !l.contains(' ')))
        .collect();
    if paths.is_empty() {
        return vec![json!({ "type": "code", "text": tail_chars(out, CODE_CHARS) })];
    }
    let mut nodes: Vec<Value> = paths
        .iter()
        .take(LISTED_FILES)
        .map(|p| json!({ "type": "file", "path": display_path(root, p) }))
        .collect();
    if paths.len() > LISTED_FILES {
        nodes.push(json!({ "type": "markdown", "text": format!("and {} more", paths.len() - LISTED_FILES) }));
    }
    nodes
}

/// A code fence language for a file, from its extension.
fn language(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next()?;
    if name == "Makefile" {
        return Some("makefile");
    }
    if name == "Dockerfile" {
        return Some("dockerfile");
    }
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "go" => "go",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "java" => "java",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "m" => "objective-c",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "lua" => "lua",
        "sh" | "bash" | "zsh" => "sh",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "sql" => "sql",
        "xml" => "xml",
        _ => return None,
    })
}

/// The first `max` characters of `s`.
fn head_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// The last `max` characters of `s`.
fn tail_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    s.chars().skip(n.saturating_sub(max)).collect()
}
