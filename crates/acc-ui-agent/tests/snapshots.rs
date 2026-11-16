//! The generated texts, checked against `tests/snapshots`. A change here
//! changes what agents read: review the diff.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

#[test]
fn skill() {
    common::golden(
        &common::dir("snapshots").join("SKILL.md"),
        &acc_ui_agent::skill_markdown(),
    );
}

#[test]
fn instructions() {
    let s = acc_ui_agent::instructions();
    common::golden(
        &common::dir("snapshots").join("instructions.txt"),
        &(s.clone() + "\n"),
    );
    // About 150 tokens; keep it short, it goes into every session.
    assert!(
        s.split_whitespace().count() < 130,
        "{} words",
        s.split_whitespace().count()
    );
}

#[test]
fn tools_list() {
    let s = serde_json::to_string_pretty(&acc_ui_agent::host::tools_list()).unwrap() + "\n";
    common::golden(&common::dir("snapshots").join("tools.json"), &s);
}

#[test]
fn server_script() {
    common::golden(
        &common::dir("snapshots").join("acc-ui-mcp.sh"),
        &acc_ui_agent::server_script(),
    );
}

/// Every node type agents may write, and every field kind, appears in the
/// skill's catalog; the ones only acc-ui makes don't.
#[test]
fn skill_lists_every_component() {
    let skill = acc_ui_agent::skill_markdown();
    for ty in acc_ui::spec::Node::TYPES {
        let internal = acc_ui::spec::Node::INTERNAL_TYPES.contains(ty);
        assert_eq!(
            skill.contains(&format!("**`{ty}`**")),
            !internal,
            "{ty} in the skill: expected {}",
            !internal
        );
    }
    for kind in ["text", "textarea", "number", "toggle", "select", "date", "slider"] {
        assert!(skill.contains(&format!("`{kind}`")), "{kind} missing");
    }
}

/// The specs in the skill's examples are valid.
#[test]
fn skill_examples_are_valid() {
    let skill = acc_ui_agent::skill_markdown();
    let mut n = 0;
    for block in skill.split("```json\n").skip(1) {
        let body = block.split("\n```").next().unwrap();
        let out = acc_ui::normalize(body);
        assert!(
            out.spec.is_some() && out.errors.is_empty(),
            "{body}\n{:?}",
            out.errors
        );
        n += 1;
    }
    assert_eq!(n, 3);
}
