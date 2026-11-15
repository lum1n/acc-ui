//! Text for the agent when it writes specs itself: the `acc-ui` skill and
//! the short session instructions. The component catalog in the skill is generated from the
//! schema, so it can't drift from what [`acc_ui::normalize`] accepts.

use serde_json::{Map, Value};

use crate::host::{ASK_TOOL, RENDER_TOOL};
use acc_ui::schema;

/// About 150 tokens for a session's system prompt: what exists and where the
/// details are.
pub fn instructions() -> String {
    format!(
        "You can show the user rich components instead of plain text: cards, key/value lists, \
tables, charts, Mermaid diagrams, checklists, progress, badges, file/PR/commit links, buttons \
and forms. Write a ```acc-ui fence holding an acc-ui/1 JSON spec \
({{\"v\":\"acc-ui/1\",\"id\":\"...\",\"root\":{{\"type\":\"card\",\"title\":\"...\",\"children\":[...]}}}}), \
or call {RENDER_TOOL} to show one or {ASK_TOOL} to show one and wait for the user's input. \
```mermaid fences render as diagrams. Use components for status, results, comparisons and \
choices; keep the prose around them short and don't repeat their content. The acc-ui skill \
lists every component. An answer to a fence arrives as \
<acc-ui-submit id=\"...\">{{values}}</acc-ui-submit>."
    )
}

/// The `SKILL.md` of the `acc-ui` skill.
pub fn skill_markdown() -> String {
    let schema = schema();
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let cat = Catalog { defs: &defs };
    let mut s = String::from(HEAD);
    s.push_str("\n## Components\n\nSet `type` to one of these. `?` marks optional fields.\n\n");
    for variant in one_of(defs.get("Node")) {
        let ty = variant
            .pointer("/properties/type/const")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let desc = text(variant.get("description"));
        s.push_str(&format!("- **`{ty}`**: {desc}"));
        if let Some(def) = variant
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| cat.resolve(r))
        {
            s.push_str(&cat.fields(def));
        }
        s.push('\n');
    }
    s.push_str("\n### Parts\n\n");
    for name in [
        "KvItem",
        "Column",
        "CheckItem",
        "Series",
        "Point",
        "Chip",
        "Field",
        "Choice",
    ] {
        if let Some(def) = defs.get(name) {
            let desc = text(def.get("description"));
            let line = format!("{desc}{}", cat.fields(def));
            s.push_str(&format!("- **{name}**: {}\n", line.trim_start()));
        }
    }
    s.push_str("\n### Values\n\n");
    for name in [
        "Tone",
        "Gap",
        "Align",
        "CheckState",
        "PrState",
        "ChartKind",
        "ButtonStyle",
        "FieldKind",
    ] {
        if let Some(def) = defs.get(name) {
            s.push_str(&format!("- **{name}**: {}\n", cat.values(def).join(", ")));
        }
    }
    s.push_str("\n### Actions\n\nAn `action` is an object with `type`:\n\n");
    for variant in one_of(defs.get("Action")) {
        let ty = variant
            .pointer("/properties/type/const")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let mut v = variant.clone();
        if let Some(Value::Object(p)) = v.get_mut("properties") {
            p.remove("type");
        }
        s.push_str(&format!(
            "- **`{ty}`**: {}{}\n",
            text(variant.get("description")),
            cat.fields(&v)
        ));
    }
    s.push_str(TAIL);
    s
}

const HEAD: &str = r#"---
name: acc-ui-components
description: Show the user rich components in the chat (cards, tables, charts, Mermaid diagrams, checklists, progress, file/PR/commit links, buttons and forms) with ```acc-ui fences or the acc_ui_render and acc_ui_ask tools. Use for status, results, comparisons, plans, metrics, and whenever you need the user to choose or fill something in.
---

# acc-ui

The user's app renders components natively. Use them where structure helps the user: a status
summary, a comparison table, a chart of numbers, a diagram of a flow, a checklist of steps, a
choice to make. Keep the prose around a component short and don't repeat what it shows.
Plain answers stay plain text.

## Delivering a component

- **Fence** (always works): a ```` ```acc-ui ```` block holding one spec. Shorthands:
  ```` ```mermaid ```` renders a diagram, ```` ```chart ```` holds one `chart` node, and
  ```` ```html render ```` shows self-contained HTML (plain ```` ```html ```` stays code).
- **`acc_ui_render(spec)`**: shows the spec and returns at once.
- **`acc_ui_ask(spec)`**: shows the spec and waits for the user's answer, returned as JSON.
  Use it when you can't carry on without the answer. The spec needs an `id` unique in the
  session. If the call times out, carry on or end your turn; a later answer arrives as a
  message (see below).

## The spec

```json
{
  "v": "acc-ui/1",
  "id": "deploy-status",
  "title": "Deploy",
  "root": {
    "type": "card",
    "title": "api-server",
    "children": [
      { "type": "kv", "items": [{ "label": "Version", "value": "2.4.1" }, { "label": "Region", "value": "eu-north-1" }] },
      { "type": "progress", "value": 0.6, "label": "Rolling out" },
      { "type": "button", "label": "Roll back", "style": "destructive",
        "action": { "type": "submit", "values": { "choice": "rollback" } } }
    ]
  }
}
```

- `root` is one node; `stack`, `row` and `card` hold `children`.
- `id`: 1-64 of `A-Z a-z 0-9 . _ -`. `fallback`: plain text for places that can't render it.
- Limits: 400 nodes, 12 levels, 256 KB, 500 table rows, 5000 chart points, 128 KB of HTML.
  Broken nodes are dropped and the rest still shows.
"#;

const TAIL: &str = r#"
Nothing runs until the user taps. `open_url` asks the user first.

## Answers

A form's submit, or a button or chip with a `submit` action, answers the component with an
object of values: the form's fields by `name`, merged with the action's `values`.

- From `acc_ui_ask`, the answer is the tool result.
- Otherwise it arrives as the user's next message:
  `<acc-ui-submit id="deploy-status">{"choice":"rollback"}</acc-ui-submit>`.
  Treat it as the user's reply to that component.

A `prompt` action sends its text as the user's message instead, which suits quick replies:

```json
{ "type": "chips", "options": [
  { "label": "Run the tests", "action": { "type": "prompt", "text": "Run the tests" } },
  { "label": "Open a PR", "action": { "type": "prompt", "text": "Open a PR" } }
] }
```

## Charts and diagrams

```json
{ "type": "chart", "kind": "line", "title": "p95 latency (ms)", "x_label": "day",
  "series": [
    { "name": "before", "points": [{ "x": "Mon", "y": 120 }, { "x": "Tue", "y": 135 }] },
    { "name": "after",  "points": [{ "x": "Mon", "y": 80 },  { "x": "Tue", "y": 78 }] }
  ] }
```

Mermaid: `{ "type": "mermaid", "code": "flowchart LR\n  A --> B" }`, or a ```` ```mermaid ```` fence.
"#;

fn one_of(def: Option<&Value>) -> Vec<Value> {
    def.and_then(|d| d.get("oneOf"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

struct Catalog<'a> {
    defs: &'a Map<String, Value>,
}

impl Catalog<'_> {
    fn resolve(&self, r: &str) -> Option<&Value> {
        self.defs.get(r.strip_prefix("#/$defs/")?)
    }

    /// `` Fields: `a` string; `b?` number: what b is.``
    fn fields(&self, def: &Value) -> String {
        let Some(props) = def.get("properties").and_then(Value::as_object) else {
            return String::new();
        };
        let required: Vec<&str> = def
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut keys: Vec<&String> = props.keys().collect();
        // Required fields first, each group in schema order.
        keys.sort_by_key(|k| !required.contains(&k.as_str()));
        let parts: Vec<String> = keys
            .into_iter()
            .map(|k| {
                let p = &props[k];
                let opt = if required.contains(&k.as_str()) { "" } else { "?" };
                let desc = text(p.get("description"));
                let desc = desc.trim_end_matches('.');
                let desc = if desc.is_empty() {
                    String::new()
                } else {
                    format!(" ({desc})")
                };
                format!("`{k}{opt}` {}{desc}", self.type_of(p))
            })
            .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!(" Fields: {}.", parts.join("; "))
        }
    }

    fn type_of(&self, p: &Value) -> String {
        if let Some(r) = p.get("$ref").and_then(Value::as_str) {
            let name = r.trim_start_matches("#/$defs/");
            return match name {
                "Node" => "node".to_owned(),
                "Action" => "action".to_owned(),
                "XValue" => "number|string".to_owned(),
                _ => name.to_owned(),
            };
        }
        if let Some(any) = p.get("anyOf").and_then(Value::as_array) {
            return any.iter().map(|a| self.type_of(a)).collect::<Vec<_>>().join("|");
        }
        match p.get("type").and_then(Value::as_str) {
            Some("array") => format!(
                "[{}]",
                p.get("items").map(|i| self.type_of(i)).unwrap_or_default()
            ),
            Some(t) => t.to_owned(),
            None => "any".to_owned(),
        }
    }

    fn values(&self, def: &Value) -> Vec<String> {
        if let Some(e) = def.get("enum").and_then(Value::as_array) {
            return e
                .iter()
                .filter_map(Value::as_str)
                .map(|s| format!("`{s}`"))
                .collect();
        }
        if let Some(c) = def.get("const").and_then(Value::as_str) {
            return vec![format!("`{c}`")];
        }
        one_of(Some(def)).iter().flat_map(|v| self.values(v)).collect()
    }
}
