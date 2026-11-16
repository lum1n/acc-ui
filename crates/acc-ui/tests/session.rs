//! The session model against recorded ACC sessions and hand-written events.
//!
//! Each `tests/sessions/<name>.ndjson` (recorded with
//! `scripts/record-fixture.py`) replays into `<name>.session.json`: the final
//! snapshot plus the regions' state at the end of every turn.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use acc_ui::SessionView;
use acc_ui::session::event::{ItemKind, OptionKind};
use acc_ui::session::{
    ActivityKind, AnswerDraft, CommandOutcome, Digest, DigestPart, Event, TurnOutcome, encode_answers,
};
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
    assert_eq!(view.now()[0].kind, ActivityKind::Building);
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

#[test]
fn digest_parts_are_structured() {
    let d = Digest {
        files_changed: 2,
        files_created: 1,
        lines_added: 5,
        lines_removed: 1,
        commands: 2,
        commands_failed: 1,
        tools: 1,
        ..Digest::default()
    };
    let parts = d.parts();
    assert_eq!(
        parts,
        [
            DigestPart::Edited {
                files: 2,
                created: 1,
                deleted: 0,
                added: 5,
                removed: 1
            },
            DigestPart::Ran {
                commands: 2,
                failed: 1
            },
            DigestPart::Tools { count: 1 },
        ]
    );
    assert_eq!(parts.iter().filter(|p| p.is_warning()).count(), 1);
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["parts"][1], json!({"kind":"ran","commands":2,"failed":1}));
    assert_eq!(
        v["summary"],
        "Edited 2 files (+5 −1) · ran 2 commands, 1 failed · 1 other tool"
    );
    assert!(Digest::default().parts().is_empty());
}

#[test]
fn commands_get_an_outcome_and_a_headline() {
    let mut view = SessionView::new();
    let cmd = |id: &str, command: &str, status: &str, exit: Value| {
        json!({"type":"item_completed","id":id,"status":status,
               "detail":{"type":"command","tool":"Bash","command":command,"output":"","exit_code":exit}})
    };
    let start = |id: &str| {
        json!({"type":"item_started","id":id,"kind":"command_execution",
               "detail":{"type":"command","tool":"Bash","command":"x","output":""}})
    };
    let long = "echo ".to_owned() + &"a".repeat(200);
    apply(
        &mut view,
        &[
            start("ok"),
            cmd("ok", "zsh -lc 'cargo test'", "completed", json!(0)),
            start("bad"),
            cmd("bad", "false", "completed", json!(1)),
            start("err"),
            cmd("err", "ls", "failed", Value::Null),
            start("unk"),
            cmd("unk", "cat a\ncat b", "completed", Value::Null),
            start("long"),
            cmd("long", &long, "completed", Value::Null),
            start("run"),
        ],
    );
    let c: Vec<_> = view
        .commands()
        .iter()
        .map(|c| (c.outcome, c.headline.as_str()))
        .collect();
    assert_eq!(c[0], (CommandOutcome::Succeeded, "cargo test"));
    assert_eq!(c[1], (CommandOutcome::Failed, "false"));
    assert_eq!(c[2].0, CommandOutcome::Failed);
    assert_eq!(c[3].0, CommandOutcome::Unknown);
    assert_eq!(c[5].0, CommandOutcome::Running);
    assert_eq!(c[4].1.chars().count(), acc_ui::session::HEADLINE_CHARS);
    assert!(c[4].1.ends_with('…'));
    assert_eq!(view.turns()[0].digest.commands_failed, 2);

    let mut view = SessionView::new();
    apply(
        &mut view,
        &[
            start("m"),
            json!({"type":"item_completed","id":"m","status":"completed",
                   "detail":{"type":"command","tool":"Bash","command":"cd x\n\nmake","output":""}}),
        ],
    );
    assert_eq!(view.commands()[0].headline, "cd x …");
}

#[test]
fn turns_count_the_steps_they_fold() {
    let mut view = SessionView::new();
    let item =
        |id: &str, kind: &str| json!({"type":"item_started","id":id,"kind":kind,"detail":{"type":"text"}});
    apply(&mut view, &[item("u", "user_message"), item("r", "reasoning")]);
    assert_eq!(view.turns()[0].steps, 0, "reasoning counts once it has text");
    apply(
        &mut view,
        &[json!({"type":"item_delta","id":"r","text":"Look first."})],
    );
    assert_eq!(view.turns()[0].steps, 1);
    apply(&mut view, &[item("a1", "agent_message")]);
    assert_eq!(view.turns()[0].steps, 1, "the latest message is the narrative");
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"c","kind":"command_execution","detail":{"type":"command","command":"ls"}}),
            item("a2", "agent_message"),
        ],
    );
    let t = &view.turns()[0];
    assert_eq!((t.steps, t.narrative_item.as_deref()), (3, Some("a2")));
}

#[test]
fn reasoning_without_text_shows_nothing() {
    let mut view = SessionView::new();
    let start =
        |id: &str, kind: &str| json!({"type":"item_started","id":id,"kind":kind,"detail":{"type":"text"}});
    let done =
        |id: &str| json!({"type":"item_completed","id":id,"status":"completed","detail":{"type":"text"}});
    apply(
        &mut view,
        &[
            start("u", "user_message"),
            // Codex with summaries off: no text, ever.
            start("r1", "reasoning"),
            done("r1"),
            // Text arrives after a later item started: still in order.
            start("r2", "reasoning"),
            json!({"type":"item_started","id":"c","kind":"command_execution","detail":{"type":"command","command":"ls"}}),
            json!({"type":"item_delta","id":"r2","text":"  "}),
            json!({"type":"item_delta","id":"r2","text":"Check the tests."}),
            done("r2"),
        ],
    );
    assert!(view.item_spec("r1").is_none());
    let t = &view.turns()[0];
    assert_eq!(t.items, ["u", "r2", "c"]);
    assert_eq!(t.steps, 2);
    let spec = view.item_spec("r2").unwrap().spec.unwrap();
    assert_eq!(spec["nodes"][0]["summary"], "Thought");
    // Running and still empty: Now says thinking, but there is no spec.
    apply(&mut view, &[start("r3", "reasoning")]);
    assert!(view.item_spec("r3").is_none());
    let now = view.now();
    assert_eq!(
        now.last().map(|a| (a.item_id.as_str(), a.kind)),
        Some(("r3", ActivityKind::Thinking))
    );
}

#[test]
fn ask_options_say_what_they_do() {
    let mut view = SessionView::new();
    apply(
        &mut view,
        &[
            json!({"type":"permission_requested","request_id":"r","title":"Bash",
                 "options":[{"id":"a","label":"Allow","kind":"allow_once"},
                            {"id":"aa","label":"Always","kind":"allow_always"},
                            {"id":"d","label":"Deny","kind":"reject_once"},
                            {"id":"x","label":"Odd","kind":"something_new"},
                            {"id":"n","label":"None"}]}),
        ],
    );
    let kinds: Vec<_> = view.asks()[0].options.iter().map(|o| o.kind).collect();
    assert_eq!(
        kinds,
        [
            Some(OptionKind::AllowOnce),
            Some(OptionKind::AllowAlways),
            Some(OptionKind::RejectOnce),
            Some(OptionKind::Other),
            None
        ]
    );
    assert!(OptionKind::AllowAlways.allows() && OptionKind::AllowAlways.persists());
    assert!(OptionKind::RejectOnce.rejects() && !OptionKind::RejectOnce.persists());
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
            (ActivityKind::Testing, "cargo test".into()),
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

#[test]
fn questions_are_answered_through_a_draft() {
    let mut view = SessionView::new();
    apply(
        &mut view,
        &[
            json!({"type":"permission_requested","request_id":"q","title":"Which DB?","kind":"question",
                 "item_id":"toolu_1",
                 "options":[{"id":"Postgres","label":"Postgres"},{"id":"SQLite","label":"SQLite"}],
                 "native":{"questions":[
                     {"question":"Which DB?","header":"DB","multiSelect":false,
                      "options":[{"label":"Postgres","description":"server"},{"label":"SQLite","description":"file"}]},
                     {"question":"Extras?","header":"Extras","multiSelect":true,
                      "options":[{"label":"Cache"},{"label":"Search"}]}]}}),
        ],
    );
    let ask = view.ask("q").unwrap();
    assert_eq!(ask.questions.len(), 2);
    assert_eq!(ask.questions[0].options[1].description.as_deref(), Some("file"));
    assert_eq!(ask.plan, None);

    let mut draft = AnswerDraft::new(ask);
    assert!(draft.pick_label(0, "SQLite"));
    assert!(draft.pick_label(1, "Search"));
    assert!(draft.set_text(1, "Queue"));
    let option_id = draft.option_id().unwrap();
    assert_eq!(option_id, r#"[["SQLite"],["Search","Queue"]]"#);
    assert_eq!(
        option_id,
        encode_answers(&[vec!["SQLite".into()], vec!["Search".into(), "Queue".into()]])
    );

    assert!(view.answered(draft.request_id()).asks);
    assert!(view.asks().is_empty());
}

#[test]
fn plan_approvals_carry_the_plan() {
    let mut view = SessionView::new();
    apply(
        &mut view,
        &[
            json!({"type":"permission_requested","request_id":"p","title":"ExitPlanMode",
                 "options":[{"id":"allow","label":"Allow","kind":"allow_once"},
                            {"id":"deny","label":"Deny","kind":"reject_once"}],
                 "item_id":"toolu_2",
                 "detail":{"type":"tool","tool":"ExitPlanMode","title":""},
                 "native":{"plan":"## Plan\n1. Add caching"}}),
        ],
    );
    let ask = view.ask("p").unwrap();
    assert_eq!(ask.plan.as_deref(), Some("## Plan\n1. Add caching"));
    assert!(ask.questions.is_empty());
    assert!(!AnswerDraft::new(ask).is_complete());
}

#[test]
fn restored_asks_keep_their_questions() {
    let snapshot = json!({
        "transcript": {"items": [
            {"id":"u","kind":"user_message","status":"completed","text":"Go","detail":{"type":"text"},"seq":0}
        ]},
        "permissions": [{"request_id":"q","title":"Which DB?","options":[],"kind":"question","item_id":null,
            "detail":null,"native":{"questions":[{"question":"Which DB?","options":[{"label":"SQLite"}]}]}}]
    });
    let view = SessionView::from_transcript(&snapshot.to_string()).unwrap();
    assert_eq!(view.asks()[0].questions[0].options[0].label, "SQLite");
}

#[test]
fn recognised_commands_feed_outcomes_files_and_digests() {
    use acc_ui::session::{CommandKind, TestCounts};
    let mut view = SessionView::new();
    view.set_root("/w");
    let start = |id: &str, command: &str| {
        json!({"type":"item_started","id":id,"kind":"command_execution",
               "detail":{"type":"command","tool":"Bash","command":command,"output":""}})
    };
    let done = |id: &str, command: &str, output: &str, exit: Value| {
        json!({"type":"item_completed","id":id,"status":"completed",
               "detail":{"type":"command","tool":"Bash","command":command,"output":output,"exit_code":exit}})
    };
    let piped = "cargo test 2>&1 | tail -3";
    let failing = "test result: FAILED. 3 passed; 1 failed; 0 ignored\nerror: test failed";
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"u","kind":"user_message","detail":{"type":"text"}}),
            start("t1", piped),
        ],
    );
    assert_eq!(view.now()[0].kind, ActivityKind::Testing);
    apply(
        &mut view,
        &[
            // `tail` exited 0; the tests failed.
            done("t1", piped, failing, json!(0)),
            start("r", "cat -n src/a.rs src/b.rs"),
        ],
    );
    assert_eq!(view.now()[0].kind, ActivityKind::Reading);
    assert_eq!(view.now()[0].target, "src/a.rs, src/b.rs");
    let read = done("r", "cat -n src/a.rs src/b.rs", "1\tfn a() {}", Value::Null);
    apply(
        &mut view,
        &[
            read.clone(),
            read,
            start("t2", "pytest -q"),
            // Claude reports no exit code; the summary says it passed.
            done("t2", "pytest -q", "....\n4 passed in 0.02s", Value::Null),
            start("x", "python3 build.py && cat out.txt"),
            done("x", "python3 build.py && cat out.txt", "", Value::Null),
        ],
    );
    let t1 = view.command("t1").unwrap();
    assert_eq!(
        (t1.kind, t1.outcome, t1.outcome_inferred),
        (CommandKind::Test, CommandOutcome::Failed, true)
    );
    assert_eq!(t1.tool.as_deref(), Some("cargo test"));
    assert_eq!(
        t1.tests,
        Some(TestCounts {
            passed: 3,
            failed: 1,
            skipped: 0
        })
    );
    let t2 = view.command("t2").unwrap();
    assert_eq!(
        (t2.outcome, t2.outcome_inferred),
        (CommandOutcome::Succeeded, true)
    );
    let r = view.command("r").unwrap();
    assert_eq!((r.kind, r.outcome), (CommandKind::Read, CommandOutcome::Unknown));
    assert_eq!(r.reads, ["/w/src/a.rs", "/w/src/b.rs"]);
    // Completed twice, read once.
    assert_eq!(view.file("/w/src/a.rs").unwrap().reads, 1);
    assert_eq!(view.file("/w/src/a.rs").unwrap().display, "src/a.rs");
    let x = view.command("x").unwrap();
    assert_eq!((x.kind, x.reads.len()), (CommandKind::Other, 0));
    assert!(view.file("/w/out.txt").is_none());

    let d = &view.turns()[0].digest;
    assert_eq!((d.commands, d.commands_failed, d.files_read), (4, 1, 2));
    // The last test run's counts.
    assert_eq!(
        d.tests,
        Some(TestCounts {
            passed: 4,
            failed: 0,
            skipped: 0
        })
    );
    assert_eq!(
        d.summary(),
        "Read 2 files · ran 4 commands, 1 failed · 4 tests passed"
    );
}

#[test]
fn completed_simple_claude_commands_succeeded() {
    let output = |c: &str| {
        if c.starts_with("pytest") {
            "1 failed, 2 passed in 0.1s"
        } else {
            "x"
        }
    };
    let run = |harness: &str, command: &str, status: &str| {
        let mut view = SessionView::new();
        apply(
            &mut view,
            &[
                json!({"type":"session_started","harness":harness,"native_id":"n"}),
                json!({"type":"item_started","id":"c","kind":"command_execution",
                       "detail":{"type":"command","tool":"Bash","command":command,"output":""}}),
                json!({"type":"item_completed","id":"c","status":status,
                       "detail":{"type":"command","tool":"Bash","command":command,"output":output(command),"exit_code":null}}),
            ],
        );
        let c = view.command("c").unwrap();
        (c.outcome, c.outcome_inferred)
    };
    assert_eq!(
        run("claude", "ls -la", "completed"),
        (CommandOutcome::Succeeded, true)
    );
    assert_eq!(
        run("claude", "make deploy 2>&1", "completed"),
        (CommandOutcome::Succeeded, true)
    );
    assert_eq!(
        run("claude", "ls | head", "completed"),
        (CommandOutcome::Unknown, false)
    );
    assert_eq!(
        run("claude", "./x.sh || true", "completed"),
        (CommandOutcome::Unknown, false)
    );
    assert_eq!(run("claude", "ls", "failed"), (CommandOutcome::Failed, false));
    assert_eq!(
        run("opencode", "ls", "completed"),
        (CommandOutcome::Unknown, false)
    );
    // What the output says still wins.
    assert_eq!(
        run("claude", "pytest -q", "completed"),
        (CommandOutcome::Failed, true)
    );
}

const SHA: &str = "f1dd04ee84b8e5da1c5e3ed2b8d9b0e4e01e5d5bd9da1e3e0b8b6f0f1a3e7c2d";

#[test]
fn prompt_images_come_from_acc_refs() {
    let mut view = SessionView::new();
    let image = json!({"media_type": "image/png", "bytes": 3, "sha256": SHA});
    apply(
        &mut view,
        &[
            // An image-only prompt: ACC's `[image]` stand-in is dropped.
            json!({"type":"item_started","id":"u1","kind":"user_message","detail":{"type":"text"},
                "native":{"images":[image, {"bogus": true}, "not a ref"]}}),
            json!({"type":"item_delta","id":"u1","text":"[image]"}),
            json!({"type":"item_completed","id":"u1","status":"completed","detail":{"type":"text"}}),
        ],
    );
    let t = &view.turns()[0];
    assert_eq!(t.prompt.as_deref(), Some(""));
    assert_eq!(t.prompt_images.len(), 1, "malformed refs are left out");
    assert_eq!(t.prompt_images[0].sha256.as_deref(), Some(SHA));
    assert_eq!(t.prompt_images[0].src(), Some(format!("sha256:{SHA}")));
    assert_eq!(view.header().title, None);
    let item = view.item("u1").unwrap();
    assert_eq!(item.text, "");
    let spec = view.item_spec("u1").unwrap();
    assert!(spec.errors.is_empty(), "{:?}", spec.errors);
    let nodes = spec.spec.unwrap()["nodes"].to_string();
    assert!(nodes.contains(&format!("\"src\":\"sha256:{SHA}\"")), "{nodes}");

    // Text with an image keeps the text; Codex's own native keys are fine.
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"u2","kind":"user_message","detail":{"type":"text"},
                "native":{"type":"userMessage","images":[{"path":"/w/shot.png"}]}}),
            json!({"type":"item_delta","id":"u2","text":"what is this?"}),
        ],
    );
    let t = &view.turns()[1];
    assert_eq!(t.prompt.as_deref(), Some("what is this?"));
    assert_eq!(t.prompt_images[0].path.as_deref(), Some("/w/shot.png"));

    // Older ACC without refs: the stand-in stays the prompt.
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"u3","kind":"user_message","detail":{"type":"text"}}),
            json!({"type":"item_delta","id":"u3","text":"[image]"}),
        ],
    );
    let t = &view.turns()[2];
    assert_eq!((t.prompt.as_deref(), t.prompt_images.len()), (Some("[image]"), 0));
}

#[test]
fn codex_image_tools_carry_their_image() {
    let mut view = SessionView::new();
    view.set_root("/w");
    apply(
        &mut view,
        &[
            json!({"type":"item_started","id":"u","kind":"user_message","detail":{"type":"text"}}),
            json!({"type":"item_started","id":"iv","kind":"tool_call","detail":{"type":"tool","tool":"imageView",
                "kind":"other","title":"/w/a.png","input":{"path":"/w/a.png"},"paths":["/w/a.png"]}}),
            json!({"type":"item_completed","id":"iv","status":"completed","detail":{"type":"tool","tool":"imageView",
                "kind":"other","title":"/w/a.png","input":{"path":"/w/a.png"},"paths":["/w/a.png"]}}),
            json!({"type":"item_started","id":"ig","kind":"tool_call","detail":{"type":"tool","tool":"imageGeneration",
                "kind":"other","title":"a cat","input":{"revisedPrompt":"a cat","savedPath":"/w/cat.png"}}}),
        ],
    );
    assert_eq!(
        view.item("ig").unwrap().images[0].path.as_deref(),
        Some("/w/cat.png")
    );
    let iv = view.item("iv").unwrap();
    assert_eq!(iv.images[0].path.as_deref(), Some("/w/a.png"));
    assert_eq!(view.files()[0].display, "a.png");
    assert_eq!(view.files()[0].reads, 1);
    let spec = view.item_spec("iv").unwrap();
    let nodes = spec.spec.unwrap()["nodes"].to_string();
    assert!(nodes.contains(r#""src":"/w/a.png""#), "{nodes}");
    assert!(nodes.contains(r#""alt":"a.png""#), "{nodes}");
}

#[test]
fn image_src_takes_a_content_hash() {
    let spec = |src: &str| json!({"root": {"type": "image", "src": src}});
    assert!(
        acc_ui::normalize_value(spec(&format!("sha256:{SHA}")))
            .errors
            .is_empty()
    );
    assert!(!acc_ui::normalize_value(spec("sha256:abc")).errors.is_empty());
    assert!(
        !acc_ui::normalize_value(spec(&format!("sha256:{}", "g".repeat(64))))
            .errors
            .is_empty()
    );
}
