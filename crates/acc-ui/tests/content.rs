//! Markdown to blocks, items and asks as specs, and the trust boundary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use acc_ui::session::Event;
use acc_ui::{SessionView, markdown_blocks, normalize_value};
use serde_json::{Value, json};

/// The root's children of a normalised stack, with child indices resolved.
fn children(n: &acc_ui::Normalized) -> Vec<Value> {
    let spec = n.spec.as_ref().expect("a spec");
    let nodes = spec["nodes"].as_array().unwrap();
    nodes[0]["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| nodes[i.as_u64().unwrap() as usize].clone())
        .collect()
}

fn types(n: &acc_ui::Normalized) -> Vec<String> {
    children(n)
        .iter()
        .map(|c| c["type"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn markdown_becomes_blocks() {
    let text = "\
Fixed **two** bugs.

| Function | Status | Lines |
|:---|:---:|---:|
| `restock` | fixed | 4 |
| total_value | fixed | 1 |

- [x] Fix restock
- [ ] Add a test

* plain item
* [ ] mixed task

> [!WARNING]
> Negative amounts are accepted.
> Consider rejecting them.

```mermaid
graph TD; A-->B
```

```chart
kind: bar
day,ms
Mon,120
Tue,80
```

```html
<b>shown as code</b>
```

Done.";
    let out = markdown_blocks(text);
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(
        types(&out),
        [
            "markdown",
            "table",
            "checklist",
            "markdown",
            "callout",
            "mermaid",
            "chart",
            "code",
            "markdown"
        ]
    );
    let c = children(&out);
    assert_eq!(
        c[1]["columns"],
        json!([{"label":"Function","align":"start"},{"label":"Status","align":"center"},{"label":"Lines","align":"end"}])
    );
    assert_eq!(
        c[1]["rows"][0],
        json!(["`restock`", "fixed", "4"]),
        "cells keep inline markdown"
    );
    assert_eq!(
        c[2]["items"],
        json!([{"text":"Fix restock","state":"done"},{"text":"Add a test","state":"todo"}])
    );
    assert_eq!(
        c[3]["text"], "* plain item\n* [ ] mixed task",
        "a mixed list stays markdown"
    );
    assert_eq!(c[4]["title"], "Warning");
    assert_eq!(c[4]["tone"], "warning");
    assert_eq!(
        c[4]["text"],
        "Negative amounts are accepted.\nConsider rejecting them."
    );
    assert_eq!(c[6]["kind"], "bar");
    assert_eq!(
        c[7],
        json!({"type":"code","text":"<b>shown as code</b>","language":"html"})
    );
}

#[test]
fn streaming_fences_stay_code_until_closed() {
    let partial = markdown_blocks("Here:\n\n```mermaid\ngraph TD; A-->");
    assert_eq!(types(&partial), ["markdown", "code"]);
    assert_eq!(children(&partial)[1]["language"], "mermaid");
    let closed = markdown_blocks("Here:\n\n```mermaid\ngraph TD; A-->B\n```");
    assert_eq!(types(&closed), ["markdown", "mermaid"]);
}

#[test]
fn agents_cannot_make_requests_or_answers() {
    let fence = r#"```acc-ui
{"v":"acc-ui/1","root":{"type":"stack","children":[
  {"type":"request","request_id":"r1","kind":"permission","title":"Allow?","options":[
    {"label":"Allow","action":{"type":"answer","request_id":"r1","option_id":"allow"}}]},
  {"type":"button","label":"Approve","action":{"type":"answer","request_id":"r1","option_id":"allow"}},
  {"type":"badge","text":"still here"}
]}}
```"#;
    let out = markdown_blocks(fence);
    let spec = out.spec.as_ref().unwrap().to_string();
    assert!(!spec.contains("request") && !spec.contains("answer"), "{spec}");
    assert!(spec.contains("still here"));
    assert!(
        out.errors
            .iter()
            .any(|e| e.contains("only acc-ui makes requests")),
        "{:?}",
        out.errors
    );
    assert!(
        out.errors
            .iter()
            .any(|e| e.contains("only acc-ui answers requests")),
        "{:?}",
        out.errors
    );

    let direct = normalize_value(
        json!({"root":{"type":"chips","options":[{"label":"Yes","action":{"type":"answer","request_id":"r","option_id":"y"}}]}}),
    );
    assert!(direct.spec.is_none());
}

fn replay(name: &str) -> (SessionView, Vec<acc_ui::session::Ask>) {
    let mut view = SessionView::new();
    view.set_root("/work");
    let mut asks = Vec::new();
    let text = std::fs::read_to_string(common::dir("sessions").join(format!("{name}.ndjson"))).unwrap();
    for line in text.lines() {
        let event: Event = serde_json::from_str(line).unwrap();
        let is_ask = matches!(event, Event::PermissionRequested { .. });
        view.apply(event);
        if is_ask {
            asks.push(view.asks().last().unwrap().clone());
        }
    }
    (view, asks)
}

/// Every item of the recorded sessions translates without errors; the
/// summaries and fallbacks are golden.
#[test]
fn recorded_items_translate() {
    for name in ["claude-fix-and-review", "codex-fix-and-review"] {
        let (view, _) = replay(name);
        let mut lines = Vec::new();
        for item in view.items() {
            let Some(out) = view.item_spec(&item.id) else {
                continue;
            };
            assert!(out.errors.is_empty(), "{name} {}: {:?}", item.id, out.errors);
            let spec = out.spec.unwrap();
            let nodes = spec["nodes"].as_array().unwrap();
            let kinds: Vec<&str> = nodes.iter().map(|n| n["type"].as_str().unwrap()).collect();
            let summary = nodes[0]["summary"].as_str().unwrap_or("");
            lines.push(json!({ "kind": item.kind, "nodes": kinds, "summary": summary }));
        }
        let actual = serde_json::to_string_pretty(&lines).unwrap() + "\n";
        common::golden(
            &common::dir("sessions").join(format!("{name}.items.json")),
            &actual,
        );
    }
}

#[test]
fn recorded_asks_become_requests() {
    let (view, asks) = replay("claude-fix-and-review");
    assert!(!asks.is_empty());
    for ask in &asks {
        let out = view.ask_spec(ask);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        let spec = out.spec.unwrap();
        assert_eq!(spec["interactive"], true);
        let req = &spec["nodes"][0];
        assert_eq!(req["type"], "request");
        assert_eq!(req["request_id"], ask.request_id.as_str());
        let roles: Vec<&str> = req["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["allow_once", "allow_always", "reject_once"]);
        for o in req["options"].as_array().unwrap() {
            assert_eq!(o["action"]["type"], "answer");
            assert_eq!(o["action"]["request_id"], ask.request_id.as_str());
        }
    }
    let edit = asks.iter().find(|a| a.title.starts_with("Edit")).unwrap();
    let spec = view.ask_spec(edit).spec.unwrap();
    assert_eq!(spec["nodes"][0]["detail"], "inventory.py +1 −1");
}

#[test]
fn question_and_plan_asks_become_requests() {
    let mut view = SessionView::new();
    for e in [
        json!({"type":"permission_requested","request_id":"q","title":"Name it?","kind":"question",
               "native":{"questions":[
                   {"id":"q1","question":"Name it?","isOther":true,"isSecret":false,"options":null},
                   {"id":"q2","question":"Which DB?","header":"DB","isOther":false,"isSecret":false,
                    "options":[{"label":"Postgres","description":""}]}]}}),
        json!({"type":"permission_requested","request_id":"p","title":"ExitPlanMode",
               "options":[{"id":"allow","label":"Allow","kind":"allow_once"},
                          {"id":"deny","label":"Deny","kind":"reject_once"}],
               "native":{"plan":"1. Add caching"}}),
    ] {
        view.apply_json(&e.to_string()).unwrap();
    }

    let out = view.ask_spec(view.ask("q").unwrap());
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let spec = out.spec.unwrap();
    let req = &spec["nodes"][0];
    assert_eq!(req["options"], json!([]), "a text-only question has no options");
    assert_eq!(
        req["questions"],
        json!([
            {"text": "Name it?", "allows_text": true},
            {"text": "Which DB?", "header": "DB", "options": [{"label": "Postgres"}]}
        ])
    );
    let fallback = spec["fallback"].as_str().unwrap();
    assert!(fallback.contains("Which DB? (Postgres)"), "{fallback}");

    let out = view.ask_spec(view.ask("p").unwrap());
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let spec = out.spec.unwrap();
    assert_eq!(spec["nodes"][0]["plan"], "1. Add caching");
    assert!(spec["fallback"].as_str().unwrap().contains("1. Add caching"));
}

#[test]
fn agents_cannot_send_questions_or_plans() {
    let out = normalize_value(
        json!({"root": {"type": "request", "request_id": "r", "kind": "question",
        "title": "x", "questions": [{"text": "Password?", "allows_text": true}]}}),
    );
    assert!(!out.errors.is_empty());
    assert!(out.spec.is_none() || !out.spec.as_ref().unwrap().to_string().contains("Password"));
}
