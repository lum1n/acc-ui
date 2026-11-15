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
        Some(serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad JSON from server: {e}: {line}")))
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
    assert!(dir.join("s1/pick-env/pid").exists());

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
    assert!(!dir.join("s1/pick-env/pid").exists());

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
    assert!(!dir.join("s1/gone/pid").exists());
}
