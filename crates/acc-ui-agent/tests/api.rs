//! The submit tag, tool recognition and the host scripts.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use acc_ui_agent::{
    Answer, AskState, UiCallKind, answer_script, mcp_server, parse_submit, recognize_tool, submit_prompt,
};
use serde_json::{Map, Value, json};

fn values(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

// Submit tag

#[test]
fn submit_round_trips() {
    let v = values(json!({ "env": "prod", "note": "a </acc-ui-submit> b", "n": 3, "on": true }));
    let prompt = submit_prompt("deploy-1", &v);
    assert_eq!(prompt.matches("</acc-ui-submit>").count(), 1);
    let s = parse_submit(&format!("Here you go: {prompt} thanks")).unwrap();
    assert_eq!(s.id, "deploy-1");
    assert_eq!(s.values, v);
    assert_eq!(s.rest, "Here you go: thanks");
}

#[test]
fn submit_parse_rejects_junk() {
    assert!(parse_submit("no tag").is_none());
    assert!(parse_submit(r#"<acc-ui-submit id="x">not json</acc-ui-submit>"#).is_none());
    assert!(parse_submit(r#"<acc-ui-submit id="x">[1]</acc-ui-submit>"#).is_none());
    assert!(parse_submit(r#"<acc-ui-submit id="x">{}"#).is_none());
}

// Tool recognition

#[test]
fn recognizes_tool_names_of_every_harness() {
    let input = json!({ "spec": { "root": { "type": "badge", "text": "x" } } });
    for name in [
        "mcp__acc_ui__acc_ui_render",
        "acc_ui/acc_ui_render",
        "acc_ui_render",
        "acc_ui: acc_ui_render",
        "acc_ui_render (acc_ui MCP Server)",
    ] {
        let call = recognize_tool(name, &input, None).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(call.kind, UiCallKind::Render);
        assert!(call.ask.is_none());
        assert!(call.spec.spec.is_some());
    }
    assert_eq!(
        recognize_tool("mcp__acc_ui__acc_ui_ask", &input, None)
            .unwrap()
            .kind,
        UiCallKind::Ask
    );
    for name in ["Bash", "acc_ui_renderer", "xacc_ui_render", "mcp__other__render"] {
        assert!(recognize_tool(name, &input, None).is_none(), "{name}");
    }
}

#[test]
fn recognizes_specs_in_any_shape() {
    let spec = json!({ "root": { "type": "badge", "text": "x" } });
    let as_string = json!({ "spec": spec.to_string() });
    let bare = spec.clone();
    let whole_args_string = Value::String(json!({ "spec": spec }).to_string());
    for input in [as_string, bare] {
        assert!(
            recognize_tool("acc_ui_render", &input, None)
                .unwrap()
                .spec
                .spec
                .is_some()
        );
    }
    // Arguments as one JSON string aren't unwrapped twice; the spec is missing.
    let call = recognize_tool("acc_ui_render", &whole_args_string, None).unwrap();
    assert!(call.spec.spec.is_none());
    let none = recognize_tool("acc_ui_render", &json!({}), None).unwrap();
    assert!(none.spec.spec.is_none() && !none.spec.errors.is_empty());
}

#[test]
fn ask_state_follows_the_output() {
    let input = json!({ "spec": { "id": "q1", "root": { "type": "form", "fields": [{ "name": "a", "kind": "text" }] } } });
    let state = |out: Option<&str>| recognize_tool("acc_ui_ask", &input, out).unwrap().ask.unwrap();
    assert_eq!(state(None), AskState::Waiting);

    let answer = acc_ui_agent::host::answer_result("q1", &Answer::Values(values(json!({ "a": "yes" }))));
    let text = answer["content"][0]["text"].as_str().unwrap();
    match state(Some(text)) {
        AskState::Answered(s) => {
            assert_eq!(s.id, "q1");
            assert_eq!(s.values["a"], "yes");
        }
        other => panic!("{other:?}"),
    }
    let dismissed = acc_ui_agent::host::answer_result("q1", &Answer::Dismissed);
    assert_eq!(
        state(dismissed["content"][0]["text"].as_str()),
        AskState::Dismissed
    );
    assert_eq!(state(Some("acc-ui: timed out. ...")), AskState::TimedOut);
    assert!(matches!(state(Some("acc-ui: error: bad")), AskState::Failed(_)));
}

// Host scripts

#[test]
fn mcp_server_launch() {
    let s = mcp_server("/home/u/.acc-ui/bin/acc-ui-mcp-abc.sh", "sess-1").unwrap();
    assert_eq!(s.name, "acc_ui");
    assert_eq!(s.command, "sh");
    assert_eq!(s.args, ["/home/u/.acc-ui/bin/acc-ui-mcp-abc.sh"]);
    assert_eq!(s.env, [("ACC_UI_SESSION".to_owned(), "sess-1".to_owned())]);
    assert!(mcp_server("/x", "bad session").is_err());
}

#[test]
fn answer_script_refuses_unsafe_ids() {
    let a = Answer::Dismissed;
    assert!(answer_script("s", "ok", &a).is_ok());
    for bad in ["", "..", "a/b", "$(x)", "a b", &"x".repeat(65)] {
        assert!(answer_script(bad, "ok", &a).is_err(), "{bad:?}");
        assert!(answer_script("s", bad, &a).is_err(), "{bad:?}");
    }
}

#[test]
fn setup_script_is_content_addressed() {
    let s = acc_ui_agent::setup_script();
    assert!(s.contains(&acc_ui_agent::server_script()));
    let path_line = s.lines().find(|l| l.starts_with("f=")).unwrap();
    assert!(path_line.contains("/acc-ui-mcp-"), "{path_line}");
}

#[test]
fn submit_ids_are_escaped_and_restored() {
    let v = values(json!({ "a": 1 }));
    let id = r#"a"b<c>&amp;d"#;
    let prompt = submit_prompt(id, &v);
    assert!(!prompt.contains("a\"b"), "quote must be escaped: {prompt}");
    let s = parse_submit(&prompt).unwrap();
    assert_eq!(s.id, id);
    assert_eq!(s.values, v);
    assert_eq!(s.rest, "");
}

#[test]
fn submit_rest_keeps_text_on_both_sides() {
    let prompt = submit_prompt("x", &values(json!({})));
    let s = parse_submit(&format!("before\n{prompt}\nafter")).unwrap();
    assert_eq!(s.rest, "before after");
    let s = parse_submit(&format!("  {prompt}  ")).unwrap();
    assert_eq!(s.rest, "");
}
