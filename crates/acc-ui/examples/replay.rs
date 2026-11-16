//! Replays a recorded ACC session through a `SessionView` and writes the
//! regions each event changed, as frames.
//!
//! ```sh
//! cargo run -p acc-ui --example replay -- crates/acc-ui/tests/sessions/codex-fix-and-review.ndjson \
//!     --root /work --html /tmp/replay.html
//! ```
//!
//! Without `--html` the frames go to stdout as NDJSON. With it they're
//! embedded in a self-contained viewer (`examples/replay/viewer.html`) that
//! plays the session back region by region. The agent's own `acc_ui`
//! calls come from `acc-ui-agent` (a dev-dependency only).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write as _;

use acc_ui::SessionView;
use acc_ui_agent::{AgentCall, AskState, UiCallKind, agent_calls};
use serde_json::{Map, Value, json};

const VIEWER: &str = include_str!("replay/viewer.html");
/// Consecutive text deltas merged into one frame, at most.
const DELTAS_PER_FRAME: usize = 6;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut input = None;
    let mut root = None;
    let mut html = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = args.next(),
            "--html" => html = args.next(),
            _ => input = Some(a),
        }
    }
    let input = input.expect("usage: replay <session.ndjson> [--root DIR] [--html OUT]");
    let text = std::fs::read_to_string(&input).unwrap();

    let mut view = SessionView::new();
    if let Some(r) = &root {
        view.set_root(r.clone());
    }
    let mut frames: Vec<Value> = Vec::new();
    let mut merged = 0;
    let mut calls_shown = json!([]);
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let ty = serde_json::from_str::<Value>(line).unwrap()["type"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let ch = view.apply_json(line).unwrap();
        if ch.is_empty() {
            continue;
        }
        let mut set = Map::new();
        if ch.header {
            set.insert("header".into(), json!(view.header()));
        }
        if ch.now {
            set.insert("now".into(), json!(view.now()));
        }
        if ch.plan {
            set.insert("plan".into(), json!(view.plan()));
        }
        if ch.asks {
            set.insert("asks".into(), json!(view.asks()));
        }
        if ch.notices {
            set.insert("notices".into(), json!(view.notices()));
        }
        if !ch.files.is_empty() {
            set.insert("files".into(), json!(view.files()));
        }
        if !ch.commands.is_empty() {
            set.insert("commands".into(), json!(view.commands()));
        }
        if !ch.subagents.is_empty() {
            set.insert("subagents".into(), json!(view.subagents()));
        }
        if !ch.turns.is_empty() {
            set.insert("turns".into(), json!(view.turns()));
        }
        if !ch.items.is_empty() {
            let calls = Value::Array(agent_calls(&view).iter().map(call_json).collect());
            if calls != calls_shown {
                set.insert("agent_calls".into(), calls.clone());
                calls_shown = calls;
            }
        }
        if set.is_empty() {
            continue;
        }
        // A streamed message changes the same regions event after event;
        // merge a few so the file stays small.
        let is_delta = ty == "item_delta";
        if let Some(last) = frames.last_mut()
            && is_delta
            && last["t"] == "item_delta"
            && merged < DELTAS_PER_FRAME
        {
            let last_set = last["set"].as_object_mut().unwrap();
            last_set.extend(set);
            last["i"] = json!(i);
            merged += 1;
            continue;
        }
        merged = 0;
        frames.push(json!({ "i": i, "t": ty, "set": set }));
    }

    match html {
        Some(out) => {
            let name = std::path::Path::new(&input)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Keep `</script>` in agent text from closing the data block.
            let data = serde_json::to_string(&frames).unwrap().replace("</", "<\\/");
            let page = VIEWER
                .replace("__SESSION_NAME__", &name)
                .replace("__FRAMES__", &data);
            std::fs::write(&out, page).unwrap();
            eprintln!("replay: {} frames -> {out}", frames.len());
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            for f in frames {
                writeln!(stdout, "{f}").unwrap();
            }
        }
    }
}

/// An agent's `acc_ui` call for the viewer: its normalised spec and, for an
/// ask, where the answer stands.
fn call_json(c: &AgentCall) -> Value {
    let (ask, answer) = match &c.call.ask {
        None => (Value::Null, Value::Null),
        Some(AskState::Waiting) => (json!("waiting"), Value::Null),
        Some(AskState::Answered(s)) => (json!("answered"), json!(s.values)),
        Some(AskState::Dismissed) => (json!("dismissed"), Value::Null),
        Some(AskState::TimedOut) => (json!("timed_out"), Value::Null),
        Some(AskState::Failed(why)) => (json!("failed"), json!(why)),
    };
    let kind = match c.call.kind {
        UiCallKind::Render => "render",
        UiCallKind::Ask => "ask",
    };
    json!({
        "item_id": c.item_id,
        "turn": c.turn,
        "kind": kind,
        "running": c.running,
        "ask": ask,
        "answer": answer,
        "spec": c.call.spec.spec,
        "errors": c.call.spec.errors,
    })
}
