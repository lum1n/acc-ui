//! Each `tests/specs/<name>.json` normalises to `<name>.out.json`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use serde_json::json;

#[test]
fn specs_match_golden_output() {
    let dir = common::dir("specs");
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".json") || name.ends_with(".out.json") {
            continue;
        }
        let input = std::fs::read_to_string(&path).unwrap();
        let out = acc_ui::normalize(&input);
        let actual =
            serde_json::to_string_pretty(&json!({ "spec": out.spec, "errors": out.errors })).unwrap();
        common::golden(&dir.join(name.replace(".json", ".out.json")), &(actual + "\n"));
        n += 1;
    }
    assert!(n >= 5);
}
