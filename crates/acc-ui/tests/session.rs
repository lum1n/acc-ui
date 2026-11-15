//! The session model against recorded ACC sessions and hand-written events.
//!
//! Each `tests/sessions/<name>.ndjson` (recorded with
//! `scripts/record-fixture.py`) replays into `<name>.session.json`: the final
//! snapshot plus the regions' state at the end of every turn.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use acc_ui::SessionView;
use acc_ui::session::event::ItemKind;
use acc_ui::session::{ActivityKind, Digest, Event, TurnOutcome};
use serde_json::{Value, json};

fn replay(lines: &str) -> (SessionView, Vec<Value>) {
    replay_into(SessionView::new(), lines)
}

fn replay_into(mut view: SessionView, lines: &str) -> (SessionView, Vec<Value>) {
    let mut turns = Vec::new();
    for (n, line) in lines.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: Event =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("line {}: {e}\n{line}", n + 1));
        let known = !matches!(event, Event::Unknown);
        let ty = serde_json::from_str::<Value>(line).unwrap()["type"].clone();
        let ignored = [
            "commands_changed",
            "options_changed",
            "panel",
            "native",
            "unrecognized",
        ];
        assert!(
            known || ignored.contains(&ty.as_str().unwrap_or("")),
            "line {}: event type {ty} isn't read",
            n + 1
        );
        let is_turn_end = matches!(event, Event::TurnCompleted);
        view.apply(event);
        if is_turn_end {
            let s = view.snapshot();
            turns.push(json!({
                "turn": s.turns.last().map(|t| t.index),
                "state": s.header.state,
                "now": s.now,
                "asks": s.asks.len(),
                "files": s.files.iter().map(|f| json!([f.display, f.change, f.added, f.removed, f.reads])).collect::<Vec<_>>(),
                "commands": s.commands.iter().map(|c| json!([c.display.lines().next(), c.exit_code])).collect::<Vec<_>>(),
                "subagents": s.subagents.iter().map(|a| json!([a.agent, a.status, a.items])).collect::<Vec<_>>(),
                "digest": s.turns.last().map(|t| t.digest.summary()),
                "outcome": s.turns.last().map(|t| t.outcome),
                "narrative": s.turns.last().and_then(|t| t.narrative.as_deref()).map(|n| n.chars().take(80).collect::<String>()),
            }));
        }
    }
    (view, turns)
}

#[test]
fn recorded_sessions_match_golden_state() {
    let dir = common::dir("sessions");
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "ndjson") {
            continue;
        }
        let (view, turns) = replay(&std::fs::read_to_string(&path).unwrap());
        let actual = serde_json::to_string_pretty(&json!({
            "turn_ends": turns,
            "final": view.snapshot(),
        }))
        .unwrap();
        common::golden(&path.with_extension("session.json"), &(actual + "\n"));
        n += 1;
    }
    assert!(n >= 2);
}

#[test]
fn recorded_sessions_fill_every_region() {
    for name in ["claude-fix-and-review", "codex-fix-and-review"] {
        let path = common::dir("sessions").join(format!("{name}.ndjson"));
        // Clients know the working directory; Claude's commands don't say it.
        let mut view = SessionView::new();
        view.set_root("/work");
        let (view, turns) = replay_into(view, &std::fs::read_to_string(path).unwrap());
        let h = view.header();
        assert!(
            h.harness.is_some() && h.model.is_some() && h.title.is_some(),
            "{name}"
        );
        // Codex on a ChatGPT plan reports usage without a cost.
        assert!(h.usage.is_some(), "{name}");
        assert!(h.title.as_ref().unwrap().chars().count() <= 81, "{name}");
        assert_eq!(turns.len(), 2, "{name}");
        assert_eq!(view.turns().len(), 2, "{name}");
        assert!(
            view.turns().iter().all(|t| !t.open && t.prompt.is_some()),
            "{name}"
        );
        assert!(view.asks().is_empty(), "{name}: asks left after the session");
        assert!(view.now().is_empty(), "{name}: still running after the session");

        let changed: Vec<_> = view
            .files()
            .iter()
            .filter(|f| f.change.is_some())
            .map(|f| f.display.as_str())
            .collect();
        for f in ["inventory.py", "test_inventory.py", "README.md"] {
            assert!(
                changed.contains(&f),
                "{name}: {f} not among changed files {changed:?}"
            );
        }
        let readme = view.files().iter().find(|f| f.display == "README.md").unwrap();
        assert_eq!(readme.change, Some(acc_ui::session::event::FileChangeKind::Add));

        // Claude reports no exit codes, only whether the call completed.
        let tests_ran = view.commands().iter().any(|c| {
            c.display.contains("unittest")
                && c.status == acc_ui::session::event::ItemStatus::Completed
                && c.exit_code.unwrap_or(0) == 0
        });
        assert!(tests_ran, "{name}: no passing test run");

        assert_eq!(view.subagents().len(), 1, "{name}: {:?}", view.subagents());
        let sub = &view.subagents()[0];
        assert!(sub.report.is_some() && sub.items > 0, "{name}: {sub:?}");
        assert_eq!(sub.turn, 1, "{name}");

        let [fix, review] = view.turns() else { panic!() };
        for t in [fix, review] {
            assert_eq!(t.outcome, TurnOutcome::Done, "{name}");
            assert!(
                t.narrative.as_deref().is_some_and(|n| !n.trim().is_empty()),
                "{name}"
            );
            assert_eq!(
                t.narrative_item.as_ref(),
                t.items
                    .iter()
                    .rev()
                    .find(|i| view.item(i).unwrap().kind == ItemKind::AgentMessage),
                "{name}"
            );
        }
        let d = &fix.digest;
        assert_eq!(
            (d.files_changed, d.files_created, d.commands_failed, d.subagents),
            (3, 1, 0, 0),
            "{name}: {d:?}"
        );
        assert!(d.commands >= 1 && d.lines_added > 0, "{name}: {d:?}");
        assert!(
            fix.digest.summary().starts_with("Edited 3 files (+"),
            "{name}: {}",
            fix.digest.summary()
        );
        let d = &review.digest;
        assert_eq!((d.files_changed, d.subagents), (0, 1), "{name}: {d:?}");
        assert!(d.commands >= 1, "{name}: the subagent's commands count: {d:?}");
        assert!(
            review.narrative.as_deref().unwrap().contains('|'),
            "{name}: the answer is a markdown table"
        );
    }
}

/// ACC's transcript of a recorded session: its item events folded into
/// items, as `acc_core::transcript::Transcript` stores them.
fn transcript(lines: &str) -> Value {
    let mut items: Vec<Value> = Vec::new();
    let find = |items: &mut Vec<Value>, id: &str| items.iter().position(|i| i["id"] == id);
    for line in lines.lines() {
        let e: Value = serde_json::from_str(line).unwrap();
        let id = e["id"].as_str().unwrap_or("");
        match e["type"].as_str().unwrap() {
            "item_started" => {
                let seq = items.len();
                items.push(
                    json!({"id": id, "kind": e["kind"], "status": "running", "text": "",
                    "detail": e["detail"], "native": null, "parent": e["parent"], "seq": seq}),
                );
            }
            // Like ACC's transcript, frames for items it never saw start
            // are dropped (Codex completes `subAgentActivity` calls it
            // never started).
            ty @ ("item_delta" | "item_updated" | "item_completed") => {
                let Some(i) = find(&mut items, id) else { continue };
                match ty {
                    "item_delta" => {
                        let text =
                            items[i]["text"].as_str().unwrap().to_owned() + e["text"].as_str().unwrap();
                        items[i]["text"] = text.into();
                    }
                    "item_updated" => items[i]["detail"] = e["detail"].clone(),
                    _ => {
                        items[i]["detail"] = e["detail"].clone();
                        items[i]["status"] = e["status"].clone();
                    }
                }
            }
            _ => {}
        }
    }
    json!({"items": items, "next_seq": items.len()})
}

#[test]
fn transcripts_restore_the_same_regions_as_live_events() {
    for name in ["claude-fix-and-review", "codex-fix-and-review"] {
        let lines = std::fs::read_to_string(common::dir("sessions").join(format!("{name}.ndjson"))).unwrap();
        let (live, _) = replay(&lines);
        let t = transcript(&lines);
        let snapshot = json!({"v": 1, "spec": {"harness": live.header().harness, "cwd": "/work", "title": "Saved"},
                              "transcript": t, "usage": live.header().usage, "permissions": [], "exited": false});
        for (json, from) in [
            (t.to_string(), "transcript"),
            (t["items"].to_string(), "items"),
            (snapshot.to_string(), "snapshot"),
        ] {
            let restored = SessionView::from_transcript(&json).unwrap();
            let strip = |v: &SessionView| {
                let s = v.snapshot();
                let turns: Vec<_> = s
                    .turns
                    .iter()
                    .map(|t| json!([t.prompt, t.narrative, t.items, t.digest, t.outcome, t.open]))
                    .collect();
                let files: Vec<_> = s
                    .files
                    .iter()
                    .map(|f| json!([f.path, f.change, f.added, f.removed, f.reads, f.edits.len()]))
                    .collect();
                let commands: Vec<_> = s
                    .commands
                    .iter()
                    .map(|c| json!([c.command, c.status, c.exit_code, c.output_tail]))
                    .collect();
                let subagents: Vec<_> = s
                    .subagents
                    .iter()
                    .map(|a| json!([a.agent, a.status, a.report, a.items, a.commands]))
                    .collect();
                json!({"turns": turns, "files": files, "commands": commands, "subagents": subagents, "now": s.now.len()})
            };
            assert_eq!(strip(&restored), strip(&live), "{name} from {from}");
            if from == "snapshot" {
                assert_eq!(restored.header().title.as_deref(), Some("Saved"));
                assert_eq!(restored.header().root.as_deref(), Some("/work"));
                assert!(
                    !restored.files()[0].display.contains('/'),
                    "{name}: shown relative to the root"
                );
                assert_eq!(restored.header().usage, live.header().usage);
            }
        }
    }
}

#[test]
fn restoring_keeps_a_running_turn_open() {
    let items = json!([
        {"id":"u","kind":"user_message","status":"completed","text":"Go","detail":{"type":"text"},"seq":0},
        {"id":"c","kind":"command_execution","status":"running","text":"building","detail":{"type":"command","tool":"x","command":"make","output":""},"seq":1}
    ]);
    let view = SessionView::from_transcript(&items.to_string()).unwrap();
    assert!(view.turns()[0].open);
    assert_eq!(view.now()[0].kind, ActivityKind::Running);
    assert_eq!(view.commands()[0].output_tail, "building");
}

#[test]
fn turn_outcomes_and_durations() {
    let mut view = SessionView::new();
    let prompt =
        |id: &str| json!({"type":"item_started","id":id,"kind":"user_message","detail":{"type":"text"}});
    let state = |s: &str, ms: u64| json!({"type":"state_changed","state":{"state":s},"observed_at_ms":ms});
    apply(
        &mut view,
        &[
            prompt("u1"),
            state("working", 1_000),
            state("done", 4_500),
            json!({"type":"turn_completed"}),
            prompt("u2"),
            state("working", 10_000),
            prompt("u3"), // sent mid-turn
            state("error", 12_000),
            json!({"type":"turn_completed"}),
            prompt("u4"),
            state("waiting", 13_000),
            json!({"type":"turn_completed"}),
            prompt("u5"),
            json!({"type":"exited","code":1}),
        ],
    );
    let outcomes: Vec<_> = view.turns().iter().map(|t| t.outcome).collect();
    assert_eq!(
        outcomes,
        [
            TurnOutcome::Done,
            TurnOutcome::Interrupted,
            TurnOutcome::Failed,
            TurnOutcome::Waiting,
            TurnOutcome::Interrupted
        ]
    );
    assert_eq!(view.turns()[0].duration_ms(), Some(3_500));
    assert_eq!(view.turns()[1].duration_ms(), Some(0));

    // The working state usually comes just before the prompt item.
    let mut view = SessionView::new();
    apply(
        &mut view,
        &[
            state("working", 1_000),
            prompt("u1"),
            state("done", 9_000),
            json!({"type":"turn_completed"}),
        ],
    );
    assert_eq!(view.turns()[0].duration_ms(), Some(8_000));
}

#[test]
fn digest_summaries() {
    let mut d = Digest::default();
    assert_eq!(d.summary(), "");
    d.files_read = 1;
    assert_eq!(d.summary(), "Read 1 file");
    d.files_changed = 3;
    d.lines_added = 19;
    d.lines_removed = 3;
    d.commands = 3;
    d.commands_failed = 1;
    d.subagents = 1;
    assert_eq!(
        d.summary(),
        "Edited 3 files (+19 −3) · read 1 file · ran 3 commands, 1 failed · 1 subagent"
    );
}

fn apply(view: &mut SessionView, events: &[Value]) {
    for e in events {
        view.apply_json(&e.to_string()).unwrap();
    }
}

#[test]
fn plan_tracks_progress() {
    let mut view = SessionView::new();
    let changes = view
        .apply_json(
            &json!({"type":"plan","entries":[
                {"content":"Read the code","status":"completed"},
                {"content":"Fix it","status":"in_progress"},
                {"content":"Run tests","status":"pending"},
                {"content":"Later","status":"someday"}
            ]})
            .to_string(),
        )
        .unwrap();
    assert!(changes.plan);
    let plan = view.plan().unwrap();
    assert_eq!((plan.completed, plan.total, plan.current), (1, 4, Some(1)));
}

#[test]
fn asks_clear_when_answered_or_their_item_finishes() {
    let mut view = SessionView::new();
    let ask = |id: &str, item: &str| {
        json!({"type":"permission_requested","request_id":id,"title":"Bash: ls","item_id":item,
               "options":[{"id":"allow","label":"Allow","kind":"allow_once"}],
               "detail":{"type":"command","tool":"Bash","command":"ls","output":""}})
    };
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"u1","kind":"user_message","detail":{"type":"text"}}),
            json!({"type":"item_started","id":"c1","kind":"command_execution","detail":{"type":"command","tool":"Bash","command":"ls","output":""}}),
            ask("r1", "c1"),
            ask("r2", "c2"),
            ask("r3", "c3"),
        ],
    );
    assert_eq!(view.asks().len(), 3);
    assert_eq!(view.asks()[0].turn, 0);

    let ch = view.answered("r2");
    assert!(ch.asks);
    apply(
        &mut view,
        &[
            json!({"type":"item_completed","id":"c1","status":"completed","detail":{"type":"command","tool":"Bash","command":"ls","output":"a\nb","exit_code":0}}),
        ],
    );
    assert_eq!(
        view.asks()
            .iter()
            .map(|a| a.request_id.as_str())
            .collect::<Vec<_>>(),
        ["r3"]
    );

    apply(&mut view, &[json!({"type":"turn_completed"})]);
    assert!(view.asks().is_empty());
    assert_eq!(view.commands()[0].output_tail, "a\nb");
}

#[test]
fn a_turn_without_a_prompt_opens_on_its_own() {
    let mut view = SessionView::new();
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"u1","kind":"user_message","detail":{"type":"text"}}),
            json!({"type":"item_delta","id":"u1","text":"Fix the bug\nplease"}),
            json!({"type":"turn_completed"}),
            // A background subagent's report wakes the agent: no user message.
            json!({"type":"item_started","id":"m1","kind":"agent_message","detail":{"type":"text"}}),
            json!({"type":"turn_completed"}),
        ],
    );
    let turns = view.turns();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].prompt.as_deref(), Some("Fix the bug\nplease"));
    assert_eq!(turns[1].prompt, None);
    assert_eq!(turns[1].items, ["m1"]);
    assert_eq!(view.header().title.as_deref(), Some("Fix the bug"));
}

#[test]
fn harness_title_wins_over_the_prompt() {
    let mut view = SessionView::new();
    apply(
        &mut view,
        &[
            json!({"type":"title_changed","title":"Bug fix"}),
            json!({"type":"item_started","id":"u1","kind":"user_message","detail":{"type":"text"}}),
            json!({"type":"item_delta","id":"u1","text":"Fix the bug"}),
        ],
    );
    assert_eq!(view.header().title.as_deref(), Some("Bug fix"));
}

#[test]
fn now_shows_running_items_and_coordination() {
    let mut view = SessionView::new();
    view.set_root("/work");
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"r","kind":"reasoning","detail":{"type":"text"}}),
            json!({"type":"item_started","id":"c","kind":"command_execution","detail":{"type":"command","tool":"x","command":"/bin/bash -lc 'cargo test\necho done'","output":""}}),
            json!({"type":"item_started","id":"f","kind":"file_change","detail":{"type":"file_change","tool":"Edit","files":[{"path":"/work/src/a.rs","kind":"update","hunks":[],"added":1,"removed":0}]}}),
            json!({"type":"item_started","id":"w","kind":"subagent","detail":{"type":"subagent","tool":"wait","description":"Wait for agent"}}),
            json!({"type":"item_started","id":"g","kind":"tool_call","detail":{"type":"tool","tool":"Grep","kind":"other","title":"TODO","input":{}}}),
        ],
    );
    let now: Vec<_> = view.now().into_iter().map(|a| (a.kind, a.target)).collect();
    assert_eq!(
        now,
        [
            (ActivityKind::Thinking, String::new()),
            (ActivityKind::Running, "cargo test".into()),
            (ActivityKind::Editing, "src/a.rs".into()),
            (ActivityKind::WaitingForSubagents, String::new()),
            (ActivityKind::Searching, "TODO".into()),
        ]
    );
    assert!(view.subagents().is_empty(), "wait isn't a subagent run");

    apply(
        &mut view,
        &[json!({"type":"item_completed","id":"r","status":"completed","detail":{"type":"text"}})],
    );
    assert_eq!(view.now().len(), 4);
}

#[test]
fn failed_edits_and_reads_land_in_files() {
    let mut view = SessionView::new();
    let edit = |id: &str, status: &str, kind: &str| {
        json!({"type":"item_completed","id":id,"status":status,
               "detail":{"type":"file_change","tool":"Edit","files":[{"path":"/w/a.py","kind":kind,"hunks":[],"added":2,"removed":1}]}})
    };
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"e1","kind":"file_change","detail":{"type":"file_change","tool":"Write","files":[]}}),
            edit("e1", "completed", "add"),
            json!({"type":"item_started","id":"e2","kind":"file_change","detail":{"type":"file_change","tool":"Edit","files":[]}}),
            edit("e2", "failed", "update"),
            json!({"type":"item_started","id":"e3","kind":"file_change","detail":{"type":"file_change","tool":"Edit","files":[]}}),
            edit("e3", "completed", "update"),
            json!({"type":"item_started","id":"r1","kind":"tool_call","detail":{"type":"tool","tool":"Read","kind":"read","title":"/w/b.py","input":{}}}),
            json!({"type":"item_completed","id":"r1","status":"completed","detail":{"type":"tool","tool":"Read","kind":"read","title":"/w/b.py","input":{},"output":"x"}}),
        ],
    );
    view.set_root("/w/");
    let files = view.files();
    assert_eq!(files.len(), 2);
    assert_eq!(
        (files[0].display.as_str(), files[0].edits.len(), files[0].added),
        ("a.py", 2, 4)
    );
    assert_eq!(files[0].change, Some(acc_ui::session::event::FileChangeKind::Add));
    assert_eq!(
        (files[1].display.as_str(), files[1].reads, files[1].change),
        ("b.py", 1, None)
    );
}

#[test]
fn unknown_events_and_fields_are_ignored() {
    let mut view = SessionView::new();
    let ch = view.apply_json(r#"{"type":"something_new","x":1}"#).unwrap();
    assert!(ch.is_empty());
    view.apply_json(
        r#"{"type":"item_started","id":"a","kind":"hologram","detail":{"type":"teleport"},"extra":true}"#,
    )
    .unwrap();
    assert_eq!(view.now()[0].kind, ActivityKind::Working);
    assert!(view.apply_json("not json").is_err());
}
