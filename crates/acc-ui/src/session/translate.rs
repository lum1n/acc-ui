//! Items and asks as `acc-ui/1` specs, for apps that show them as blocks
//! (a transcript view, an expanded step, an approval card).
//!
//! Item specs hold agent text, so they're normalised like any agent spec:
//! no requests or answers can come from them. Only [`SessionView::ask_spec`],
//! built from ACC's own request, makes a `request` node.

use serde_json::{Value, json};

use super::event::{AskKind, Detail, FileChangeKind, ItemKind, ItemStatus, LineTag};
use super::{Ask, SessionView, display_path, first_line, unwrap_shell};
use crate::blocks::blocks;
use crate::normalize::{Normalized, normalize_trusted, normalize_value, valid_id};

impl SessionView {
    /// Item `id` as a spec: messages as blocks, reasoning, commands, file
    /// changes, tool calls and subagent runs as a `disclosure` whose summary
    /// says what happened and whose children show it.
    pub fn item_spec(&self, id: &str) -> Option<Normalized> {
        let item = self.item(id)?;
        let root = self.header.root.as_deref();
        let node = match (&item.detail, item.kind) {
            (Detail::Text, ItemKind::UserMessage) => json!({ "type": "markdown", "text": item.text }),
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
            (Detail::Tool { output, .. }, _) => {
                let activity = self.activity(item);
                let summary = match &activity {
                    Some(a) if !a.target.is_empty() => format!("{} {}", a.kind.past_label(), a.target),
                    Some(a) => a.kind.past_label().to_owned(),
                    None => "Tool call".to_owned(),
                };
                let children = match output.as_deref().map(str::trim_end) {
                    Some(o) if !o.is_empty() => vec![json!({ "type": "code", "text": tail_chars(o, 8000) })],
                    _ => Vec::new(),
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
                if let Some(role @ ("allow_once" | "allow_always" | "reject_once" | "reject_always")) =
                    o.kind.as_deref()
                {
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
        normalize_trusted(json!({ "root": request }))
    }
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

/// The last `max` characters of `s`.
fn tail_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    s.chars().skip(n.saturating_sub(max)).collect()
}
