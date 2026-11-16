//! The generated texts, checked against `tests/snapshots`. A change here
//! changes what agents read: review the diff.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

#[test]
fn schema() {
    let s = serde_json::to_string_pretty(&acc_ui::schema()).unwrap() + "\n";
    common::golden(&common::dir("snapshots").join("schema.json"), &s);
}

#[test]
fn conventions() {
    let s = acc_ui::conventions();
    common::golden(
        &common::dir("snapshots").join("conventions.txt"),
        &(s.clone() + "\n"),
    );
    // It goes into every turn; keep it short.
    let words = s.split_whitespace().count();
    assert!(words < 80, "{words} words");
}

#[test]
fn conventions_skill() {
    common::golden(
        &common::dir("snapshots").join("SKILL.md"),
        &acc_ui::conventions_skill(),
    );
}

/// The skill's chart and mermaid examples render.
#[test]
fn conventions_skill_examples_render() {
    let skill = acc_ui::conventions_skill();
    let mut n = 0;
    for block in skill.split("\n```").skip(1).step_by(2) {
        let (info, body) = block.split_once('\n').unwrap();
        if acc_ui::is_component_fence(info) {
            let out = acc_ui::parse_fence(info, body).unwrap();
            assert!(
                out.spec.is_some() && out.errors.is_empty(),
                "{info}: {:?}",
                out.errors
            );
            n += 1;
        }
    }
    assert_eq!(n, 2);
}

/// The schema's node types match `Node::TYPES`, which normalize checks,
/// less the ones only acc-ui makes.
#[test]
fn schema_node_types_match() {
    let schema = acc_ui::schema();
    let types: Vec<&str> = schema["$defs"]["Node"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["properties"]["type"]["const"].as_str().unwrap())
        .collect();
    let agent_types: Vec<&str> = acc_ui::spec::Node::TYPES
        .iter()
        .filter(|t| !acc_ui::spec::Node::INTERNAL_TYPES.contains(t))
        .copied()
        .collect();
    assert_eq!(types, agent_types);
    let s = schema.to_string();
    assert!(
        !s.contains("\"answer\"") && !s.contains("request_id"),
        "answer/request leaked into the schema"
    );
}
