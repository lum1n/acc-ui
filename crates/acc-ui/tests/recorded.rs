//! Features checked against recorded ACC sessions, one fixture each
//! (`tests/sessions/<name>.ndjson`, recorded with
//! `scripts/record-fixture.py`): questions and plan approvals answered
//! through ACC, test runners and type checkers, image prompts, and ACP
//! harnesses.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use acc_ui::SessionView;
use acc_ui::session::event::{AskKind, Detail, FileChangeKind, ItemKind, OptionKind};
use acc_ui::session::{
    AnswerDraft, Ask, CommandEntry, CommandKind, CommandOutcome, Diagnostics, Event, TestCounts, TurnOutcome,
    encode_answers,
};

/// The session `name` replayed with `/work` as its root, and every ask it
/// raised as it was raised.
fn replay(name: &str) -> (SessionView, Vec<Ask>) {
    let mut view = SessionView::new();
    view.set_root("/work");
    let mut asks = Vec::new();
    let path = common::dir("sessions").join(format!("{name}.ndjson"));
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let event: Event = serde_json::from_str(line).unwrap();
        let ask = matches!(event, Event::PermissionRequested { .. });
        view.apply(event);
        if ask {
            asks.push(view.asks().last().unwrap().clone());
        }
    }
    // Every recorded session ends with its last turn done.
    assert!(view.asks().is_empty(), "{name}: asks left");
    assert!(view.now().is_empty(), "{name}: still running");
    assert!(view.turns().iter().all(|t| !t.open), "{name}");
    (view, asks)
}

/// The commands that ran `tool`, in order.
fn runs<'a>(view: &'a SessionView, tool: &str) -> Vec<&'a CommandEntry> {
    view.commands()
        .iter()
        .filter(|c| c.tool.as_deref() == Some(tool))
        .collect()
}

fn tests(passed: u32, failed: u32) -> Option<TestCounts> {
    Some(TestCounts {
        passed,
        failed,
        skipped: 0,
    })
}

fn diags(errors: u32, warnings: u32) -> Option<Diagnostics> {
    Some(Diagnostics { errors, warnings })
}

fn tool_output(view: &SessionView, id: &str) -> String {
    match &view.item(id).unwrap().detail {
        Detail::Tool { output, .. } => output.clone().unwrap_or_default(),
        other => panic!("{other:?}"),
    }
}

// Questions and plan approvals

/// Claude's `AskUserQuestion` with three questions, one multi-select, the
/// last answered with typed text, answered through ACC with the option id
/// an `AnswerDraft` makes.
#[test]
fn claude_ask_user_question() {
    let (view, asks) = replay("claude-ask-questions");
    let [ask, write] = &asks[..] else {
        panic!("{asks:?}")
    };
    assert_eq!(ask.kind, AskKind::Question);
    assert_eq!(ask.title, "What is your favourite colour?");
    let item = ask.item_id.as_deref().unwrap();
    assert_eq!(view.item(item).unwrap().kind, ItemKind::ToolCall);

    let q = &ask.questions;
    assert_eq!(q.len(), 3);
    let labels = |i: usize| q[i].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>();
    assert_eq!(labels(0), ["Red", "Green", "Blue"]);
    assert_eq!(labels(1), ["Cheese", "Ham", "Pineapple", "Olives"]);
    assert_eq!(q[1].header.as_deref(), Some("Toppings"));
    assert_eq!(
        q.iter().map(|q| q.multi_select).collect::<Vec<_>>(),
        [false, true, false]
    );
    // Claude keys answers by question text and always takes typed text.
    assert!(q.iter().all(|q| q.id.is_none() && q.allows_text && q.required));
    assert_eq!(q[2].options.len(), 2, "two suggested names");

    let mut draft = AnswerDraft::new(ask);
    assert!(draft.pick_label(0, "Blue"));
    assert!(draft.pick_label(1, "Cheese"));
    assert!(draft.pick_label(1, "Olives"));
    assert!(!draft.is_complete());
    assert!(draft.set_text(2, "Zephyr"));
    // What the recording sent; Claude wrote these answers to answers.txt.
    let sent = r#"[["Blue"],["Cheese","Olives"],["Zephyr"]]"#;
    assert_eq!(draft.option_id().as_deref(), Some(sent));
    assert_eq!(
        sent,
        encode_answers(&[
            vec!["Blue".into()],
            vec!["Cheese".into(), "Olives".into()],
            vec!["Zephyr".into()]
        ])
    );
    let answers = view.file("/work/answers.txt").unwrap();
    assert_eq!((answers.change, answers.added), (Some(FileChangeKind::Add), 3));

    assert_eq!(write.kind, AskKind::Permission);
    assert!(write.questions.is_empty() && write.plan.is_none());
}

/// Claude in plan mode asks to leave it with `ExitPlanMode`; the approval
/// carries the plan and, approved, Claude makes the change.
#[test]
fn claude_plan_approval() {
    let (view, asks) = replay("claude-plan-approval");
    let plan_ask = asks.iter().find(|a| a.plan.is_some()).unwrap();
    assert_eq!(plan_ask.title, "ExitPlanMode");
    assert_eq!(plan_ask.kind, AskKind::Permission);
    assert!(plan_ask.questions.is_empty());
    let plan = plan_ask.plan.as_deref().unwrap();
    assert!(plan.starts_with("# Plan") && plan.contains("## Steps"), "{plan}");
    let kinds: Vec<_> = plan_ask.options.iter().map(|o| o.kind).collect();
    assert_eq!(kinds, [Some(OptionKind::AllowOnce), Some(OptionKind::RejectOnce)]);
    assert!(!AnswerDraft::new(plan_ask).is_complete(), "nothing to draft");
    let tool = plan_ask.item_id.as_deref().unwrap();
    assert!(tool_output(&view, tool).starts_with("User has approved your plan"));
    // The only plan approval: the others are tool permissions.
    assert_eq!(asks.iter().filter(|a| a.plan.is_some()).count(), 1);

    let greet = view.file("/work/greet.py").unwrap();
    assert_eq!(greet.change, Some(FileChangeKind::Update));
    let t = &view.turns()[0];
    assert_eq!(t.outcome, TurnOutcome::Done);
}

/// Codex's `requestUserInput` in plan mode: one question with an id and a
/// header that also takes typed text.
#[test]
fn codex_request_user_input() {
    let (view, asks) = replay("codex-request-user-input");
    let [ask] = &asks[..] else { panic!("{asks:?}") };
    assert_eq!(ask.kind, AskKind::Question);
    let [q] = &ask.questions[..] else { panic!() };
    assert_eq!(q.id.as_deref(), Some("language"));
    assert_eq!(q.header.as_deref(), Some("Language"));
    assert_eq!(q.text, "Which language should the hello-world script use?");
    assert!(q.allows_text && !q.multi_select && !q.secret);
    assert_eq!(
        q.options[0].description.as_deref(),
        Some("Use a Python script that prints Hello, world!.")
    );
    let mut draft = AnswerDraft::new(ask);
    assert!(draft.pick_label(0, "Bash"));
    assert_eq!(draft.option_id().as_deref(), Some(r#"[["Bash"]]"#));
    // Codex shows no item for the call the question belongs to.
    assert!(view.item(ask.item_id.as_deref().unwrap()).is_none());

    let t = &view.turns()[0];
    assert!(t.narrative.as_deref().unwrap().contains("Bash"));
    let rg = &view.commands()[0];
    assert_eq!(rg.display, "rg --files -g '!**/.git/**'");
    assert_eq!(rg.kind, CommandKind::List);
}

// Test runners and type checkers

/// Codex runs `cargo test` (one failure) and `cargo clippy` (two
/// warnings), fixes both and runs them again. Codex reports exit codes.
#[test]
fn codex_cargo_test_and_clippy() {
    let (view, _) = replay("codex-cargo-test-clippy");
    let test = runs(&view, "cargo test");
    let clippy = runs(&view, "cargo clippy");
    assert_eq!((test.len(), clippy.len()), (2, 2));

    let (t, c) = (test[0], clippy[0]);
    assert_eq!((t.kind, t.headline.as_str()), (CommandKind::Test, "cargo test"));
    assert_eq!(
        (t.exit_code, t.outcome, t.outcome_inferred),
        (Some(101), CommandOutcome::Failed, false)
    );
    assert_eq!(t.tests, tests(2, 1));
    assert_eq!(t.diagnostics, diags(0, 0), "it compiled");
    assert_eq!(c.kind, CommandKind::Lint);
    assert_eq!((c.exit_code, c.outcome), (Some(0), CommandOutcome::Succeeded));
    assert_eq!(c.diagnostics, diags(0, 2));

    let (t, c) = (test[1], clippy[1]);
    assert_eq!(t.outcome, CommandOutcome::Succeeded);
    // The doc-test summary's `0 passed` adds nothing.
    assert_eq!(t.tests, tests(3, 0));
    assert_eq!(c.diagnostics, diags(0, 0));

    let d = &view.turns()[0].digest;
    assert_eq!((d.commands_failed, d.tests), (1, tests(3, 0)));
    assert!(d.summary().ends_with("3 tests passed"), "{}", d.summary());
}

/// Claude runs vitest (one failure) and `tsc --noEmit` (one error), fixes
/// both and runs them again. Claude reports no exit codes: failed calls
/// are failed items, the rest is read from the output.
#[test]
fn claude_vitest_and_tsc() {
    let (view, _) = replay("claude-vitest-tsc");
    let vitest = runs(&view, "vitest");
    let tsc = runs(&view, "tsc");
    assert_eq!((vitest.len(), tsc.len()), (2, 2));

    let (v, t) = (vitest[0], tsc[0]);
    assert_eq!((v.kind, v.exit_code), (CommandKind::Test, None));
    assert_eq!(v.outcome, CommandOutcome::Failed);
    assert_eq!(v.tests, tests(2, 1));
    assert_eq!((t.kind, t.outcome), (CommandKind::Build, CommandOutcome::Failed));
    assert_eq!(t.diagnostics, diags(1, 0));

    let (v, t) = (vitest[1], tsc[1]);
    assert_eq!(
        (v.outcome, v.outcome_inferred, v.tests),
        (CommandOutcome::Succeeded, true, tests(3, 0))
    );
    // `(Bash completed with no output)`: a silent tsc found nothing.
    assert_eq!(t.output_tail, "(Bash completed with no output)");
    assert_eq!(
        (t.outcome, t.diagnostics),
        (CommandOutcome::Succeeded, diags(0, 0))
    );

    let d = &view.turns()[0].digest;
    assert_eq!((d.commands, d.commands_failed, d.tests), (4, 2, tests(3, 0)));
    let cart = view.file("/work/src/cart.ts").unwrap();
    assert_eq!(
        (cart.display.as_str(), cart.change),
        ("src/cart.ts", Some(FileChangeKind::Update))
    );
}

// Images

/// An image sent with the prompt through ACC comes back as a ref on the
/// prompt, under Claude and Codex alike.
#[test]
fn image_prompts_carry_their_ref() {
    const SHA: &str = "cc4ecc7934027a14a5581e18633ed8237dd2d9782456ca0aa1965aed56b7b12e";
    for name in ["claude-image-prompt", "codex-image-prompt"] {
        let (view, _) = replay(name);
        let t = &view.turns()[0];
        assert!(
            t.prompt.as_deref().unwrap().starts_with("Which two colours"),
            "{name}"
        );
        let [image] = &t.prompt_images[..] else {
            panic!("{name}: {t:?}")
        };
        assert_eq!(image.media_type.as_deref(), Some("image/png"), "{name}");
        assert_eq!(
            (image.bytes, image.sha256.as_deref()),
            (Some(145), Some(SHA)),
            "{name}"
        );
        assert_eq!(image.src(), Some(format!("sha256:{SHA}")), "{name}");

        let prompt = view
            .items()
            .iter()
            .find(|i| i.kind == ItemKind::UserMessage)
            .unwrap();
        let spec = view.item_spec(&prompt.id).unwrap();
        assert!(spec.errors.is_empty(), "{name}: {:?}", spec.errors);
        let nodes = spec.spec.unwrap()["nodes"].to_string();
        assert!(
            nodes.contains(&format!("\"src\":\"sha256:{SHA}\"")),
            "{name}: {nodes}"
        );
        // The image is half red, half blue.
        let answer = t.narrative.as_deref().unwrap().to_lowercase();
        assert!(
            answer.contains("red") && answer.contains("blue"),
            "{name}: {answer}"
        );
    }
}

// ACP harnesses

/// Cursor over ACP fixes a bug and runs the script. ACP agents don't echo
/// the prompt, so the turn has none; Cursor's to-do list arrives only as
/// `cursor/update_todos` native events, so there is no plan.
#[test]
fn cursor_edit_and_run() {
    let (view, asks) = replay("cursor-edit-and-run");
    let h = view.header();
    assert_eq!(h.harness.as_deref(), Some("cursor"));
    assert_eq!(h.title.as_deref(), Some("Todo And Bugfix"));
    assert!(asks.is_empty(), "Cursor ran without asking");
    let [t] = view.turns() else { panic!() };
    assert_eq!((t.prompt.as_deref(), t.outcome), (None, TurnOutcome::Done));
    assert!(view.plan().is_none());

    let calc = view.file("/work/calc.py").unwrap();
    assert_eq!(
        (calc.change, calc.added, calc.removed),
        (Some(FileChangeKind::Update), 1, 1)
    );
    assert_eq!(calc.reads, 1);
    let [run] = view.commands() else {
        panic!("{:?}", view.commands())
    };
    assert_eq!(run.display, "python3 calc.py");
    assert_eq!((run.exit_code, run.outcome), (Some(0), CommandOutcome::Succeeded));
    assert!(
        t.digest
            .summary()
            .starts_with("Edited 1 file (+1 −1) · ran 1 command"),
        "{}",
        t.digest.summary()
    );
}

/// GitHub Copilot over ACP asks before editing and running; its command
/// output ends with its own exit marker instead of an exit code.
#[test]
fn copilot_edit_and_run() {
    let (view, asks) = replay("copilot-edit-and-run");
    let h = view.header();
    assert_eq!(h.harness.as_deref(), Some("copilot"));
    // Copilot titles the session with the whole prompt.
    let title = h.title.as_deref().unwrap();
    assert!(
        title.starts_with("First write") && title.ends_with('…'),
        "{title}"
    );
    assert!(title.chars().count() <= 81);

    let [edit, run] = &asks[..] else {
        panic!("{asks:?}")
    };
    for a in [edit, run] {
        let kinds: Vec<_> = a.options.iter().map(|o| o.kind).collect();
        assert_eq!(
            kinds,
            [
                Some(OptionKind::AllowOnce),
                Some(OptionKind::AllowAlways),
                Some(OptionKind::RejectOnce)
            ]
        );
    }
    assert_eq!(edit.title, "Update file");
    assert!(matches!(run.subject, Some(Detail::Command { .. })), "{run:?}");

    let [t] = view.turns() else { panic!() };
    assert_eq!(t.prompt, None);
    let [cmd] = view.commands() else { panic!() };
    assert_eq!(cmd.exit_code, None);
    assert!(
        cmd.output_tail
            .ends_with("<shellId: 0 completed with exit code 0>")
    );
    assert_eq!(cmd.outcome, CommandOutcome::Unknown);
    assert_eq!(
        view.file("/work/calc.py").unwrap().change,
        Some(FileChangeKind::Update)
    );
}
