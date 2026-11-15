//! An agent's markdown as blocks: the parts that have a component become
//! that component, the rest stays markdown.
//!
//! | Markdown | Block |
//! |---|---|
//! | a GFM table | `table` (cells keep their inline markdown) |
//! | a list where every item is a task (`- [ ]`, `- [x]`) | `checklist` |
//! | `> [!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, `[!CAUTION]` | `callout` |
//! | a closed component fence (` ```mermaid `, ` ```chart `, ...) | that component |
//! | any other fence, or a component fence still streaming | `code` |
//! | everything else | `markdown`, consecutive blocks merged, as written |
//!
//! Inline markdown (emphasis, code spans, links) is left to the app. Parsing
//! is deterministic and cheap, so while a message streams, parse the whole
//! text again on each delta; blocks only grow or change at the end.

use std::ops::Range;

use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde_json::{Value, json};

use crate::fence::{fence_node, is_component_fence};
use crate::normalize::{Normalized, normalize_value};

/// `text` as a normalised spec: a `stack` of blocks.
pub fn markdown_blocks(text: &str) -> Normalized {
    normalize_value(json!({ "root": { "type": "stack", "children": blocks(text) } }))
}

/// The block nodes of `text`, before normalising.
pub(crate) fn blocks(text: &str) -> Vec<Value> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_GFM;
    let mut out = Vec::new();
    let mut prose: Option<Range<usize>> = None;
    let mut depth = 0usize;
    let mut block: Vec<(Event<'_>, Range<usize>)> = Vec::new();
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        match &event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        block.push((event, range));
        if depth > 0 {
            continue;
        }
        let events = std::mem::take(&mut block);
        let Some((first, whole)) = events.first().cloned() else {
            continue;
        };
        match component(text, &first, whole.clone(), &events) {
            Some(node) => {
                if let Some(p) = prose.take() {
                    push_prose(&mut out, text, p);
                }
                out.push(node);
            }
            None => {
                prose = Some(match prose {
                    Some(p) => p.start..whole.end,
                    None => whole,
                });
            }
        }
    }
    if let Some(p) = prose {
        push_prose(&mut out, text, p);
    }
    out
}

fn push_prose(out: &mut Vec<Value>, text: &str, range: Range<usize>) {
    let md = text[range].trim();
    if !md.is_empty() {
        out.push(json!({ "type": "markdown", "text": md }));
    }
}

/// The component for one top-level block, or `None` for prose.
fn component(
    text: &str,
    first: &Event<'_>,
    whole: Range<usize>,
    events: &[(Event<'_>, Range<usize>)],
) -> Option<Value> {
    let Event::Start(tag) = first else {
        return None;
    };
    match tag {
        Tag::Table(aligns) => Some(table(text, aligns, events)),
        Tag::List(_) => checklist(text, events),
        Tag::BlockQuote(Some(kind)) => Some(callout(text, *kind, events)),
        Tag::CodeBlock(kind) => Some(code(text, kind, whole, events)),
        _ => None,
    }
}

fn table(text: &str, aligns: &[Alignment], events: &[(Event<'_>, Range<usize>)]) -> Value {
    let mut head: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut in_head = false;
    let mut cell: Option<Range<usize>> = None;
    let mut cell_open = false;
    for (event, range) in events {
        match event {
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::Start(Tag::TableRow) => rows.push(Vec::new()),
            Event::Start(Tag::TableCell) => {
                cell_open = true;
                cell = None;
            }
            Event::End(TagEnd::TableCell) => {
                cell_open = false;
                let s = cell.take().map_or("", |r| text[r].trim()).to_owned();
                match (in_head, rows.last_mut()) {
                    (true, _) | (false, None) => head.push(s),
                    (false, Some(row)) => row.push(s),
                }
            }
            _ if cell_open => {
                cell = Some(match cell.take() {
                    Some(c) => c.start..range.end.max(c.end),
                    None => range.clone(),
                });
            }
            _ => {}
        }
    }
    let columns: Vec<Value> = head
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let mut c = json!({ "label": label });
            let align = match aligns.get(i) {
                Some(Alignment::Left) => Some("start"),
                Some(Alignment::Center) => Some("center"),
                Some(Alignment::Right) => Some("end"),
                _ => None,
            };
            if let Some(a) = align {
                c["align"] = a.into();
            }
            c
        })
        .collect();
    json!({ "type": "table", "columns": columns, "rows": rows })
}

/// A list where every item is a task, as a checklist.
fn checklist(text: &str, events: &[(Event<'_>, Range<usize>)]) -> Option<Value> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut current: Option<(bool, usize, usize)> = None; // (checked, text start, item end)
    let mut item_start = false;
    for (event, range) in events {
        match event {
            Event::Start(Tag::Item) => {
                depth += 1;
                if depth == 1 {
                    item_start = true;
                    current = None;
                }
            }
            Event::TaskListMarker(checked) if depth == 1 && item_start => {
                current = Some((*checked, range.end, range.end));
                item_start = false;
            }
            Event::End(TagEnd::Item) => {
                if depth == 1 {
                    let (checked, start, _) = current.take()?;
                    let body = text[start..range.end].trim();
                    if body.is_empty() {
                        return None;
                    }
                    items.push(json!({ "text": body, "state": if checked { "done" } else { "todo" } }));
                }
                depth = depth.saturating_sub(1);
            }
            Event::Start(Tag::Paragraph) => {}
            // Content before any task marker: an ordinary item.
            _ if depth == 1 && item_start && !matches!(event, Event::Start(_) | Event::End(_)) => {
                return None;
            }
            _ => {}
        }
    }
    (!items.is_empty()).then(|| json!({ "type": "checklist", "items": items }))
}

fn callout(text: &str, kind: BlockQuoteKind, events: &[(Event<'_>, Range<usize>)]) -> Value {
    let (title, tone) = match kind {
        BlockQuoteKind::Note => ("Note", "info"),
        BlockQuoteKind::Tip => ("Tip", "success"),
        BlockQuoteKind::Important => ("Important", "accent"),
        BlockQuoteKind::Warning => ("Warning", "warning"),
        BlockQuoteKind::Caution => ("Caution", "danger"),
    };
    // The body: from the first child block to the last, without `>`s.
    let inner = &events[1..events.len().saturating_sub(1)];
    let body = match (inner.first(), inner.last()) {
        (Some((_, a)), Some((_, b))) => text[a.start..b.end]
            .lines()
            .map(|l| {
                let l = l.trim_start();
                let l = l.strip_prefix('>').unwrap_or(l);
                l.strip_prefix(' ').unwrap_or(l)
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned(),
        _ => String::new(),
    };
    json!({ "type": "callout", "title": title, "tone": tone, "text": body })
}

fn code(
    text: &str,
    kind: &CodeBlockKind<'_>,
    whole: Range<usize>,
    events: &[(Event<'_>, Range<usize>)],
) -> Value {
    let body: String = events
        .iter()
        .filter_map(|(e, _)| match e {
            Event::Text(t) => Some(t.as_ref()),
            _ => None,
        })
        .collect();
    let body = body.strip_suffix('\n').unwrap_or(&body).to_owned();
    let info = match kind {
        CodeBlockKind::Fenced(info) => info.trim(),
        CodeBlockKind::Indented => "",
    };
    if is_component_fence(info)
        && fence_closed(&text[whole])
        && let Some(Ok(node)) = fence_node(info, &body)
    {
        return node;
    }
    let mut node = json!({ "type": "code", "text": body });
    if let Some(lang) = info.split_whitespace().next() {
        node["language"] = lang.into();
    }
    node
}

/// Whether a fenced block's source ends with its closing fence (it has
/// finished streaming).
fn fence_closed(source: &str) -> bool {
    let source = source.trim_end();
    let mut lines = source.lines();
    let Some(open) = lines.next().map(str::trim_start) else {
        return false;
    };
    let ch = match open.chars().next() {
        Some(c @ ('`' | '~')) => c,
        _ => return false,
    };
    let n = open.chars().take_while(|&c| c == ch).count();
    match lines.last().map(str::trim) {
        Some(close) => close.len() >= n && close.chars().all(|c| c == ch),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{blocks, fence_closed};
    use serde_json::json;

    #[test]
    fn closing_fences() {
        assert!(fence_closed("```mermaid\ngraph TD\n```"));
        assert!(fence_closed("````\na\n`````\n"));
        assert!(!fence_closed("```mermaid\ngraph TD"));
        assert!(!fence_closed("````\na\n```"));
    }

    #[test]
    fn prose_runs_merge() {
        let b = blocks("# Title\n\nSome *text*.\n\n- a\n- b\n\n```rust\nfn x() {}\n```\n\nAfter.");
        assert_eq!(
            b,
            [
                json!({"type":"markdown","text":"# Title\n\nSome *text*.\n\n- a\n- b"}),
                json!({"type":"code","text":"fn x() {}","language":"rust"}),
                json!({"type":"markdown","text":"After."}),
            ]
        );
    }
}
