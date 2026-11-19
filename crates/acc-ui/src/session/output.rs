//! Reading test runners', compilers' and linters' output: counts and
//! whether it passed. Only summary lines each tool prints itself count;
//! anything else leaves the verdict open.

use super::command::{Diagnostics, Family, TestCounts};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TestFacts {
    pub counts: Option<TestCounts>,
    pub verdict: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DiagFacts {
    pub counts: Option<Diagnostics>,
    pub verdict: Option<bool>,
}

/// A test run's summary, read the way its runner prints it; for runners
/// that run anything (`make test`, `npm test`), the first summary found.
pub(crate) fn tests(family: Family, out: &str) -> TestFacts {
    type P = fn(&str) -> Option<TestFacts>;
    let parsers: &[P] = match family {
        Family::Cargo => &[cargo_tests],
        Family::Go => &[go_tests],
        Family::Python => &[pytest, unittest],
        Family::Js => &[jest_vitest, mocha, bun],
        Family::Deno => &[deno],
        Family::Swift => &[swift_tests],
        Family::Ruby => &[rspec],
        Family::Maven => &[maven],
        Family::Dotnet => &[dotnet],
        _ => &[
            cargo_tests,
            pytest,
            unittest,
            jest_vitest,
            mocha,
            go_tests,
            rspec,
            swift_tests,
            maven,
        ],
    };
    let mut facts = parsers.iter().find_map(|p| p(out)).unwrap_or_default();
    // `** TEST FAILED **` and friends, from xcodebuild.
    if out.contains("** TEST FAILED **") {
        facts.verdict = Some(false);
    } else if facts.verdict.is_none() && out.contains("** TEST SUCCEEDED **") {
        facts.verdict = Some(true);
    }
    facts
}

/// A build's or linter's findings.
pub(crate) fn diagnostics(family: Family, tool: Option<&str>, out: &str) -> DiagFacts {
    let mut facts = match family {
        Family::Cargo => cargo_diags(out),
        Family::Go => go_diags(out),
        Family::Tsc => tsc(out),
        Family::Eslint => eslint(out),
        Family::Ruff => ruff(out),
        Family::Mypy => mypy(out),
        Family::Swift => {
            let mut f = colon_diags(out);
            if out.lines().any(|l| l.trim_start().starts_with("Build complete!")) && f.verdict.is_none() {
                f.verdict = Some(true);
            }
            f
        }
        _ => colon_diags(out),
    };
    if out.contains("** BUILD FAILED **") {
        facts.verdict = Some(false);
    } else if facts.verdict.is_none() && out.contains("** BUILD SUCCEEDED **") {
        facts.verdict = Some(true);
    }
    // `go build`, `go vet`, `tsc` and `eslint` print nothing when all is well.
    let silent =
        matches!(family, Family::Go | Family::Tsc | Family::Eslint) && !tool.is_some_and(|t| t == "go test");
    if silent && facts.verdict.is_none() && out.trim().is_empty() {
        facts = DiagFacts {
            counts: Some(Diagnostics::default()),
            verdict: Some(true),
        };
    }
    facts
}

// Helpers

/// Each `<number> <word>` in `s`, the word lower-cased and stripped of
/// punctuation: `"3 passed, 1 failed in 0.2s"` gives `(3, "passed")`,
/// `(1, "failed")`, `(0, "in")`… callers look up the words they know.
fn counts(s: &str) -> Vec<(u32, String)> {
    let words: Vec<&str> = s.split_whitespace().collect();
    words
        .windows(2)
        .filter_map(|w| {
            // `(3 errors`, but not `(6)` or `1,234`.
            let n = w[0].strip_prefix('(').unwrap_or(w[0]);
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let n = n.parse().ok()?;
            let word = w[1]
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase();
            Some((n, word))
        })
        .collect()
}

/// Each `<word>: <number>` or `<word>=<number>` in `s`: `"Failures: 1"`,
/// `failures=2`.
fn keyed(s: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for part in s.split([',', '(', ')', '|']) {
        let Some((k, v)) = part.split_once([':', '=']) else {
            continue;
        };
        let k = k.trim().trim_start_matches(|c: char| !c.is_alphanumeric());
        if let Ok(n) = v.trim().parse() {
            out.push((k.to_ascii_lowercase(), n));
        }
    }
    out
}

fn sum(list: &[(u32, String)], words: &[&str]) -> u32 {
    list.iter()
        .filter(|(_, w)| words.contains(&w.as_str()))
        .map(|(n, _)| n)
        .sum()
}

fn key(list: &[(String, u32)], k: &str) -> u32 {
    list.iter().filter(|(w, _)| w == k).map(|(_, n)| n).sum()
}

fn verdict(c: TestCounts) -> Option<bool> {
    if c.failed > 0 {
        Some(false)
    } else if c.passed > 0 {
        Some(true)
    } else {
        None
    }
}

fn facts(c: TestCounts) -> TestFacts {
    TestFacts {
        counts: Some(c),
        verdict: verdict(c),
    }
}

fn lines(out: &str) -> impl Iterator<Item = &str> {
    out.lines().map(str::trim)
}

// Test runners

/// `test result: ok. 4 passed; 0 failed; 1 ignored; …`, one per test
/// binary; nextest's `Summary […] 4 tests run: 3 passed, 1 failed, 1 skipped`.
fn cargo_tests(out: &str) -> Option<TestFacts> {
    let mut c = TestCounts::default();
    let mut found = false;
    let mut failed = false;
    for l in lines(out) {
        if let Some(rest) = l.strip_prefix("test result: ") {
            found = true;
            failed |= rest.starts_with("FAILED");
            let n = counts(rest);
            c.passed += sum(&n, &["passed"]);
            c.failed += sum(&n, &["failed"]);
            c.skipped += sum(&n, &["ignored"]);
        } else if l.starts_with("Summary [")
            && let Some((_, rest)) = l.split_once("tests run:").or_else(|| l.split_once("test run:"))
        {
            found = true;
            let n = counts(rest);
            c.passed += sum(&n, &["passed"]);
            c.failed += sum(&n, &["failed", "timed"]);
            c.skipped += sum(&n, &["skipped"]);
        }
    }
    found.then(|| {
        let mut f = facts(c);
        if failed {
            f.verdict = Some(false);
        } else if f.verdict.is_none() {
            // Every binary reported ok, with nothing to run.
            f.verdict = Some(true);
        }
        f
    })
}

/// `--- PASS: TestX`, `--- FAIL: TestY` (top-level only, with `-v`), and
/// the package lines `ok  pkg 0.1s` / `FAIL pkg 0.1s` / `FAIL`.
fn go_tests(out: &str) -> Option<TestFacts> {
    let mut c = TestCounts::default();
    let (mut ok, mut fail, mut per_test) = (false, false, false);
    for l in out.lines() {
        if l.starts_with("--- PASS:") {
            c.passed += 1;
            per_test = true;
        } else if l.starts_with("--- FAIL:") {
            c.failed += 1;
            per_test = true;
            fail = true;
        } else if l.starts_with("--- SKIP:") {
            c.skipped += 1;
            per_test = true;
        } else if l == "FAIL" || l.starts_with("FAIL\t") || l.starts_with("FAIL ") {
            fail = true;
        } else if l == "PASS" || l.starts_with("ok \t") || l.starts_with("ok  ") || l.starts_with("ok\t") {
            ok = true;
        }
    }
    (ok || fail || per_test).then(|| TestFacts {
        counts: per_test.then_some(c),
        verdict: if fail {
            Some(false)
        } else {
            ok.then_some(true).or(verdict(c))
        },
    })
}

/// The last `=== 1 failed, 3 passed, 1 skipped in 0.12s ===` or `-q`'s
/// `4 passed in 0.01s`.
fn pytest(out: &str) -> Option<TestFacts> {
    let line = lines(out).filter(|l| is_pytest_summary(l)).last()?;
    let body = line.trim_matches(|c: char| c == '=' || c.is_whitespace());
    let body = body.rsplit_once(" in ").map_or(body, |(b, _)| b);
    let n = counts(body);
    Some(facts(TestCounts {
        passed: sum(&n, &["passed", "xpassed"]),
        failed: sum(&n, &["failed", "error", "errors"]),
        skipped: sum(&n, &["skipped", "xfailed", "deselected"]),
    }))
}

fn is_pytest_summary(l: &str) -> bool {
    let body = l.trim_matches(|c: char| c == '=' || c.is_whitespace());
    let Some((counts_part, time)) = body.rsplit_once(" in ") else {
        return false;
    };
    let time = time.split_whitespace().next().unwrap_or("");
    let timed = time.ends_with('s') && time.trim_end_matches('s').parse::<f64>().is_ok();
    timed
        && counts(counts_part)
            .iter()
            .any(|(_, w)| matches!(w.as_str(), "passed" | "failed" | "error" | "errors"))
}

/// `Ran 4 tests in 0.001s`, then `OK`, `OK (skipped=1)` or `FAILED
/// (failures=1, errors=1)`.
fn unittest(out: &str) -> Option<TestFacts> {
    let all: Vec<&str> = lines(out).collect();
    let i = all.iter().rposition(|l| {
        l.strip_prefix("Ran ")
            .is_some_and(|r| r.contains(" test") && r.contains(" in "))
    })?;
    let ran: u32 = all[i]["Ran ".len()..].split_whitespace().next()?.parse().ok()?;
    let status = all[i + 1..].iter().find(|l| !l.is_empty())?;
    if !(status.starts_with("OK") || status.starts_with("FAILED")) {
        return None;
    }
    let k = keyed(status);
    let failed = key(&k, "failures") + key(&k, "errors") + key(&k, "unexpected successes");
    let skipped = key(&k, "skipped") + key(&k, "expected failures");
    Some(TestFacts {
        counts: Some(TestCounts {
            passed: ran.saturating_sub(failed + skipped),
            failed,
            skipped,
        }),
        verdict: Some(status.starts_with("OK")),
    })
}

/// Jest's `Tests:       1 failed, 5 passed, 6 total`; Vitest's
/// `Tests  1 failed | 5 passed (6)`.
fn jest_vitest(out: &str) -> Option<TestFacts> {
    let line = lines(out)
        .filter(|l| l.starts_with("Tests:") || l.starts_with("Tests "))
        .last()?;
    let n = counts(line);
    let c = TestCounts {
        passed: sum(&n, &["passed"]),
        failed: sum(&n, &["failed"]),
        skipped: sum(&n, &["skipped", "todo", "pending"]),
    };
    (c != TestCounts::default()).then(|| facts(c))
}

/// `  5 passing (12ms)`, `  1 failing`, `  2 pending`.
fn mocha(out: &str) -> Option<TestFacts> {
    let mut c = TestCounts::default();
    let mut found = false;
    for l in lines(out) {
        let mut w = l.split_whitespace();
        let (Some(n), Some(word)) = (w.next(), w.next()) else {
            continue;
        };
        let Ok(n) = n.parse::<u32>() else { continue };
        match word {
            "passing" if l.ends_with(')') => {
                c.passed = n;
                found = true;
            }
            "failing" if found => c.failed = n,
            "pending" if found => c.skipped = n,
            _ => {}
        }
    }
    found.then(|| facts(c))
}

/// `bun test`: ` 5 pass`, ` 1 fail`, ` 2 skip` on lines of their own.
fn bun(out: &str) -> Option<TestFacts> {
    let mut c = TestCounts::default();
    let mut found = false;
    for l in lines(out) {
        let Some((n, word)) = l.split_once(' ') else {
            continue;
        };
        let Ok(n) = n.parse::<u32>() else { continue };
        match word {
            "pass" => c.passed = n,
            "fail" => c.failed = n,
            "skip" => c.skipped = n,
            _ => continue,
        }
        found = true;
    }
    found.then(|| facts(c))
}

/// `ok | 5 passed | 0 failed (12ms)` or `FAILED | 4 passed | 1 failed`.
fn deno(out: &str) -> Option<TestFacts> {
    let line = lines(out)
        .filter(|l| (l.starts_with("ok |") || l.starts_with("FAILED |")) && l.contains(" passed"))
        .last()?;
    let n = counts(line);
    Some(TestFacts {
        counts: Some(TestCounts {
            passed: sum(&n, &["passed"]),
            failed: sum(&n, &["failed"]),
            skipped: sum(&n, &["ignored"]),
        }),
        verdict: Some(line.starts_with("ok")),
    })
}

/// XCTest's last `Executed 5 tests, with 1 failure (0 unexpected) in …`,
/// and Swift Testing's `Test run with 5 tests passed` / `… failed`.
fn swift_tests(out: &str) -> Option<TestFacts> {
    let xctest = lines(out).filter(|l| l.starts_with("Executed ")).last().map(|l| {
        let n = counts(l);
        let total = sum(&n, &["tests", "test"]);
        let failed = sum(&n, &["failure", "failures"]);
        TestCounts {
            passed: total.saturating_sub(failed),
            failed,
            skipped: sum(&n, &["skipped"]),
        }
    });
    let testing = lines(out)
        .filter_map(|l| {
            let rest = &l[l.find("Test run with ")? + "Test run with ".len()..];
            let n: u32 = rest.split_whitespace().next()?.parse().ok()?;
            Some((n, rest.contains(" failed")))
        })
        .last();
    if xctest.is_none() && testing.is_none() {
        return None;
    }
    let mut c = xctest.unwrap_or_default();
    let mut fail = c.failed > 0;
    if let Some((n, failed)) = testing {
        fail |= failed;
        if !failed {
            c.passed += n;
        }
    }
    Some(TestFacts {
        // A failed Swift Testing run doesn't say how many tests failed.
        counts: (xctest.is_some() || testing.is_some_and(|(_, f)| !f)).then_some(c),
        verdict: Some(!fail),
    })
}

/// `10 examples, 1 failure, 2 pending`.
fn rspec(out: &str) -> Option<TestFacts> {
    let line = lines(out)
        .filter(|l| l.contains(" example") && (l.contains(" failure")))
        .last()?;
    let n = counts(line);
    let total = sum(&n, &["example", "examples"]);
    let failed = sum(&n, &["failure", "failures"]);
    let skipped = sum(&n, &["pending"]);
    Some(facts(TestCounts {
        passed: total.saturating_sub(failed + skipped),
        failed,
        skipped,
    }))
}

/// The last `Tests run: 10, Failures: 1, Errors: 0, Skipped: 2`.
fn maven(out: &str) -> Option<TestFacts> {
    let line = lines(out).filter(|l| l.contains("Tests run:")).last()?;
    let k = keyed(line);
    let failed = key(&k, "failures") + key(&k, "errors");
    let skipped = key(&k, "skipped");
    Some(facts(TestCounts {
        passed: key(&k, "tests run").saturating_sub(failed + skipped),
        failed,
        skipped,
    }))
}

/// `Passed!  - Failed:     0, Passed:    10, Skipped:     0, Total: 10`.
fn dotnet(out: &str) -> Option<TestFacts> {
    let line = lines(out)
        .filter(|l| (l.starts_with("Passed!") || l.starts_with("Failed!")) && l.contains("Total:"))
        .last()?;
    let k = keyed(line.split_once(" - ").map_or(line, |(_, r)| r));
    Some(TestFacts {
        counts: Some(TestCounts {
            passed: key(&k, "passed"),
            failed: key(&k, "failed"),
            skipped: key(&k, "skipped"),
        }),
        verdict: Some(line.starts_with("Passed!")),
    })
}

// Builds and linters

/// Cargo: `error: could not compile `x` (lib) due to 2 previous errors`,
/// `warning: `x` (lib) generated 3 warnings`, `Finished …`.
fn cargo_diags(out: &str) -> DiagFacts {
    let mut d = Diagnostics::default();
    let (mut summarised_errors, mut loose_errors, mut failed, mut finished) = (0, 0, false, false);
    for l in lines(out) {
        if l.starts_with("error: could not compile") {
            failed = true;
            if let Some((_, rest)) = l.split_once("due to ") {
                summarised_errors += sum(&counts(rest), &["previous"]);
            }
        } else if l.starts_with("error[") || (l.starts_with("error:") && !not_a_diagnostic(l)) {
            loose_errors += 1;
        } else if l.starts_with("warning: ") && l.contains(" generated ") {
            let n = counts(l.rsplit_once(" generated ").map_or("", |(_, r)| r));
            d.warnings += sum(&n, &["warning", "warnings"]);
        } else if l.starts_with("Finished ") {
            finished = true;
        }
    }
    d.errors = if summarised_errors > 0 {
        summarised_errors
    } else {
        loose_errors
    };
    let verdict = if failed || d.errors > 0 {
        Some(false)
    } else {
        finished.then_some(true)
    };
    DiagFacts {
        counts: (verdict.is_some() || d != Diagnostics::default()).then_some(d),
        verdict,
    }
}

/// Cargo's own `error:` lines that report no compiler finding: the end of
/// a failed compile, and failed test targets (`error: test failed, to rerun
/// pass `--lib``, `error: 2 targets failed:`).
fn not_a_diagnostic(l: &str) -> bool {
    l.starts_with("error: aborting")
        || l.starts_with("error: test failed")
        || (l.starts_with("error: ") && l.contains(" targets failed"))
}

/// Go: one `file.go:12:3: message` line per problem.
fn go_diags(out: &str) -> DiagFacts {
    let errors = u32::try_from(
        out.lines()
            .filter(|l| {
                l.split_once(".go:")
                    .is_some_and(|(f, r)| !f.contains(' ') && r.starts_with(|c: char| c.is_ascii_digit()))
            })
            .count(),
    )
    .unwrap_or(u32::MAX);
    DiagFacts {
        counts: (errors > 0).then_some(Diagnostics { errors, warnings: 0 }),
        verdict: (errors > 0 || out.lines().any(|l| l.starts_with("FAIL"))).then_some(false),
    }
}

/// `Found 3 errors in 2 files.` / `Found 1 error.`, else `error TS2304` lines.
fn tsc(out: &str) -> DiagFacts {
    let found = lines(out)
        .filter_map(|l| l.strip_prefix("Found "))
        .map(|r| sum(&counts(r), &["error", "errors"]))
        .find(|n| *n > 0);
    let errors = found.unwrap_or_else(|| {
        u32::try_from(out.lines().filter(|l| l.contains(" error TS")).count()).unwrap_or(u32::MAX)
    });
    DiagFacts {
        counts: (errors > 0).then_some(Diagnostics { errors, warnings: 0 }),
        verdict: (errors > 0).then_some(false),
    }
}

/// `✖ 5 problems (3 errors, 2 warnings)`.
fn eslint(out: &str) -> DiagFacts {
    let Some(l) = lines(out).filter(|l| l.contains(" problem")).last() else {
        return DiagFacts::default();
    };
    let n = counts(l);
    let d = Diagnostics {
        errors: sum(&n, &["error", "errors"]),
        warnings: sum(&n, &["warning", "warnings"]),
    };
    DiagFacts {
        counts: Some(d),
        verdict: Some(d.errors == 0),
    }
}

/// `Found 3 errors.` or `All checks passed!`.
fn ruff(out: &str) -> DiagFacts {
    if lines(out).any(|l| l == "All checks passed!") {
        return DiagFacts {
            counts: Some(Diagnostics::default()),
            verdict: Some(true),
        };
    }
    let errors = lines(out)
        .filter_map(|l| l.strip_prefix("Found "))
        .map(|r| sum(&counts(r), &["error", "errors"]))
        .find(|n| *n > 0);
    DiagFacts {
        counts: errors.map(|errors| Diagnostics { errors, warnings: 0 }),
        verdict: errors.map(|_| false),
    }
}

/// `Found 3 errors in 2 files (checked 10 source files)` or `Success: no
/// issues found`; pyright's `2 errors, 1 warning, 0 informations`.
fn mypy(out: &str) -> DiagFacts {
    if lines(out).any(|l| l.starts_with("Success: no issues found") || l.starts_with("0 errors, ")) {
        let warnings = lines(out)
            .find(|l| l.starts_with("0 errors, "))
            .map_or(0, |l| sum(&counts(l), &["warning", "warnings"]));
        return DiagFacts {
            counts: Some(Diagnostics { errors: 0, warnings }),
            verdict: Some(true),
        };
    }
    let line = lines(out)
        .filter(|l| {
            (l.starts_with("Found ") && l.contains(" error"))
                || (l.contains(" errors, ") && l.contains(" warning"))
        })
        .last();
    let Some(l) = line else {
        return DiagFacts::default();
    };
    let n = counts(l.strip_prefix("Found ").unwrap_or(l));
    let d = Diagnostics {
        errors: sum(&n, &["error", "errors"]),
        warnings: sum(&n, &["warning", "warnings"]),
    };
    DiagFacts {
        counts: Some(d),
        verdict: Some(d.errors == 0),
    }
}

/// Compilers' `file:line:col: error: message` and `…: warning: …` lines.
fn colon_diags(out: &str) -> DiagFacts {
    let count =
        |tag: &str| u32::try_from(out.lines().filter(|l| l.contains(tag)).count()).unwrap_or(u32::MAX);
    let d = Diagnostics {
        errors: count(": error: ") + count(": fatal error: "),
        warnings: count(": warning: "),
    };
    DiagFacts {
        counts: (d != Diagnostics::default()).then_some(d),
        verdict: (d.errors > 0).then_some(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(family: Family, out: &str) -> (Option<(u32, u32, u32)>, Option<bool>) {
        let f = tests(family, out);
        (f.counts.map(|c| (c.passed, c.failed, c.skipped)), f.verdict)
    }

    fn d(family: Family, tool: &str, out: &str) -> (Option<(u32, u32)>, Option<bool>) {
        let f = diagnostics(family, Some(tool), out);
        (f.counts.map(|c| (c.errors, c.warnings)), f.verdict)
    }

    #[test]
    fn cargo() {
        let out = "running 3 tests\ntest a ... ok\n\ntest result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\n   Doc-tests x\n\ntest result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured\n";
        assert_eq!(t(Family::Cargo, out), (Some((4, 2, 1)), Some(false)));
        assert_eq!(
            t(Family::Cargo, "test result: ok. 0 passed; 0 failed; 0 ignored"),
            (Some((0, 0, 0)), Some(true))
        );
        assert_eq!(
            t(
                Family::Cargo,
                "     Summary [   0.031s] 12 tests run: 11 passed, 1 failed, 2 skipped"
            ),
            (Some((11, 1, 2)), Some(false))
        );
        let build = "warning: unused variable: `x`\nwarning: `acc-ui` (lib) generated 2 warnings\nerror[E0425]: cannot find value `y`\nerror: could not compile `acc-ui` (lib) due to 1 previous error; 2 warnings emitted\n";
        assert_eq!(
            d(Family::Cargo, "cargo build", build),
            (Some((1, 2)), Some(false))
        );
        assert_eq!(
            d(
                Family::Cargo,
                "cargo clippy",
                "    Checking x v0.1.0\n    Finished `dev` profile in 1.2s\n"
            ),
            (Some((0, 0)), Some(true))
        );
        assert_eq!(d(Family::Cargo, "cargo build", "   Compiling x\n"), (None, None));
        // A failed test run compiled fine.
        let failed = "    Finished `test` profile in 0.2s\ntest result: FAILED. 2 passed; 1 failed\n\nerror: test failed, to rerun pass `--lib`\n";
        assert_eq!(d(Family::Cargo, "cargo test", failed), (Some((0, 0)), Some(true)));
        let failed = "error: 2 targets failed:\n    `--lib`\n    `--test api`\n";
        assert_eq!(d(Family::Cargo, "cargo test", failed), (None, None));
    }

    #[test]
    fn python() {
        assert_eq!(
            t(Family::Python, "....\n------\nRan 4 tests in 0.000s\n\nOK\n"),
            (Some((4, 0, 0)), Some(true))
        );
        assert_eq!(
            t(
                Family::Python,
                "Ran 5 tests in 0.01s\n\nFAILED (failures=1, errors=1, skipped=1)"
            ),
            (Some((2, 2, 1)), Some(false))
        );
        assert_eq!(
            t(
                Family::Python,
                "=========== 1 failed, 10 passed, 2 skipped in 0.12s ===========\n"
            ),
            (Some((10, 1, 2)), Some(false))
        );
        assert_eq!(
            t(Family::Python, "5 passed in 0.01s"),
            (Some((5, 0, 0)), Some(true))
        );
        assert_eq!(
            t(Family::Python, "== 3 passed, 1 warning in 2.50s =="),
            (Some((3, 0, 0)), Some(true))
        );
        assert_eq!(t(Family::Python, "collected 3 items\n..."), (None, None));
    }

    #[test]
    fn go() {
        assert_eq!(t(Family::Go, "ok  \texample.com/x\t0.01s\n"), (None, Some(true)));
        assert_eq!(
            t(
                Family::Go,
                "--- PASS: TestA (0.00s)\n--- FAIL: TestB (0.00s)\n    --- FAIL: TestB/sub\nFAIL\nFAIL\texample.com/x\t0.01s\n"
            ),
            (Some((1, 1, 0)), Some(false))
        );
        assert_eq!(
            d(
                Family::Go,
                "go build",
                "# example.com/x\n./main.go:5:2: undefined: y\n"
            ),
            (Some((1, 0)), Some(false))
        );
        assert_eq!(d(Family::Go, "go vet", ""), (Some((0, 0)), Some(true)));
        // A test run that printed nothing says nothing.
        assert_eq!(d(Family::Go, "go test", ""), (None, None));
    }

    #[test]
    fn javascript() {
        assert_eq!(
            t(Family::Js, "Tests:       1 failed, 5 passed, 6 total\nTime: 1s"),
            (Some((5, 1, 0)), Some(false))
        );
        assert_eq!(
            t(
                Family::Js,
                " Test Files  2 passed (2)\n      Tests  7 passed | 1 skipped (8)\n"
            ),
            (Some((7, 0, 1)), Some(true))
        );
        assert_eq!(
            t(Family::Js, "  5 passing (12ms)\n  1 failing\n"),
            (Some((5, 1, 0)), Some(false))
        );
        assert_eq!(
            t(Family::Js, " 3 pass\n 0 fail\nRan 3 tests across 1 files."),
            (Some((3, 0, 0)), Some(true))
        );
        assert_eq!(
            t(Family::Deno, "FAILED | 4 passed | 1 failed (20ms)"),
            (Some((4, 1, 0)), Some(false))
        );
        assert_eq!(
            d(
                Family::Tsc,
                "tsc",
                "a.ts(1,1): error TS2304: x\nFound 1 error in a.ts:1\n"
            ),
            (Some((1, 0)), Some(false))
        );
        assert_eq!(d(Family::Tsc, "tsc", ""), (Some((0, 0)), Some(true)));
        assert_eq!(
            d(Family::Eslint, "eslint", "✖ 5 problems (3 errors, 2 warnings)"),
            (Some((3, 2)), Some(false))
        );
    }

    #[test]
    fn others() {
        assert_eq!(
            t(
                Family::Swift,
                "Test Suite 'All tests' failed.\n\t Executed 5 tests, with 1 failure (0 unexpected) in 0.01 (0.02) seconds\n"
            ),
            (Some((4, 1, 0)), Some(false))
        );
        assert_eq!(
            t(
                Family::Swift,
                "✔ Test run with 3 tests passed after 0.001 seconds."
            ),
            (Some((3, 0, 0)), Some(true))
        );
        assert_eq!(
            t(
                Family::Swift,
                "✘ Test run with 3 tests failed after 0.1 seconds with 1 issue."
            ),
            (None, Some(false))
        );
        assert_eq!(
            t(Family::Ruby, "10 examples, 1 failure, 2 pending"),
            (Some((7, 1, 2)), Some(false))
        );
        assert_eq!(
            t(Family::Maven, "Tests run: 10, Failures: 1, Errors: 0, Skipped: 2"),
            (Some((7, 1, 2)), Some(false))
        );
        assert_eq!(
            t(
                Family::Dotnet,
                "Passed!  - Failed:     0, Passed:    10, Skipped:     1, Total:    11"
            ),
            (Some((10, 0, 1)), Some(true))
        );
        assert_eq!(t(Family::Unknown, "** TEST FAILED **"), (None, Some(false)));
        // `make test` running pytest underneath.
        assert_eq!(
            t(Family::Unknown, "== 2 passed in 0.1s =="),
            (Some((2, 0, 0)), Some(true))
        );
        assert_eq!(
            d(Family::Ruff, "ruff", "All checks passed!"),
            (Some((0, 0)), Some(true))
        );
        assert_eq!(
            d(Family::Ruff, "ruff", "x.py:1:1: F401 unused\nFound 1 error.\n"),
            (Some((1, 0)), Some(false))
        );
        assert_eq!(
            d(
                Family::Mypy,
                "mypy",
                "Found 2 errors in 1 file (checked 3 source files)"
            ),
            (Some((2, 0)), Some(false))
        );
        assert_eq!(
            d(Family::Unknown, "gcc", "a.c:1:1: warning: x\na.c:2:1: error: y\n"),
            (Some((1, 1)), Some(false))
        );
        assert_eq!(
            d(Family::Unknown, "xcodebuild", "** BUILD SUCCEEDED **"),
            (None, Some(true))
        );
        assert_eq!(d(Family::Unknown, "make", "cc -o x x.c\n"), (None, None));
    }
}
