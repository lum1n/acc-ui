//! Golden-file comparison shared by the tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub fn dir(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(sub)
}

/// Compares `actual` with the file at `path`; with `UPDATE_GOLDEN=1` set,
/// writes it instead.
pub fn golden(path: &Path, actual: &str) {
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("{} is missing; run with UPDATE_GOLDEN=1", path.display()));
    assert!(
        expected == actual,
        "{} differs; run with UPDATE_GOLDEN=1 and review the diff.\n--- actual ---\n{actual}",
        path.display()
    );
}
