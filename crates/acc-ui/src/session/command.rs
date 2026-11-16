//! What a shell command is: a test run, a build, a file read, a search.
//!
//! Conservative on purpose. A command is recognised only when every part of
//! it is understood: `cd x && cargo test 2>&1 | tail -20` is a test run,
//! `python3 build.py && cat out.txt` is [`CommandKind::Other`]. Command
//! substitution, subshells, heredocs and process substitution give up
//! straight away. Paths that need expansion (`$HOME/x`, `*.rs`, `~/x`) are
//! never reported as read.

use serde::Serialize;

use super::output;

/// What kind of work a command does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// Not recognised, or several kinds of work at once.
    #[default]
    Other,
    /// A test runner: `cargo test`, `pytest`, `go test`, `npm test`.
    Test,
    /// A build, compile or type check: `cargo build`, `tsc`, `make`.
    Build,
    /// A linter: `cargo clippy`, `eslint`, `ruff check`, `go vet`.
    Lint,
    /// A formatter: `cargo fmt`, `prettier`, `black`.
    Format,
    /// Installing dependencies: `npm install`, `pip install`, `cargo add`.
    Install,
    /// Git, other than `git grep`.
    Git,
    /// A content search: `rg`, `grep`, `find`, `git grep`.
    Search,
    /// Listing files: `ls`, `tree`, `rg --files`.
    List,
    /// Showing files: `cat`, `head`, `tail`, `sed -n`, `bat`, `nl`.
    Read,
}

impl CommandKind {
    /// Which kind wins when one command does several (`cargo build &&
    /// cargo test` is a test run).
    fn rank(self) -> u8 {
        match self {
            Self::Other => 0,
            Self::List => 1,
            Self::Read => 2,
            Self::Search => 3,
            Self::Git => 4,
            Self::Install => 5,
            Self::Format => 6,
            Self::Lint => 7,
            Self::Build => 8,
            Self::Test => 9,
        }
    }
}

/// Test counts from a runner's summary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TestCounts {
    pub passed: u32,
    /// Failures and errors.
    pub failed: u32,
    pub skipped: u32,
}

/// Compiler or linter findings, from its summary or its diagnostic lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Diagnostics {
    pub errors: u32,
    pub warnings: u32,
}

/// The tool family, which says how to read the output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Family {
    #[default]
    Unknown,
    Cargo,
    Go,
    Python,
    Js,
    Deno,
    Swift,
    Ruby,
    Maven,
    Dotnet,
    Tsc,
    Eslint,
    Ruff,
    Mypy,
}

/// A file a command reads: the word as written and where it points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadArg {
    pub raw: String,
    pub path: String,
}

/// What [`recognise`] and [`Recognised::read_output`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Recognised {
    pub kind: CommandKind,
    /// The program, with its subcommand where that says what runs:
    /// `cargo test`, `pytest`, `git commit`, `rg`.
    pub tool: Option<String>,
    pub family: Family,
    pub reads: Vec<ReadArg>,
    pub tests: Option<TestCounts>,
    pub diagnostics: Option<Diagnostics>,
    /// What the output says: `Some(false)` for failed tests or a failed
    /// build, `Some(true)` for a clear success marker.
    pub verdict: Option<bool>,
}

impl Recognised {
    /// Reads a finished command's output: test counts, diagnostics and
    /// whether it says it passed.
    pub fn read_output(&mut self, out: &str) {
        let tests = (self.kind == CommandKind::Test).then(|| output::tests(self.family, out));
        // Compile errors also stop test runs, for the compiled languages.
        let compiled = matches!(self.family, Family::Cargo | Family::Go | Family::Swift);
        let diags = (matches!(self.kind, CommandKind::Build | CommandKind::Lint)
            || (self.kind == CommandKind::Test && compiled))
            .then(|| output::diagnostics(self.family, self.tool.as_deref(), out));
        self.tests = tests.and_then(|t| t.counts);
        self.diagnostics = diags.and_then(|d| d.counts);
        let tv = tests.and_then(|t| t.verdict);
        let dv = diags.and_then(|d| d.verdict);
        self.verdict = if tv == Some(false) || dv == Some(false) {
            Some(false)
        } else if self.kind == CommandKind::Test {
            tv
        } else {
            dv
        };
        // A file that isn't there wasn't read.
        self.reads
            .retain(|r| !out.contains(&format!("{}: No such file or directory", r.raw)));
    }
}

/// Recognises `command` (unwrapped from its shell), run in `cwd`, which
/// relative paths resolve against; without one they aren't reported.
pub(crate) fn recognise(command: &str, cwd: Option<&str>) -> Recognised {
    let Some(segments) = lex(command).map(split) else {
        return Recognised::default();
    };
    let mut cwd = cwd.map(normalize);
    let mut best: Option<Known> = None;
    let mut reads = Vec::new();
    for seg in &segments {
        match classify(seg, &mut cwd) {
            Class::Trivial => {}
            Class::Unknown => return Recognised::default(),
            Class::Known(k) => {
                reads.extend(k.reads.iter().cloned());
                if best.as_ref().is_none_or(|b| k.kind.rank() > b.kind.rank()) {
                    best = Some(k);
                }
            }
        }
    }
    let Some(best) = best else {
        return Recognised::default();
    };
    Recognised {
        kind: best.kind,
        tool: Some(best.tool),
        family: best.family,
        reads,
        ..Recognised::default()
    }
}

// Lexing

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    /// A word, and whether it is literal (no unquoted expansion or glob).
    Word(String, bool),
    /// `|` or `|&`: the next command reads this one's output.
    Pipe,
    /// `&&`, `||`, `;`, `&` or a newline.
    Sep,
    /// `>`, `>>`, `<`, `&>`: the next word is a file, not an argument.
    Redirect,
}

/// The command as words and operators, or `None` when it uses shell
/// features this doesn't follow.
fn lex(s: &str) -> Option<Vec<Tok>> {
    let mut toks = Vec::new();
    let mut chars = s.chars().peekable();
    let mut word = String::new();
    let mut literal = true;
    let mut in_word = false;
    macro_rules! end_word {
        () => {
            if in_word {
                toks.push(Tok::Word(std::mem::take(&mut word), literal));
                literal = true;
                in_word = false;
            }
        };
    }
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => end_word!(),
            '\n' | ';' => {
                end_word!();
                toks.push(Tok::Sep);
            }
            '#' if !in_word => {
                while chars.peek().is_some_and(|&c| c != '\n') {
                    chars.next();
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            e @ ('"' | '\\' | '$' | '`') => word.push(e),
                            '\n' => {}
                            e => {
                                word.push('\\');
                                word.push(e);
                            }
                        },
                        '`' => return None,
                        '$' => {
                            if chars.peek() == Some(&'(') {
                                return None;
                            }
                            literal = false;
                            word.push('$');
                        }
                        c => word.push(c),
                    }
                }
            }
            '\\' => match chars.next() {
                Some('\n') => {}
                Some(e) => {
                    in_word = true;
                    word.push(e);
                }
                None => {}
            },
            '`' | '(' | ')' => return None,
            '$' => {
                if chars.peek() == Some(&'(') {
                    return None;
                }
                in_word = true;
                literal = false;
                word.push('$');
            }
            '|' => {
                end_word!();
                match chars.peek() {
                    Some('|') => {
                        chars.next();
                        toks.push(Tok::Sep);
                    }
                    Some('&') => {
                        chars.next();
                        toks.push(Tok::Pipe);
                    }
                    _ => toks.push(Tok::Pipe),
                }
            }
            '&' => {
                end_word!();
                match chars.peek() {
                    Some('&') => {
                        chars.next();
                        toks.push(Tok::Sep);
                    }
                    Some('>') => {
                        chars.next();
                        if chars.peek() == Some(&'>') {
                            chars.next();
                        }
                        toks.push(Tok::Redirect);
                    }
                    _ => toks.push(Tok::Sep),
                }
            }
            '>' | '<' => {
                // `2>`: the digits were a file descriptor, not a word.
                if in_word && literal && !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
                    word.clear();
                    in_word = false;
                }
                end_word!();
                match chars.peek() {
                    Some('<' | '(') => return None,
                    Some('>') if c == '>' => {
                        chars.next();
                    }
                    _ => {}
                }
                if chars.peek() == Some(&'&') {
                    // `>&2`, `2>&1`, `>&-`: a descriptor, no file.
                    chars.next();
                    while chars.peek().is_some_and(|c| c.is_ascii_digit() || *c == '-') {
                        chars.next();
                    }
                } else {
                    toks.push(Tok::Redirect);
                }
            }
            '*' | '?' | '[' | '{' | '}' => {
                in_word = true;
                literal = false;
                word.push(c);
            }
            '~' if !in_word => {
                in_word = true;
                literal = false;
                word.push(c);
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        toks.push(Tok::Word(word, literal));
    }
    Some(toks)
}

/// One simple command: its words, redirect targets dropped, and whether it
/// reads the previous command's output.
#[derive(Debug, Clone, Default)]
struct Segment {
    words: Vec<(String, bool)>,
    piped: bool,
}

fn split(toks: Vec<Tok>) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut seg = Segment::default();
    let mut redirect = false;
    for t in toks {
        match t {
            Tok::Word(w, lit) => {
                if !std::mem::take(&mut redirect) {
                    seg.words.push((w, lit));
                }
            }
            Tok::Redirect => redirect = true,
            Tok::Pipe => {
                out.push(std::mem::take(&mut seg));
                seg.piped = true;
            }
            Tok::Sep => {
                out.push(std::mem::take(&mut seg));
            }
        }
    }
    out.push(seg);
    out.retain(|s| !s.words.is_empty());
    out
}

// Classifying

enum Class {
    /// Changes nothing worth showing (`cd`, `echo`), or filters the
    /// previous command's output (`| tail -20`).
    Trivial,
    Unknown,
    Known(Known),
}

struct Known {
    kind: CommandKind,
    tool: String,
    family: Family,
    reads: Vec<ReadArg>,
}

fn known(kind: CommandKind, tool: impl Into<String>, family: Family) -> Class {
    Class::Known(Known {
        kind,
        tool: tool.into(),
        family,
        reads: Vec::new(),
    })
}

const TRIVIAL: &[&str] = &[
    "pwd", "echo", "printf", "true", ":", "export", "set", "unset", "sleep", "clear", "which", "type", "date",
];

/// Programs that only transform their input, after a `|`.
const FILTERS: &[&str] = &[
    "head", "tail", "grep", "egrep", "fgrep", "rg", "sort", "uniq", "wc", "cut", "tr", "sed", "awk", "jq",
    "cat", "tee", "column", "nl", "less", "more", "fold", "rev", "paste",
];

fn classify(seg: &Segment, cwd: &mut Option<String>) -> Class {
    let words: Vec<&(String, bool)> = seg.words.iter().collect();
    let words = strip_prefixes(&words);
    let Some(((first, first_lit), rest)) = words.split_first().map(|(f, r)| (*f, r)) else {
        return Class::Trivial;
    };
    if !first_lit {
        return Class::Unknown;
    }
    let mut program = first.rsplit('/').next().unwrap_or(first.as_str());
    let mut rest: Vec<&(String, bool)> = rest.to_vec();
    // `python -m pytest` runs `pytest`.
    if matches!(program, "python" | "python3" | "py") || program.starts_with("python3.") {
        match rest.first() {
            Some((m, true)) if m == "-m" && rest.len() > 1 && rest[1].1 => {
                program = rest[1].0.as_str();
                rest.drain(..2);
            }
            _ => return Class::Unknown,
        }
    }
    let args: Vec<&str> = rest.iter().map(|(w, _)| w.as_str()).collect();
    if program == "cd" {
        *cwd = match rest.first() {
            Some((d, true)) => resolve(cwd.as_deref(), d),
            _ => None,
        };
        return Class::Trivial;
    }
    if seg.piped && FILTERS.contains(&program) && !(program == "sed" && has_in_place(&args)) {
        return Class::Trivial;
    }
    if TRIVIAL.contains(&program) {
        return Class::Trivial;
    }
    if let Some(c) = read_command(program, &rest, cwd.as_deref()) {
        return c;
    }
    tool_command(program, &args)
}

/// Drops `FOO=bar`, `env`, `time`, `timeout 60`, `nice`, `command`, `npx`
/// and other prefixes that run the command after them.
fn strip_prefixes<'a>(mut w: &'a [&'a (String, bool)]) -> &'a [&'a (String, bool)] {
    loop {
        let Some(((first, _), rest)) = w.split_first().map(|(f, r)| (*f, r)) else {
            return w;
        };
        let skip_opts = |r: &'a [&'a (String, bool)]| {
            let n = r.iter().take_while(|(a, _)| a.starts_with('-')).count();
            &r[n..]
        };
        w = match first.as_str() {
            a if is_assignment(a) => rest,
            "env" => {
                let r = skip_opts(rest);
                let n = r.iter().take_while(|(a, _)| is_assignment(a)).count();
                &r[n..]
            }
            "time" | "command" | "exec" | "nohup" | "sudo" => skip_opts(rest),
            "nice" => {
                let r = skip_opts(rest);
                // `nice -n 10 x`: the value went with the option.
                if rest.first().is_some_and(|(a, _)| a == "-n") {
                    r.get(1..).unwrap_or(&[])
                } else {
                    r
                }
            }
            "timeout" => {
                let r = skip_opts(rest);
                r.get(1..).unwrap_or(&[])
            }
            "npx" | "bunx" => skip_opts(rest),
            "pnpm" | "yarn" if rest.first().is_some_and(|(a, _)| a == "exec" || a == "dlx") => &rest[1..],
            "uv" | "poetry" | "pipenv" | "rye" | "pdm" if rest.first().is_some_and(|(a, _)| a == "run") => {
                skip_opts(&rest[1..])
            }
            "bundle" if rest.first().is_some_and(|(a, _)| a == "exec") => &rest[1..],
            _ => return w,
        };
    }
}

fn is_assignment(w: &str) -> bool {
    w.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty()
            && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && !k.as_bytes()[0].is_ascii_digit()
    })
}

fn has_in_place(args: &[&str]) -> bool {
    args.iter().any(|a| {
        *a == "--in-place"
            || a.starts_with("--in-place=")
            || (a.starts_with('-') && !a.starts_with("--") && a[1..].contains('i'))
    })
}

/// `cat`, `head`, `tail`, `sed -n`, `bat`, `nl`, `less`: the files they show.
fn read_command(program: &str, rest: &[&(String, bool)], cwd: Option<&str>) -> Option<Class> {
    // Options that take the next word as their value.
    let valued: &[&str] = match program {
        "cat" | "nl" | "less" | "more" => &[],
        "head" | "tail" => &[
            "-n",
            "-c",
            "--lines",
            "--bytes",
            "-s",
            "--sleep-interval",
            "--pid",
        ],
        "bat" | "batcat" => &[
            "-l",
            "--language",
            "-r",
            "--line-range",
            "--style",
            "--theme",
            "-H",
            "--highlight-line",
            "-m",
            "--map-syntax",
        ],
        "sed" => &["-e", "--expression", "-f", "--file", "-l", "--line-length"],
        _ => return None,
    };
    let args: Vec<&str> = rest.iter().map(|(w, _)| w.as_str()).collect();
    if program == "sed" {
        // Only `sed -n '…p' file` shows a file; other scripts change text.
        let quiet = args.iter().any(|a| {
            *a == "-n"
                || *a == "--quiet"
                || *a == "--silent"
                || (a.starts_with('-') && !a.starts_with("--") && a[1..].contains('n') && !a.contains('e'))
        });
        if !quiet || has_in_place(&args) {
            return Some(Class::Unknown);
        }
    }
    let mut files = Vec::new();
    let mut i = 0;
    let mut opts = true;
    let mut script_given = program == "sed"
        && args
            .iter()
            .any(|a| matches!(*a, "-e" | "--expression" | "-f" | "--file"));
    while i < rest.len() {
        let (w, lit) = rest[i];
        i += 1;
        if opts && w == "--" {
            opts = false;
        } else if opts && w.starts_with('-') && w != "-" {
            if valued.contains(&w.as_str()) {
                i += 1;
            }
        } else if program == "sed" && !script_given {
            script_given = true;
        } else if w != "-" {
            files.push((w.as_str(), *lit));
        }
    }
    if files.is_empty() {
        // Reads standard input: nothing to show.
        return Some(Class::Unknown);
    }
    let reads = files
        .into_iter()
        .filter(|(_, lit)| *lit)
        .filter_map(|(raw, _)| {
            Some(ReadArg {
                raw: raw.to_owned(),
                path: resolve(cwd, raw)?,
            })
        })
        .collect();
    Some(Class::Known(Known {
        kind: CommandKind::Read,
        tool: program.to_owned(),
        family: Family::Unknown,
        reads,
    }))
}

/// The first argument that isn't an option (or a `+toolchain`).
fn subcommand<'a>(args: &[&'a str]) -> Option<&'a str> {
    args.iter()
        .copied()
        .find(|a| !a.starts_with('-') && !a.starts_with('+'))
}

/// A package manager script name (`test`, `test:unit`, `build`) as a kind.
fn script_kind(name: &str) -> Option<CommandKind> {
    let base = name.split([':', '-']).next().unwrap_or(name);
    Some(match base {
        "test" | "tests" | "e2e" => CommandKind::Test,
        "build" | "compile" | "typecheck" | "tsc" | "check" if name != "check-format" => CommandKind::Build,
        "lint" => CommandKind::Lint,
        "format" | "fmt" | "prettier" => CommandKind::Format,
        _ => return None,
    })
}

fn tool_command(program: &str, args: &[&str]) -> Class {
    use CommandKind as K;
    use Family as F;
    let sub = subcommand(args);
    let has = |w: &str| args.contains(&w);
    match program {
        "cargo" => match sub {
            Some("test" | "t") => known(K::Test, "cargo test", F::Cargo),
            Some("nextest") => known(K::Test, "cargo nextest", F::Cargo),
            Some("build" | "b" | "check" | "c") => {
                known(K::Build, format!("cargo {}", long_cargo(sub)), F::Cargo)
            }
            Some("clippy") => known(K::Lint, "cargo clippy", F::Cargo),
            Some("fmt") => known(K::Format, "cargo fmt", F::Cargo),
            Some("add" | "install" | "fetch") => {
                known(K::Install, format!("cargo {}", sub.unwrap_or("")), F::Cargo)
            }
            _ => Class::Unknown,
        },
        "go" => match sub {
            Some("test") => known(K::Test, "go test", F::Go),
            Some("build") => known(K::Build, "go build", F::Go),
            Some("vet") => known(K::Lint, "go vet", F::Go),
            Some("fmt") => known(K::Format, "go fmt", F::Go),
            Some("get") => known(K::Install, "go get", F::Go),
            Some("mod") if args.iter().any(|a| matches!(*a, "tidy" | "download")) => {
                known(K::Install, "go mod", F::Go)
            }
            _ => Class::Unknown,
        },
        "pytest" | "py.test" => known(K::Test, "pytest", F::Python),
        "unittest" => known(K::Test, "unittest", F::Python),
        "tox" | "nox" => known(K::Test, program, F::Python),
        "npm" | "pnpm" | "yarn" | "bun" => js_manager(program, args),
        "jest" | "vitest" | "mocha" | "ava" => known(K::Test, program, F::Js),
        "playwright" | "cypress" if matches!(sub, Some("test" | "run")) => known(K::Test, program, F::Js),
        "deno" => match sub {
            Some("test") => known(K::Test, "deno test", F::Deno),
            Some("check") => known(K::Build, "deno check", F::Deno),
            Some("lint") => known(K::Lint, "deno lint", F::Deno),
            Some("fmt") => known(K::Format, "deno fmt", F::Deno),
            _ => Class::Unknown,
        },
        "swift" => match sub {
            Some("test") => known(K::Test, "swift test", F::Swift),
            Some("build") => known(K::Build, "swift build", F::Swift),
            _ => Class::Unknown,
        },
        "xcodebuild" => {
            if has("test") || has("test-without-building") {
                known(K::Test, "xcodebuild test", F::Unknown)
            } else if args
                .iter()
                .any(|a| matches!(*a, "-list" | "-showsdks" | "-version" | "-showBuildSettings"))
            {
                Class::Unknown
            } else {
                known(K::Build, "xcodebuild", F::Unknown)
            }
        }
        "make" | "gmake" | "just" => {
            let target = args
                .iter()
                .copied()
                .find(|a| !a.starts_with('-') && !a.contains('='));
            match target {
                None if program != "just" => known(K::Build, program, F::Unknown),
                Some("build" | "all") => known(
                    K::Build,
                    format!("{program} {}", target.unwrap_or("")),
                    F::Unknown,
                ),
                Some("test" | "tests" | "check") => {
                    known(K::Test, format!("{program} {}", target.unwrap_or("")), F::Unknown)
                }
                Some("lint") => known(K::Lint, format!("{program} lint"), F::Unknown),
                Some("fmt" | "format") => known(
                    K::Format,
                    format!("{program} {}", target.unwrap_or("")),
                    F::Unknown,
                ),
                _ => Class::Unknown,
            }
        }
        "ninja" => known(K::Build, "ninja", F::Unknown),
        "cmake" if has("--build") => known(K::Build, "cmake --build", F::Unknown),
        "ctest" => known(K::Test, "ctest", F::Unknown),
        "gradle" | "gradlew" | "mvn" | "mvnw" => {
            let family = if program.starts_with("mvn") {
                F::Maven
            } else {
                F::Unknown
            };
            let name = program.trim_end_matches('w');
            if args
                .iter()
                .any(|a| matches!(*a, "test" | "check" | "verify") || a.ends_with(":test"))
            {
                known(K::Test, format!("{name} test"), family)
            } else if args
                .iter()
                .any(|a| matches!(*a, "build" | "assemble" | "compile" | "package" | "install"))
            {
                known(K::Build, format!("{name} build"), family)
            } else {
                Class::Unknown
            }
        }
        "dotnet" => match sub {
            Some("test") => known(K::Test, "dotnet test", F::Dotnet),
            Some("build") => known(K::Build, "dotnet build", F::Dotnet),
            _ => Class::Unknown,
        },
        "mix" => match sub {
            Some("test") => known(K::Test, "mix test", F::Unknown),
            Some("compile") => known(K::Build, "mix compile", F::Unknown),
            _ => Class::Unknown,
        },
        "rspec" => known(K::Test, "rspec", F::Ruby),
        "phpunit" => known(K::Test, "phpunit", F::Unknown),
        "zig" => match (sub, args.get(1).copied()) {
            (Some("build"), Some("test")) => known(K::Test, "zig build test", F::Unknown),
            (Some("build"), _) => known(K::Build, "zig build", F::Unknown),
            (Some("test"), _) => known(K::Test, "zig test", F::Unknown),
            _ => Class::Unknown,
        },
        "tsc" | "vue-tsc" => known(K::Build, program, F::Tsc),
        "rustc" | "gcc" | "g++" | "cc" | "c++" | "clang" | "clang++" | "javac" | "swiftc" | "kotlinc" => {
            known(K::Build, program, F::Unknown)
        }
        "eslint" => known(K::Lint, "eslint", F::Eslint),
        "ruff" => match sub {
            Some("format") => known(K::Format, "ruff format", F::Ruff),
            None | Some("check") => known(K::Lint, "ruff", F::Ruff),
            // `ruff src/`: the old way to check a path.
            Some(s) if s.contains('/') || s.contains('.') => known(K::Lint, "ruff", F::Ruff),
            _ => Class::Unknown,
        },
        "mypy" | "pyright" => known(K::Lint, program, F::Mypy),
        "flake8" | "pylint" | "golangci-lint" | "staticcheck" | "swiftlint" | "shellcheck" | "oxlint"
        | "stylelint" | "ktlint" => known(K::Lint, program, F::Unknown),
        "biome" => match sub {
            Some("lint" | "check" | "ci") => known(K::Lint, "biome", F::Unknown),
            Some("format") => known(K::Format, "biome format", F::Unknown),
            _ => Class::Unknown,
        },
        "prettier" | "black" | "isort" | "gofmt" | "goimports" | "rustfmt" | "swift-format"
        | "swiftformat" | "clang-format" | "shfmt" => known(K::Format, program, F::Unknown),
        "pip" | "pip3" => match sub {
            Some("install") => known(K::Install, "pip install", F::Python),
            _ => Class::Unknown,
        },
        "uv" => match sub {
            Some("add" | "sync") => known(K::Install, format!("uv {}", sub.unwrap_or("")), F::Python),
            Some("pip") if has("install") => known(K::Install, "uv pip install", F::Python),
            _ => Class::Unknown,
        },
        "poetry" | "bundle" | "gem" | "brew" | "apt" | "apt-get"
            if matches!(sub, Some("install" | "add")) =>
        {
            known(K::Install, format!("{program} {}", sub.unwrap_or("")), F::Unknown)
        }
        "git" => git(args),
        "rg" if args.contains(&"--files") => known(K::List, "rg --files", F::Unknown),
        "rg" | "grep" | "egrep" | "fgrep" | "ag" | "ack" | "fd" | "fdfind" | "find" => {
            known(K::Search, program, F::Unknown)
        }
        "ls" | "tree" => known(K::List, program, F::Unknown),
        _ => Class::Unknown,
    }
}

fn long_cargo(sub: Option<&str>) -> &'static str {
    match sub {
        Some("b" | "build") => "build",
        _ => "check",
    }
}

fn js_manager(program: &str, args: &[&str]) -> Class {
    use CommandKind as K;
    let sub = subcommand(args);
    let tool = |kind, what: &str| known(kind, format!("{program} {what}"), Family::Js);
    match sub {
        None if program == "yarn" => tool(K::Install, "install"),
        None => Class::Unknown,
        Some("install" | "i" | "add" | "ci") => tool(K::Install, sub.unwrap_or("")),
        Some("test" | "t") => tool(K::Test, "test"),
        Some("run" | "run-script") => {
            let script = args.iter().copied().filter(|a| !a.starts_with('-')).nth(1);
            match script.and_then(script_kind) {
                Some(k) => tool(k, script.unwrap_or("")),
                None => Class::Unknown,
            }
        }
        // `pnpm build`, `yarn lint`, `bun run test`; `npm build` isn't a script.
        Some(s) if program != "npm" => match script_kind(s) {
            Some(k) => tool(k, s),
            None => Class::Unknown,
        },
        _ => Class::Unknown,
    }
}

fn git(args: &[&str]) -> Class {
    // Global options before the subcommand; `-C dir` and `-c k=v` take a value.
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        i += if matches!(args[i], "-C" | "-c") { 2 } else { 1 };
    }
    match args.get(i).copied() {
        Some("grep") => known(CommandKind::Search, "git grep", Family::Unknown),
        Some("ls-files") => known(CommandKind::List, "git ls-files", Family::Unknown),
        Some(sub) => known(CommandKind::Git, format!("git {sub}"), Family::Unknown),
        None => Class::Unknown,
    }
}

// Paths

/// `path` made absolute against `cwd` and tidied (`.` and `..` resolved
/// by name); `None` when it is relative and there is no `cwd`.
fn resolve(cwd: Option<&str>, path: &str) -> Option<String> {
    if path.starts_with('/') {
        return Some(normalize(path));
    }
    Some(normalize(&format!("{}/{path}", cwd?)))
}

fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(c: &str) -> (CommandKind, Option<String>) {
        let r = recognise(c, Some("/w"));
        (r.kind, r.tool)
    }

    fn reads(c: &str, cwd: Option<&str>) -> Vec<String> {
        recognise(c, cwd).reads.into_iter().map(|r| r.path).collect()
    }

    #[test]
    fn recognises_tools() {
        use CommandKind as K;
        for (c, k, t) in [
            ("cargo test -p acc-ui", K::Test, "cargo test"),
            ("cargo +nightly t", K::Test, "cargo test"),
            ("cargo nextest run", K::Test, "cargo nextest"),
            ("RUST_LOG=debug cargo build --release", K::Build, "cargo build"),
            (
                "cargo clippy --all-targets -- -D warnings",
                K::Lint,
                "cargo clippy",
            ),
            ("python3 -m unittest", K::Test, "unittest"),
            ("python -m pytest -q tests/", K::Test, "pytest"),
            ("uv run pytest", K::Test, "pytest"),
            ("go test ./...", K::Test, "go test"),
            ("go vet ./...", K::Lint, "go vet"),
            ("npm test", K::Test, "npm test"),
            ("npm run build", K::Build, "npm build"),
            ("pnpm test:unit", K::Test, "pnpm test:unit"),
            ("pnpm install --frozen-lockfile", K::Install, "pnpm install"),
            ("yarn", K::Install, "yarn install"),
            ("npx tsc --noEmit", K::Build, "tsc"),
            ("npx -y vitest run", K::Test, "vitest"),
            ("swift test --filter X", K::Test, "swift test"),
            ("xcodebuild -scheme App test", K::Test, "xcodebuild test"),
            ("make", K::Build, "make"),
            ("make -j8 test", K::Test, "make test"),
            ("./gradlew test", K::Test, "gradle test"),
            ("timeout 600 cargo test", K::Test, "cargo test"),
            ("git -C repo status --short", K::Git, "git status"),
            ("git --no-pager diff --stat", K::Git, "git diff"),
            ("git grep foo", K::Search, "git grep"),
            ("rg -n 'fn main' src", K::Search, "rg"),
            ("rg --files -g '*.rs'", K::List, "rg --files"),
            ("ls -la", K::List, "ls"),
            ("ruff check .", K::Lint, "ruff"),
            ("ruff format", K::Format, "ruff format"),
            ("pip install -r requirements.txt", K::Install, "pip install"),
            ("cat -n a.py", K::Read, "cat"),
            ("sed -n '1,80p' a.py", K::Read, "sed"),
        ] {
            assert_eq!(kind(c), (k, Some(t.to_owned())), "{c}");
        }
    }

    #[test]
    fn compound_commands_take_the_weightiest_part() {
        use CommandKind as K;
        assert_eq!(kind("cd crates/x && cargo test 2>&1 | tail -20").0, K::Test);
        assert_eq!(kind("cargo build && cargo test").0, K::Test);
        assert_eq!(kind("pwd && rg --files").0, K::List);
        assert_eq!(kind("cargo fmt && git diff").0, K::Format);
        assert_eq!(kind("cargo test || true").0, K::Test);
        assert_eq!(
            kind("git status; git log --oneline -5").1.as_deref(),
            Some("git status")
        );
        assert_eq!(kind("python3 -m unittest 2>&1 | tail -8").0, K::Test);
        assert_eq!(kind("cat a.txt | grep foo | wc -l").0, K::Read);
    }

    #[test]
    fn anything_unclear_is_other() {
        for c in [
            "python3 build.py && cat out.txt",
            "./run.sh",
            "echo $(cargo test)",
            "cargo test `x`",
            "(cd x && cargo test)",
            "cat <<EOF\nhi\nEOF",
            "diff <(cat a) <(cat b)",
            "cargo run",
            "cat",
            "cargo test | python3 summarise.py",
            "sed -i 's/a/b/' x.rs",
            "sed 's/a/b/' x.rs",
            "make deploy",
            "npm build",
            "for f in *.rs; do cat $f; done",
            "zsh -lc 'unterminated",
            "echo hi",
            "",
        ] {
            assert_eq!(recognise(c, Some("/w")).kind, CommandKind::Other, "{c}");
        }
    }

    #[test]
    fn reads_resolve_against_cwd_and_cd() {
        assert_eq!(
            reads("cat -n /work/inventory.py; cat -n /work/test_inventory.py", None),
            ["/work/inventory.py", "/work/test_inventory.py"]
        );
        assert_eq!(
            reads("cat AGENTS.md && cat ./src/../inventory.py", Some("/work")),
            ["/work/AGENTS.md", "/work/inventory.py"]
        );
        assert_eq!(
            reads("cd sub && head -n 20 a.rs b.rs", Some("/w")),
            ["/w/sub/a.rs", "/w/sub/b.rs"]
        );
        assert_eq!(reads("tail -f -n 5 log.txt", Some("/w")), ["/w/log.txt"]);
        assert_eq!(reads("sed -n '10,20p' src/x.rs", Some("/w")), ["/w/src/x.rs"]);
        assert_eq!(reads("sed -n -e 1p a b", Some("/w")), ["/w/a", "/w/b"]);
        assert_eq!(reads("bat -l rust --style plain x", Some("/w")), ["/w/x"]);
        // No cwd: relative paths are unknown.
        assert!(reads("cat a.rs", None).is_empty());
        // Expansions and globs aren't paths.
        assert!(reads("cat $HOME/x ~/y *.rs src/{a,b}.rs", Some("/w")).is_empty());
        // `cd` somewhere unknown forgets the cwd.
        assert!(reads("cd $X && cat a", Some("/w")).is_empty());
        // Redirect targets and filters read nothing.
        assert_eq!(reads("cat a > b", Some("/w")), ["/w/a"]);
        assert_eq!(
            reads("cargo test 2>&1 | head -50", Some("/w")),
            Vec::<String>::new()
        );
    }

    #[test]
    fn missing_files_were_not_read() {
        let mut r = recognise("cat a b", Some("/w"));
        r.read_output("hello\ncat: b: No such file or directory\n");
        assert_eq!(r.reads.len(), 1);
        assert_eq!(r.reads[0].path, "/w/a");
    }

    #[test]
    fn lexes_quotes_and_redirects() {
        assert_eq!(
            lex(r#"a 'b c' "d \"e\"" f\ g 2>&1 >x"#).unwrap(),
            vec![
                Tok::Word("a".into(), true),
                Tok::Word("b c".into(), true),
                Tok::Word("d \"e\"".into(), true),
                Tok::Word("f g".into(), true),
                Tok::Redirect,
                Tok::Word("x".into(), true),
            ]
        );
        assert_eq!(
            lex("a && b || c; d | e |& f &")
                .unwrap()
                .iter()
                .filter(|t| **t == Tok::Sep)
                .count(),
            4
        );
        assert_eq!(
            lex("echo \"$X\"").unwrap(),
            vec![Tok::Word("echo".into(), true), Tok::Word("$X".into(), false)]
        );
    }
}
