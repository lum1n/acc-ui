//! The agent layer read from a `SessionView`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use acc_ui::SessionView;
use acc_ui_agent::{AskState, Delivery, UiCallKind, agent_call, agent_calls, item_spec, open_agent_asks};
use serde_json::{Map, Value, json};

fn apply(view: &mut SessionView, events: &[Value]) {
    for e in events {
        view.apply_json(&e.to_string()).unwrap();
    }
}

fn spec(id: &str) -> Value {
    json!({"id": id, "root": {"type": "form", "fields": [{"type": "text", "id": "env", "label": "Env"}],
        "submit": {"type": "submit"}}})
}

fn tool(name: &str, input: Value, output: Option<&str>) -> Value {
    json!({"type": "tool", "tool": name, "kind": "mcp", "title": "", "input": input, "output": output})
}

fn values() -> Map<String, Value> {
    json!({"env": "prod"}).as_object().unwrap().clone()
}

#[test]
fn calls_are_found_under_every_harness_name() {
    let mut view = SessionView::new();
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"u","kind":"user_message","detail":{"type":"text"}}),
            json!({"type":"item_started","id":"r","kind":"tool_call",
                "detail": tool("mcp__acc_ui__acc_ui_render", json!({"spec": {"root": {"type": "badge", "text": "ok"}}}), None)}),
            json!({"type":"item_completed","id":"r","status":"completed",
                "detail": tool("mcp__acc_ui__acc_ui_render", json!({"spec": {"root": {"type": "badge", "text": "ok"}}}), Some("acc-ui: shown"))}),
            json!({"type":"item_started","id":"a","kind":"tool_call",
                "detail": tool("acc_ui/acc_ui_ask", json!({"spec": spec("deploy")}), None)}),
            json!({"type":"item_started","id":"x","kind":"tool_call",
                "detail": tool("mcp__other__render", json!({}), None)}),
        ],
    );
    let calls = agent_calls(&view);
    assert_eq!(
        calls.iter().map(|c| c.item_id.as_str()).collect::<Vec<_>>(),
        ["r", "a"]
    );
    assert_eq!(calls[0].call.kind, UiCallKind::Render);
    assert!(calls[0].call.spec.spec.is_some());
    assert!(agent_call(&view, "x").is_none());

    let open = open_agent_asks(&view);
    assert_eq!(open.len(), 1);
    let ask = &open[0];
    assert_eq!(
        (ask.spec_id(), ask.running, ask.call.ask.clone()),
        (Some("deploy"), true, Some(AskState::Waiting))
    );

    // While it waits, an answer is a script for the host.
    let Some(Ok(Delivery::Script(s))) = ask.answer("s1", Some(values())) else {
        panic!("script")
    };
    assert!(
        s.contains("s1") && s.contains("deploy") && s.contains("prod"),
        "{s}"
    );
    assert!(matches!(ask.answer("s1", None), Some(Ok(Delivery::Script(_)))));
    assert!(matches!(ask.answer("bad id!", Some(values())), Some(Err(_))));
}

#[test]
fn answered_dismissed_and_timed_out_asks() {
    let answered = "acc-ui: answered: <acc-ui-submit id=\"deploy\">{\"env\":\"prod\"}</acc-ui-submit>";
    let mut view = SessionView::new();
    let start = |id: &str| {
        json!({"type":"item_started","id":id,"kind":"tool_call",
            "detail": tool("mcp__acc_ui__acc_ui_ask", json!({"spec": spec(id)}), None)})
    };
    let done = |id: &str, status: &str, out: Option<&str>| {
        json!({"type":"item_completed","id":id,"status":status,
            "detail": tool("mcp__acc_ui__acc_ui_ask", json!({"spec": spec(id)}), out)})
    };
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"u","kind":"user_message","detail":{"type":"text"}}),
            start("a1"),
            done("a1", "completed", Some(answered)),
            start("a2"),
            done(
                "a2",
                "completed",
                Some("acc-ui: dismissed: the user closed \"a2\" without answering."),
            ),
            start("a3"),
            done("a3", "completed", Some("acc-ui: timed out")),
            // Completed with no output: the server went away.
            start("a4"),
            done("a4", "completed", None),
            start("a5"),
            done("a5", "failed", None),
        ],
    );
    let state = |id: &str| agent_call(&view, id).unwrap().call.ask.unwrap();
    assert!(matches!(state("a1"), AskState::Answered(_)));
    assert_eq!(state("a2"), AskState::Dismissed);
    assert_eq!(state("a3"), AskState::TimedOut);
    assert_eq!(state("a4"), AskState::TimedOut);
    assert!(matches!(state("a5"), AskState::Failed(_)));

    let open: Vec<String> = open_agent_asks(&view).into_iter().map(|c| c.item_id).collect();
    assert_eq!(open, ["a3", "a4"]);
    // Nobody waits any more: the answer goes as the next prompt, and a
    // dismissal needs nothing sent.
    let a3 = agent_call(&view, "a3").unwrap();
    let Some(Ok(Delivery::Prompt(p))) = a3.answer("s1", Some(values())) else {
        panic!("prompt")
    };
    assert_eq!(p, acc_ui_agent::submit_prompt("a3", &values()));
    assert_eq!(a3.answer("s1", None), None);
    assert_eq!(
        agent_call(&view, "a1").unwrap().answer("s1", Some(values())),
        None
    );
}

/// A recorded Claude session with the acc_ui server, launched the way ACC
/// passes `LaunchRequest.mcp_servers`: one render, then an ask answered
/// with the answer script while the call waited.
#[test]
fn recorded_claude_session_with_the_server() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../acc-ui/tests/sessions/claude-acc-ui-agent.ndjson");
    let mut view = SessionView::new();
    let mut waiting = Vec::new();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        view.apply_json(line).unwrap();
        waiting.extend(open_agent_asks(&view).into_iter().map(|c| c.item_id));
    }
    waiting.dedup();

    let [render, ask] = &agent_calls(&view)[..] else {
        panic!("{:?}", agent_calls(&view))
    };
    assert_eq!(render.call.kind, UiCallKind::Render);
    // Claude gave the render no id; normalizing gave it one.
    assert!(render.spec_id().unwrap().starts_with("u-"));
    assert!(
        render.call.spec.errors.is_empty(),
        "{:?}",
        render.call.spec.errors
    );
    let nodes = render.call.spec.spec.as_ref().unwrap()["nodes"].to_string();
    assert!(
        nodes.contains("\"table\"") && nodes.contains("notes.txt"),
        "{nodes}"
    );
    assert!(!render.awaits_answer());

    assert_eq!(ask.call.kind, UiCallKind::Ask);
    assert_eq!(ask.spec_id(), Some("pick-name"));
    assert!(ask.call.spec.errors.is_empty(), "{:?}", ask.call.spec.errors);
    // It waited for the answer while it ran, and took it.
    assert_eq!(waiting, std::slice::from_ref(&ask.item_id));
    let Some(AskState::Answered(s)) = &ask.call.ask else {
        panic!("{:?}", ask.call.ask)
    };
    assert_eq!(
        (s.id.as_str(), &s.values["name"]),
        ("pick-name", &json!("Aurora"))
    );
    assert!(open_agent_asks(&view).is_empty());
    assert_eq!(ask.answer("rec", Some(values())), None, "answered already");
    assert_eq!((render.turn, ask.turn), (0, 0));
}

#[test]
fn the_ask_tool_points_at_the_render_schema() {
    let tools = acc_ui_agent::host::tools_list();
    let render = tools["tools"][0]["inputSchema"].to_string();
    let ask = tools["tools"][1]["inputSchema"].to_string();
    assert!(ask.len() < 400, "{} bytes", ask.len());
    assert!(render.len() > 5000);
    assert!(
        tools.to_string().len() < 17_000,
        "{} bytes",
        tools.to_string().len()
    );
}

#[test]
fn item_spec_shows_a_calls_own_spec() {
    let mut view = SessionView::new();
    // A request node can't come from an agent: normalising drops it.
    let sneaky = json!({"spec": {"root": {"type": "stack", "children": [
        {"type": "badge", "text": "ok"},
        {"type": "request", "request_id": "r", "kind": "permission", "title": "Allow?", "options": []}
    ]}}});
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"u","kind":"user_message","detail":{"type":"text"}}),
            json!({"type":"item_started","id":"r","kind":"tool_call",
                "detail": tool("mcp__acc_ui__acc_ui_render", sneaky, None)}),
            json!({"type":"item_started","id":"bad","kind":"tool_call",
                "detail": tool("mcp__acc_ui__acc_ui_render", json!({"spec": "not json"}), None)}),
            json!({"type":"item_started","id":"x","kind":"tool_call",
                "detail": tool("mcp__other__render", json!({}), Some("done"))}),
        ],
    );
    let nodes = item_spec(&view, "r").unwrap().spec.unwrap()["nodes"].to_string();
    assert!(nodes.contains("\"badge\""), "{nodes}");
    assert!(!nodes.contains("\"request\""), "{nodes}");
    // A spec that didn't survive, and other tools, show as tool calls.
    for id in ["bad", "x"] {
        let spec = item_spec(&view, id).unwrap().spec.unwrap();
        assert_eq!(spec["nodes"][0]["type"], "disclosure", "{id}");
        assert_eq!(spec, view.item_spec(id).unwrap().spec.unwrap());
    }
}
