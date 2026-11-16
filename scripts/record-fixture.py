#!/usr/bin/env python3
"""Record an ACC session as NDJSON events, for acc-ui's session fixtures.

Runs `acc run <harness> <cwd> --json`, sends each prompt once the previous
turn completes, answers every permission request with its first allowing
option (or the first option), and writes the events it printed to OUT.

    scripts/record-fixture.py --acc ../acc/target/release/acc \\
        claude /tmp/scratch crates/acc-ui/tests/sessions/claude-fix.ndjson \\
        "first prompt" "second prompt"

A prompt starting with `:` is an `acc run` command (`:set permission plan`)
sent without waiting for a turn. `--image PATH` attaches an image to the
first prompt. `--answer OPTION_ID` answers the next question or plan
approval with that option id instead (`[["Blue"],["A","B"]]` encodes
answers to every question; see acc_ui::session::encode_answers).

`--acc-ui` gives a Claude session the acc_ui MCP server (acc-ui-agent's
`mcp_server`), installed under a temporary ACC_UI_DIR; each `--ui-answer
VALUES_JSON` answers the next `acc_ui_ask` call with the answer script.

OUT is scrubbed for publishing: `native` payloads are dropped except the
ones acc-ui reads (a request's questions or plan, a prompt's image refs),
the scratch path becomes `/work`, the home directory `/home/user`, and
quota account ids zeros. A path split across streamed deltas is caught by
scrubbing each item's whole text. `--scrub-only OUT CWD` re-scrubs an
existing recording.
"""

import argparse
import base64
import json
import mimetypes
import os
import queue
import shlex
import signal
import subprocess
import sys
import tempfile
import threading
import time

ALLOW = ("allow_once", "allow_always")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def is_mcp(event):
    return "mcp" in (event.get("title") or "").lower() or (
        (event.get("detail") or {}).get("kind") == "mcp"
    )


def pick_option(event):
    options = event.get("options") or []
    # Keep the user's own MCP servers (notes, trackers) out of fixtures.
    if is_mcp(event) and "acc_ui" not in json.dumps(event):
        for o in options:
            if o.get("kind") in ("reject_once", "reject_always"):
                return o["id"]
    for o in options:
        if o.get("kind") in ALLOW:
            return o["id"]
    for o in options:
        if o.get("kind") not in ("reject_once", "reject_always"):
            return o["id"]
    return options[0]["id"] if options else "yes"


def scripted(event):
    """Whether `--answer` answers this request: a question or a plan approval."""
    if event.get("kind") == "question":
        return True
    tool = ((event.get("detail") or {}).get("tool") or "").lower()
    return "exitplanmode" in (tool + (event.get("title") or "").lower())


def scrub_text(text, paths):
    for path, to in paths:
        text = text.replace(path, to)
    return text


def scrub_paths(cwd, extra=()):
    paths = [(cwd.rstrip("/"), "/work")]
    paths += [(p.rstrip("/"), "/tmp/acc-ui") for p in extra]
    return paths + [(os.path.expanduser("~"), "/home/user")]


def keep_native(e):
    """The part of `e`'s native payload acc-ui reads, if any."""
    native = e.get("native")
    if not isinstance(native, dict):
        return None
    if e.get("type") == "permission_requested":
        return native
    if e.get("type") == "item_started" and native.get("images"):
        return {"images": native["images"]}
    return None


def scrub(lines, paths):
    """Scrubbed copies of the NDJSON event `lines`."""
    events = [json.loads(l) for l in lines]
    for e in events:
        native = keep_native(e)
        e.pop("native", None)
        if native is not None:
            e["native"] = native
        if e.get("type") == "quota" and e.get("account"):
            e["account"] = "00000000-0000-0000-0000-000000000000"
    # Streamed text: scrub each item's whole text; where that changes it,
    # its first delta carries the scrubbed text and the rest are dropped.
    whole = {}
    for e in events:
        if e.get("type") == "item_delta":
            whole[e["id"]] = whole.get(e["id"], "") + e.get("text", "")
    rewrite = {i for i, t in whole.items() if scrub_text(t, paths) != t}
    out, done = [], set()
    for e in events:
        if e.get("type") == "item_delta" and e["id"] in rewrite:
            if e["id"] in done:
                continue
            e["text"] = scrub_text(whole[e["id"]], paths)
            done.add(e["id"])
        out.append(scrub_text(json.dumps(e, ensure_ascii=False), paths))
    return out


def attachment(path):
    media = mimetypes.guess_type(path)[0] or "image/png"
    with open(path, "rb") as f:
        return {"media_type": media, "data": base64.b64encode(f.read()).decode()}


def dump(what, *args):
    cmd = ["cargo", "run", "-q", "-p", "acc-ui-agent", "--example", "dump", "--", what, *args]
    return subprocess.run(cmd, cwd=ROOT, check=True, capture_output=True, text=True).stdout


class AccUi:
    """The acc_ui MCP server for a Claude session, as acc-ui-agent installs it."""

    SESSION = "rec"

    def __init__(self, harness):
        if harness != "claude":
            sys.exit("acc-record: --acc-ui wires Claude only")
        self.dir = tempfile.mkdtemp(prefix="acc-ui-rec-")
        env = {**os.environ, "ACC_UI_DIR": self.dir}
        script = subprocess.run(
            ["sh"], input=dump("setup"), env=env, check=True, capture_output=True, text=True
        ).stdout.strip()
        config = {"mcpServers": {"acc_ui": {
            "command": "sh",
            "args": [script],
            "env": {"ACC_UI_SESSION": self.SESSION, "ACC_UI_DIR": self.dir},
        }}}
        # `acc run` has no flag for session MCP servers; ACC passes Claude
        # its `LaunchRequest.mcp_servers` the same way.
        self.program = os.path.join(self.dir, "claude")
        with open(self.program, "w") as f:
            f.write(f"#!/bin/sh\nexec claude {shlex.quote('--mcp-config=' + json.dumps(config))} \"$@\"\n")
        os.chmod(self.program, 0o755)
        self.answered = set()

    def answer(self, event, values):
        """Answers the `acc_ui_ask` call `event` starts, once."""
        detail = event.get("detail") or {}
        if "acc_ui_ask" not in (detail.get("tool") or ""):
            return False
        spec = (detail.get("input") or {}).get("spec") or {}
        if isinstance(spec, str):
            spec = json.loads(spec)
        sid = spec.get("id")
        if not sid or event["id"] in self.answered:
            return False
        self.answered.add(event["id"])
        script = dump("answer", self.SESSION, sid, json.dumps(values))
        request = os.path.join(self.dir, self.SESSION, sid, "request")

        def deliver():
            # The server clears any answer when the call reaches it (after
            # Claude's permission prompt), so answer once it waits.
            deadline = time.monotonic() + 300
            while not os.path.exists(request) and time.monotonic() < deadline:
                time.sleep(0.2)
            env = {**os.environ, "ACC_UI_DIR": self.dir}
            subprocess.run(["sh"], input=script, env=env, check=True, text=True)
            print(f"acc-record: ui answer {sid!r} -> {values}", file=sys.stderr)

        threading.Thread(target=deliver, daemon=True).start()
        return True


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--scrub-only":
        out, cwd = sys.argv[2], sys.argv[3]
        with open(out) as f:
            lines = [l for l in f if l.strip()]
        with open(out, "w") as f:
            f.write("\n".join(scrub(lines, scrub_paths(cwd))) + "\n")
        return

    ap = argparse.ArgumentParser()
    ap.add_argument("--acc", default="acc")
    ap.add_argument("--turn-timeout", type=float, default=900)
    ap.add_argument("--extra", action="append", default=[], help="extra arg for acc run")
    ap.add_argument("--image", action="append", default=[], help="image for the first prompt")
    ap.add_argument("--answer", action="append", default=[], help="option id for the next question or plan")
    ap.add_argument("--acc-ui", action="store_true", help="give Claude the acc_ui MCP server")
    ap.add_argument("--ui-answer", action="append", default=[], help="values for the next acc_ui_ask")
    ap.add_argument("harness")
    ap.add_argument("cwd")
    ap.add_argument("out")
    ap.add_argument("prompts", nargs="+")
    args = ap.parse_args()

    acc_ui = AccUi(args.harness) if args.acc_ui else None
    cmd = [args.acc, "run", args.harness, args.cwd, "--json", *args.extra]
    if acc_ui:
        cmd += ["--program", acc_ui.program]
    proc = subprocess.Popen(
        cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1
    )
    lines = queue.Queue()

    def pump():
        for line in proc.stdout:
            lines.put(line)
        lines.put(None)

    threading.Thread(target=pump, daemon=True).start()

    def send(text):
        proc.stdin.write(text + "\n")
        proc.stdin.flush()

    prompts = list(args.prompts)
    images = [attachment(p) for p in args.image]
    answers = list(args.answer)
    ui_answers = [json.loads(v) for v in args.ui_answer]

    def next_prompt():
        """Sends commands up to the next prompt and that; False when none is left."""
        nonlocal images
        while prompts:
            text = prompts.pop(0)
            if text.startswith(":"):
                send(text)
                continue
            if images:
                text = json.dumps({"type": "prompt", "text": text, "attachments": images})
                images = []
            send(text)
            return True
        return False

    # Killed (a `timeout` around it): save what was recorded so far.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit("acc-record: terminated"))
    events = []
    try:
        record(args, lines, send, next_prompt, events, answers, acc_ui, ui_answers)
    finally:
        proc.stdin.close()
        try:
            proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            proc.kill()
        extra = [acc_ui.dir] if acc_ui else []
        with open(args.out, "w") as f:
            f.write("\n".join(scrub(events, scrub_paths(os.path.abspath(args.cwd), extra))) + "\n")
        print(f"acc-record: {len(events)} events -> {args.out}", file=sys.stderr)


def record(args, lines, send, next_prompt, events, answers, acc_ui, ui_answers):
    """Reads events into `events` until the last turn completes."""
    next_prompt()
    deadline = time.monotonic() + args.turn_timeout
    while True:
        try:
            line = lines.get(timeout=max(0.1, deadline - time.monotonic()))
        except queue.Empty:
            print("acc-record: turn timed out", file=sys.stderr)
            break
        if line is None:
            break
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        events.append(line)
        kind = event.get("type")
        if kind == "permission_requested":
            option = answers.pop(0) if answers and scripted(event) else pick_option(event)
            print(f"acc-record: answer {event.get('title')!r} -> {option}", file=sys.stderr)
            send(json.dumps({
                "type": "answer_permission",
                "request_id": event["request_id"],
                "option_id": option,
            }))
        elif kind in ("item_started", "item_updated") and acc_ui and ui_answers:
            if acc_ui.answer(event, ui_answers[0]):
                ui_answers.pop(0)
        elif kind == "turn_completed":
            if not next_prompt():
                break
            deadline = time.monotonic() + args.turn_timeout


if __name__ == "__main__":
    main()
