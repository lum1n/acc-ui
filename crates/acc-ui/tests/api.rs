//! Limits, ids and fences.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use acc_ui::normalize::{
    MAX_CHART_POINTS, MAX_DEPTH, MAX_HTML_BYTES, MAX_NODES, MAX_SPEC_BYTES, MAX_TABLE_ROWS,
};
use acc_ui::{is_component_fence, normalize, normalize_value, parse_fence};
use serde_json::{Value, json};

fn nodes(v: &Value) -> &Vec<Value> {
    v["nodes"].as_array().unwrap()
}

// Limits

#[test]
fn oversized_spec_is_refused() {
    let big = "x".repeat(MAX_SPEC_BYTES);
    let spec = json!({ "root": { "type": "markdown", "text": big } }).to_string();
    let out = normalize(&spec);
    assert!(out.spec.is_none());
    assert!(out.errors[0].contains("limit"), "{:?}", out.errors);
}

#[test]
fn deep_nesting_is_cut() {
    let mut node = json!({ "type": "badge", "text": "deep" });
    for _ in 0..MAX_DEPTH + 5 {
        node = json!({ "type": "stack", "children": [node] });
    }
    let out = normalize_value(json!({ "root": node }));
    let spec = out.spec.unwrap();
    assert_eq!(nodes(&spec).len(), MAX_DEPTH + 1);
    assert_eq!(out.errors.len(), 1);
    assert!(out.errors[0].contains("nested deeper"));
}

#[test]
fn node_count_is_capped() {
    let children: Vec<Value> = (0..MAX_NODES + 50)
        .map(|i| json!({ "type": "badge", "text": i.to_string() }))
        .collect();
    let out = normalize_value(json!({ "root": { "type": "stack", "children": children } }));
    assert_eq!(nodes(&out.spec.unwrap()).len(), MAX_NODES);
    assert_eq!(out.errors.len(), 1, "one error, not one per node");
}

#[test]
fn table_rows_and_chart_points_are_capped() {
    let rows: Vec<Value> = (0..MAX_TABLE_ROWS + 10).map(|i| json!([i])).collect();
    let points: Vec<Value> = (0..MAX_CHART_POINTS + 10).map(|i| json!([i, i])).collect();
    let out = normalize_value(json!({ "root": { "type": "stack", "children": [
        { "type": "table", "columns": ["n"], "rows": rows },
        { "type": "chart", "kind": "line", "series": [{ "points": points }] }
    ] } }));
    let spec = out.spec.unwrap();
    assert_eq!(nodes(&spec)[1]["rows"].as_array().unwrap().len(), MAX_TABLE_ROWS);
    assert_eq!(
        nodes(&spec)[2]["series"][0]["points"].as_array().unwrap().len(),
        MAX_CHART_POINTS
    );
    assert_eq!(out.errors.len(), 2);
}

#[test]
fn oversized_html_is_dropped() {
    let out = normalize_value(json!({ "root": { "type": "stack", "children": [
        { "type": "html", "html": "x".repeat(MAX_HTML_BYTES + 1) },
        { "type": "badge", "text": "kept" }
    ] } }));
    assert_eq!(nodes(&out.spec.unwrap()).len(), 2);
    assert!(out.errors[0].contains("html is"));
}

#[test]
fn ids_are_kept_or_derived_from_content() {
    let a = normalize(r#"{"id":"run-1.a_b","root":{"type":"badge","text":"x"}}"#);
    assert_eq!(a.id(), Some("run-1.a_b"));
    let b = normalize(r#"{"root":{"type":"badge","text":"x"}}"#);
    let c = normalize(r#"{"root":{"type":"badge","text":"x"}}"#);
    assert_eq!(b.id(), c.id(), "same content, same id");
    assert!(b.id().unwrap().starts_with("u-"));
    let d = normalize(r#"{"id":"..","root":{"type":"badge","text":"x"}}"#);
    assert!(d.id().unwrap().starts_with("u-"));
    assert_eq!(d.errors.len(), 1);
}

#[test]
fn children_come_after_their_parent() {
    let out = normalize(include_str!("specs/all-components.json"));
    let spec = out.spec.unwrap();
    for (i, n) in nodes(&spec).iter().enumerate() {
        for c in n["children"].as_array().into_iter().flatten() {
            assert!(c.as_u64().unwrap() as usize > i);
        }
    }
}

// Fences

#[test]
fn fences() {
    assert!(is_component_fence("acc-ui"));
    assert!(is_component_fence("Mermaid"));
    assert!(is_component_fence("html render"));
    assert!(!is_component_fence("html"));
    assert!(!is_component_fence("json"));
    assert!(!is_component_fence(""));

    assert!(parse_fence("rust", "fn main() {}").is_none());
    assert!(parse_fence("html", "<b>x</b>").is_none());

    let m = parse_fence("mermaid", "graph TD; A-->B").unwrap();
    assert_eq!(nodes(m.spec.as_ref().unwrap())[0]["type"], "mermaid");

    let c = parse_fence("chart", r#"{"kind":"bar","series":[{"points":[["a",1]]}]}"#).unwrap();
    assert_eq!(
        nodes(c.spec.as_ref().unwrap())[0]["series"][0]["points"][0]["x"],
        "a"
    );

    let h = parse_fence("html render", "<p>hi</p>").unwrap();
    assert_eq!(nodes(h.spec.as_ref().unwrap())[0]["html"], "<p>hi</p>");

    let bad = parse_fence("acc-ui", "{ not json").unwrap();
    assert!(bad.spec.is_none() && !bad.errors.is_empty());
    let bad_chart = parse_fence("chart", "[1,2]").unwrap();
    assert!(bad_chart.spec.is_none());
}

#[test]
fn csv_chart_fence() {
    let out = parse_fence(
        "chart",
        "kind: line\ntitle: p95 (ms)\ny: ms\nday,before,after\nMon,120,80\nTue,135,\nWed,\"1,000\",75\n",
    )
    .unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let chart = &nodes(out.spec.as_ref().unwrap())[0];
    assert_eq!(chart["kind"], "line");
    assert_eq!(chart["title"], "p95 (ms)");
    assert_eq!(chart["x_label"], "day");
    assert_eq!(chart["y_label"], "ms");
    assert_eq!(chart["series"][0]["name"], "before");
    assert_eq!(
        chart["series"][0]["points"].as_array().unwrap().len(),
        2,
        "1,000 isn't a number"
    );
    assert_eq!(
        chart["series"][1]["points"].as_array().unwrap().len(),
        2,
        "empty cell skipped"
    );
    assert_eq!(chart["series"][0]["points"][0], json!({ "x": "Mon", "y": 120.0 }));
}

#[test]
fn csv_chart_kind_follows_x() {
    let kind = |body: &str| {
        let out = parse_fence("chart", body).unwrap();
        nodes(out.spec.as_ref().unwrap())[0]["kind"].clone()
    };
    assert_eq!(kind("n,load\n1,2\n2,3"), "line");
    assert_eq!(kind("svc,ms\napi,2\nweb,3"), "bar");
    assert_eq!(kind("kind: pie\nsvc,ms\napi,2"), "pie");
    let tabs = parse_fence("chart", "a\tb\nx\t1").unwrap();
    assert!(tabs.spec.is_some());
    for bad in ["", "only-one-column\n1", "kind: nope\na,b\nx,1"] {
        assert!(parse_fence("chart", bad).unwrap().spec.is_none(), "{bad:?}");
    }
}
