//! The MCP server script, run with a real shell: setup, the JSON-RPC
//! exchange, answers written by `answer_script`, cancellation, timeouts.
//!
//! `ACC_UI_TEST_SH` picks the shell command (default `sh`);
//! scripts/test-shells.sh runs these under dash+mawk and busybox too.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use acc_ui_agent::{Answer, AskState, answer_script, recognize_tool, server_script, setup_script};
use serde_json::{Value, json};

/// The shell command: `sh`, or `ACC_UI_TEST_SH` split on whitespace.
fn shell() -> Command {
    let sh = std::env::var("ACC_UI_TEST_SH").unwrap_or_else(|_| "sh".to_owned());
    let mut words = sh.split_whitespace();
    let mut cmd = Command::new(words.next().unwrap_or("sh"));
    cmd.args(words);
    cmd
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("acc-ui-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run_sh(script: &str, dir: &Path) -> String {
    let out = shell()
        .arg("-c")
        .arg(script)
        .env("ACC_UI_DIR", dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Server {
    fn start(script: &Path, dir: &Path, timeout_secs: u32) -> Server {
        Server::start_reading_after(script, dir, timeout_secs, Duration::ZERO)
    }

    /// Starts the server and leaves its stdout unread for `delay`, so its
    /// writes queue up behind a full pipe.
    fn start_reading_after(script: &Path, dir: &Path, timeout_secs: u32, delay: Duration) -> Server {
        let mut child = shell()
            .arg(script)
            .env("ACC_UI_DIR", dir)
            .env("ACC_UI_SESSION", "s1")
            .env("ACC_UI_ASK_TIMEOUT", timeout_secs.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Server {
            stdin: child.stdin.take(),
            child,
            lines,
        }
    }

    fn send(&mut self, msg: &str) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    fn recv(&self, wait: Duration) -> Option<Value> {
        let line = self.lines.recv_timeout(wait).ok()?;
        Some(serde_json::from_str(&line).unwrap_or_else(|e| {
            let start: String = line.chars().take(300).collect();
            panic!("bad JSON from server: {e}: {start}...")
        }))
    }

    fn call(&mut self, msg: Value) -> Value {
        self.send(&msg.to_string());
        let r = self.recv(Duration::from_secs(10)).expect("no reply");
        assert_eq!(r["id"], msg["id"], "{r}");
        r
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn tool_call(id: Value, name: &str, args: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": args } })
}

fn text_of(r: &Value) -> &str {
    r["result"]["content"][0]["text"].as_str().unwrap_or_default()
}

fn install(dir: &Path) -> PathBuf {
    let path = run_sh(&setup_script(), dir).trim().to_owned();
    assert!(path.starts_with(dir.to_str().unwrap()), "{path}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), server_script());
    // A second run keeps the same file.
    assert_eq!(run_sh(&setup_script(), dir).trim(), path);
    PathBuf::from(path)
}

#[test]
fn json_rpc_exchange() {
    let dir = temp_dir("rpc");
    let script = install(&dir);
    let mut s = Server::start(&script, &dir, 30);

    // Claude Code's initialize, as recorded.
    let init = s.call(json!({"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{"roots":{},"elicitation":{}},"clientInfo":{"name":"claude-code","version":"2.3.1"}},"jsonrpc":"2.0","id":0}));
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(init["result"]["serverInfo"]["name"], "acc_ui");
    s.send(r#"{"method":"notifications/initialized","jsonrpc":"2.0"}"#);

    let list = s.call(json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}));
    assert_eq!(list["result"], acc_ui_agent::host::tools_list());

    let ping = s.call(json!({"jsonrpc":"2.0","id":"p","method":"ping"}));
    assert_eq!(ping["result"], json!({}));

    let render = s.call(tool_call(
        json!(2),
        "acc_ui_render",
        json!({ "spec": { "v": "acc-ui/1", "root": { "type": "badge", "text": "q \" } ] , \\ ünï 🚀" } } }),
    ));
    assert!(text_of(&render).starts_with("acc-ui: shown"), "{render}");

    let unknown = s.call(json!({"jsonrpc":"2.0","id":3,"method":"resources/list"}));
    assert_eq!(unknown["error"]["code"], -32601);

    let bad_tool = s.call(tool_call(json!(4), "nope", json!({})));
    assert_eq!(bad_tool["error"]["code"], -32602);

    s.send("this is not json");
    let parse = s.recv(Duration::from_secs(5)).unwrap();
    assert_eq!(parse["error"]["code"], -32700);
    assert_eq!(parse["id"], Value::Null);

    for (id, spec) in [
        (5, json!("{\"id\":\"q\"}")),
        (6, json!({ "id": "a/b", "root": {} })),
        (7, json!({ "id": 7, "root": {} })),
        (
            8,
            json!({ "root": { "type": "badge", "text": "x", "id": "nested-is-not-the-spec-id" } }),
        ),
    ] {
        let r = s.call(tool_call(json!(id), "acc_ui_ask", json!({ "spec": spec })));
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert!(text_of(&r).starts_with("acc-ui: error"), "{r}");
    }

    // A long line full of escapes, as an html node makes.
    let html = "<div class=\"x\">\\ \" é</div>\n".repeat(6000);
    let started = Instant::now();
    let r = s.call(tool_call(
        json!(9),
        "acc_ui_render",
        json!({ "spec": { "root": { "type": "html", "html": html } } }),
    ));
    assert!(text_of(&r).starts_with("acc-ui: shown"));
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );

    // EOF ends the server.
    drop(s.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "server didn't exit on EOF");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn ask_is_answered_through_the_file() {
    let dir = temp_dir("ask");
    let script = install(&dir);
    let mut s = Server::start(&script, &dir, 30);

    let spec = json!({ "v": "acc-ui/1", "id": "pick-env", "root": { "type": "form", "fields": [{ "name": "env", "kind": "select", "options": ["prod", "staging"] }] } });
    s.send(&tool_call(json!(10), "acc_ui_ask", json!({ "spec": spec })).to_string());
    // Other requests are served while the ask waits.
    let ping = s.call(json!({"jsonrpc":"2.0","id":11,"method":"ping"}));
    assert_eq!(ping["result"], json!({}));
    assert!(dir.join("s1/pick-env/waiting").exists());

    let values = json!({ "env": "prod", "note": "it's \"fine\" </acc-ui-submit> $(rm -rf /) `x`" });
    let Value::Object(map) = values.clone() else {
        unreachable!()
    };
    run_sh(
        &answer_script("s1", "pick-env", &Answer::Values(map)).unwrap(),
        &dir,
    );

    let r = s.recv(Duration::from_secs(10)).expect("no answer");
    assert_eq!(r["id"], 10);
    assert_eq!(r["result"]["structuredContent"]["values"], values);
    let call = recognize_tool(
        "mcp__acc_ui__acc_ui_ask",
        &json!({ "spec": spec }),
        Some(text_of(&r)),
    )
    .unwrap();
    match call.ask.unwrap() {
        AskState::Answered(sub) => {
            assert_eq!(sub.id, "pick-env");
            assert_eq!(Value::Object(sub.values), values);
        }
        other => panic!("{other:?}"),
    }
    assert!(!dir.join("s1/pick-env/waiting").exists());

    // Dismissing.
    s.send(&tool_call(json!(12), "acc_ui_ask", json!({ "spec": spec })).to_string());
    std::thread::sleep(Duration::from_millis(300));
    run_sh(
        &answer_script("s1", "pick-env", &Answer::Dismissed).unwrap(),
        &dir,
    );
    let r = s.recv(Duration::from_secs(10)).expect("no answer");
    assert_eq!(r["id"], 12);
    assert!(text_of(&r).starts_with("acc-ui: dismissed"), "{r}");
}

#[test]
fn ask_times_out_and_can_be_cancelled() {
    let dir = temp_dir("timeout");
    let script = install(&dir);
    let mut s = Server::start(&script, &dir, 2);

    s.send(
        &tool_call(
            json!("t"),
            "acc_ui_ask",
            json!({ "spec": { "id": "slow", "root": { "type": "badge", "text": "x" } } }),
        )
        .to_string(),
    );
    s.send(
        &tool_call(
            json!("c"),
            "acc_ui_ask",
            json!({ "spec": { "id": "gone", "root": { "type": "badge", "text": "x" } } }),
        )
        .to_string(),
    );
    std::thread::sleep(Duration::from_millis(500));
    s.send(
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"c","reason":"user"}}"#,
    );

    let r = s.recv(Duration::from_secs(10)).expect("no timeout reply");
    assert_eq!(r["id"], "t");
    assert!(text_of(&r).starts_with("acc-ui: timed out"), "{r}");
    let call = recognize_tool("acc_ui_ask", &json!({}), Some(text_of(&r))).unwrap();
    assert_eq!(call.ask, Some(AskState::TimedOut));
    // The cancelled call never replies.
    assert!(s.recv(Duration::from_secs(3)).is_none());
    assert!(!dir.join("s1/gone/waiting").exists());
}

/// Writes `answer.json` for `id` the way `answer_script` does: a temporary
/// file renamed into place.
fn write_answer(dir: &Path, id: &str, answer: &Answer) {
    let d = dir.join("s1").join(id);
    std::fs::create_dir_all(&d).unwrap();
    let tmp = d.join(".answer.test");
    let body = acc_ui_agent::host::answer_result(id, answer);
    std::fs::write(&tmp, format!("{body}\n")).unwrap();
    std::fs::rename(&tmp, d.join("answer.json")).unwrap();
}

fn badge_spec(id: &str) -> Value {
    json!({ "id": id, "root": { "type": "badge", "text": "x" } })
}

/// Replies far longer than PIPE_BUF, from several waiting asks at once and
/// from the main loop, each arrive as one whole line.
#[test]
fn concurrent_long_replies_dont_interleave() {
    let dir = temp_dir("interleave");
    let script = install(&dir);
    let mut s = Server::start_reading_after(&script, &dir, 30, Duration::from_secs(4));

    let asks = 6;
    for i in 0..asks {
        let spec = badge_spec(&format!("big-{i}"));
        s.send(&tool_call(json!(format!("a{i}")), "acc_ui_ask", json!({ "spec": spec })).to_string());
    }
    std::thread::sleep(Duration::from_millis(500));
    for i in 0..asks {
        let mut values = serde_json::Map::new();
        values.insert("blob".to_owned(), json!(i.to_string().repeat(150_000)));
        write_answer(&dir, &format!("big-{i}"), &Answer::Values(values));
    }
    // Keep the main loop writing long replies while the waiters wake up.
    let lists = 60;
    for i in 0..lists {
        s.send(&json!({"jsonrpc":"2.0","id":format!("l{i}"),"method":"tools/list"}).to_string());
        std::thread::sleep(Duration::from_millis(25));
    }

    let (mut answered, mut listed) = (0, 0);
    while answered < asks || listed < lists {
        let r = s.recv(Duration::from_secs(20)).expect("missing replies");
        let id = r["id"].as_str().unwrap().to_owned();
        if let Some(i) = id.strip_prefix('a') {
            let blob = r["result"]["structuredContent"]["values"]["blob"]
                .as_str()
                .unwrap();
            assert!(blob == i.repeat(150_000), "{id}: wrong blob");
            answered += 1;
        } else {
            assert_eq!(r["result"], acc_ui_agent::host::tools_list(), "{id}");
            listed += 1;
        }
    }
}

fn wait_exit(s: &mut Server, secs: u64) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while s.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "server didn't exit");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Stdout closes once the server is gone: nothing it started outlives it.
fn assert_stdout_closed(s: &Server, secs: u64) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        match s.lines.recv_timeout(Duration::from_millis(100)) {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            Ok(line) => panic!("unexpected output: {line}"),
            Err(_) => assert!(Instant::now() < deadline, "stdout still open"),
        }
    }
}

/// The server's private state (`$ACC_UI_DIR/run~`) is gone.
fn assert_no_run_state(dir: &Path) {
    let left: Vec<_> = std::fs::read_dir(dir.join("run~"))
        .map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}

/// On EOF the server answers what it already read, stops its waiters and
/// cleans up.
#[test]
fn eof_with_waiting_asks() {
    let dir = temp_dir("eof");
    let script = install(&dir);
    let mut s = Server::start(&script, &dir, 30);

    s.send(&tool_call(json!(40), "acc_ui_ask", json!({ "spec": badge_spec("w1") })).to_string());
    s.send(&json!({"jsonrpc":"2.0","id":41,"method":"tools/list"}).to_string());
    drop(s.stdin.take());
    let r = s.recv(Duration::from_secs(5)).expect("no reply");
    assert_eq!(r["id"], 41);
    wait_exit(&mut s, 5);
    assert_stdout_closed(&s, 3);
    assert!(!dir.join("s1/w1/waiting").exists());
    assert_no_run_state(&dir);
}

/// SIGTERM stops the server and its waiters and cleans up.
#[test]
fn terminated_with_waiting_asks() {
    let dir = temp_dir("term");
    let script = install(&dir);
    let mut s = Server::start(&script, &dir, 30);

    s.send(&tool_call(json!(50), "acc_ui_ask", json!({ "spec": badge_spec("w2") })).to_string());
    let ping = s.call(json!({"jsonrpc":"2.0","id":51,"method":"ping"}));
    assert_eq!(ping["result"], json!({}));
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(s.child.id().to_string())
        .status()
        .unwrap();
    assert!(status.success());
    wait_exit(&mut s, 5);
    assert_stdout_closed(&s, 3);
    assert_no_run_state(&dir);
}
