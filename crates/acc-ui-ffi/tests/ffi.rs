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

fn recorded(name: &str) -> String {
    let path = format!("{}/../acc-ui/tests/sessions/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn session_view_replays_a_recorded_session() {
    let view = SessionView::new();
    view.set_root("/work".to_owned());
    let mut touched_turns = 0;
    for line in recorded("codex-fix-and-review.ndjson").lines() {
        let ch = view.apply_json(line.to_owned()).unwrap();
        touched_turns += ch.turns.len();
        for id in &ch.commands {
            assert!(view.command(id.clone()).is_some(), "changes name commands it has");
        }
        for path in &ch.files {
            assert!(view.file(path.clone()).is_some());
        }
    }
    assert!(touched_turns > 0);
    let snap = view.snapshot();
    assert_eq!(snap.header.turns as usize, snap.turns.len());
    assert!(matches!(snap.header.state, RunState::Done));
    let fix = &snap.turns[0];
    assert!(fix.digest.summary.starts_with("Edited 3 files"));
    assert!(matches!(fix.digest.parts[0], DigestPart::Edited { files: 3, .. }));
    assert!(fix.duration_ms.is_some());
    assert!(
        snap.commands
            .iter()
            .all(|c| c.outcome == CommandOutcome::Succeeded)
    );
    assert!(snap.files.iter().any(|f| f.display == "inventory.py"));
    assert_eq!(view.turn(0).unwrap(), *fix);
    assert!(view.turn(99).is_none());

    let answer = fix.narrative_item.clone().unwrap();
    let spec = view.item_spec(answer).unwrap();
    assert!(spec.spec_json.is_some(), "{:?}", spec.errors);
    assert!(view.apply_json("not json".to_owned()).is_err());
}

#[test]
fn session_view_restores_from_a_transcript_and_answers_asks() {
    let transcript = r#"{"items":[
        {"id":"u","kind":"user_message","status":"completed","text":"list files","detail":{"type":"text"},"seq":0},
        {"id":"c","kind":"command_execution","status":"completed","text":"",
         "detail":{"type":"command","tool":"Bash","command":"ls","output":"a"},"seq":1}
    ],"next_seq":2}"#;
    let view = SessionView::from_transcript(transcript.to_owned()).unwrap();
    assert_eq!(view.turns()[0].prompt.as_deref(), Some("list files"));
    assert_eq!(view.commands()[0].outcome, CommandOutcome::Unknown);
    let items = view.items();
    assert_eq!(items[1].output.as_deref(), Some("a"), "a command's full output");
    assert_eq!(items[0].output, None);
    assert!(SessionView::from_transcript("nope".to_owned()).is_err());

    let view = SessionView::new();
    view.apply_json(
        r#"{"type":"permission_requested","request_id":"r","title":"Bash: ls",
            "options":[{"id":"y","label":"Allow","kind":"allow_once"}],
            "detail":{"type":"command","tool":"Bash","command":"ls","output":""}}"#
            .replace('\n', ""),
    )
    .unwrap();
    let ask = view.ask("r".to_owned()).unwrap();
    assert_eq!(ask.options[0].kind, Some(OptionKind::AllowOnce));
    assert!(matches!(ask.subject, Some(ItemDetail::Command { .. })));
    let spec = view.ask_spec("r".to_owned()).unwrap();
    assert!(spec.spec_json.unwrap().contains("\"request\""));
    assert!(view.answered("r".to_owned()).asks);
    assert!(view.ask_spec("r".to_owned()).is_none());
}

#[test]
fn links_carry_utf16_ranges() {
    let text = "Fixed 🎉 in src/main.rs:12 and https://x.dev".to_owned();
    let links = detect_links(text.clone(), Some("/repo".to_owned()));
    let file = links.iter().find(|l| l.kind == LinkKind::File).unwrap();
    assert_eq!(&text[file.start as usize..file.end as usize], "src/main.rs:12");
    let utf16: Vec<u16> = text.encode_utf16().collect();
    assert_eq!(
        String::from_utf16(&utf16[file.start_utf16 as usize..file.end_utf16 as usize]).unwrap(),
        "src/main.rs:12"
    );
    assert_eq!(
        file.start - file.start_utf16,
        2,
        "the emoji is 4 bytes, 2 UTF-16 units"
    );
    let blocks = markdown_blocks("| a |\n|---|\n| 1 |".to_owned());
    assert!(blocks.spec_json.unwrap().contains("table"));
}

#[test]
fn questions_answer_through_a_draft() {
    let view = SessionView::new();
    let event = serde_json::json!({"type":"permission_requested","request_id":"q","title":"Which DB?",
        "kind":"question","native":{"questions":[
            {"id":"q1","question":"Which DB?","isOther":false,"options":[{"label":"Postgres","description":"server"}]},
            {"id":"q2","question":"Name?","isOther":true,"isSecret":true,"options":null}]}});
    view.apply_json(event.to_string()).unwrap();
    let ask = view.ask("q".into()).unwrap();
    assert_eq!(ask.questions[0].options[0].description.as_deref(), Some("server"));
    assert!(ask.questions[1].secret && ask.questions[1].allows_text);

    let draft = view.answer_draft("q".into()).unwrap();
    assert_eq!(draft.missing(), [0, 1]);
    assert!(draft.pick(0, 0));
    assert!(!draft.set_text(0, "MySQL".into()));
    assert!(draft.set_text(1, "acme".into()));
    assert_eq!(draft.picked(0), [0]);
    assert_eq!(draft.option_id().as_deref(), Some(r#"[["Postgres"],["acme"]]"#));
    assert_eq!(
        encode_answers(vec![vec!["Postgres".into()], vec!["acme".into()]]),
        draft.option_id().unwrap()
    );
    assert!(view.answered(draft.request_id()).asks);
    assert!(view.answer_draft("q".into()).is_none());
}
