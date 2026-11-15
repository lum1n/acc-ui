//! The FFI wrappers: JSON in and out as strings.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use acc_ui_ffi::*;

#[test]
fn normalize_returns_json_and_id() {
    let n = normalize(r#"{"id":"a","root":{"type":"badge","text":"x"}}"#.to_owned());
    assert_eq!(n.id.as_deref(), Some("a"));
    assert!(n.spec_json.unwrap().contains("\"nodes\""));
    let bad = normalize("nope".to_owned());
    assert!(bad.spec_json.is_none() && bad.id.is_none() && !bad.errors.is_empty());
}

#[test]
fn recognize_takes_input_as_json_text() {
    let input = r#"{"spec":{"id":"q","root":{"type":"badge","text":"x"}}}"#.to_owned();
    let call = recognize_tool("mcp__acc_ui__acc_ui_ask".to_owned(), input, None).unwrap();
    assert_eq!(call.kind, UiCallKind::Ask);
    assert_eq!(call.ask, Some(AskState::Waiting));
    // Input that isn't JSON is taken as a spec string.
    let raw = recognize_tool(
        "acc_ui_render".to_owned(),
        "{\"root\":{\"type\":\"badge\",\"text\":\"x\"}".to_owned(),
        None,
    )
    .unwrap();
    assert!(raw.spec.spec_json.is_none());
    assert!(recognize_tool("Bash".to_owned(), "{}".to_owned(), None).is_none());
}

#[test]
fn submit_round_trips_through_json_text() {
    let prompt = submit_prompt("a".to_owned(), r#"{"x":1}"#.to_owned()).unwrap();
    let s = parse_submit(prompt).unwrap();
    assert_eq!(s.id, "a");
    assert_eq!(s.values_json, r#"{"x":1}"#);
    assert!(submit_prompt("a".to_owned(), "[1]".to_owned()).is_err());
}

#[test]
fn host_helpers() {
    let s = mcp_server("/p.sh".to_owned(), "s".to_owned()).unwrap();
    assert_eq!(s.env.get("ACC_UI_SESSION").map(String::as_str), Some("s"));
    assert!(
        answer_script("s".to_owned(), "q".to_owned(), None)
            .unwrap()
            .contains("dismissed")
    );
    assert!(answer_script("s".to_owned(), "q".to_owned(), Some("1".to_owned())).is_err());
    assert_eq!(spec_version(), "acc-ui/1");
    assert!(schema_json().starts_with('{'));
}

#[test]
fn texts() {
    assert!(conventions().contains("```chart"));
    assert!(conventions_skill().starts_with("---\nname: acc-ui-formatting"));
    assert!(agent_skill_markdown().contains("name: acc-ui-components"));
    assert!(agent_instructions().contains("acc_ui_render"));
}
