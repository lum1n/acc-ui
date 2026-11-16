#!/usr/bin/env python3
"""Record an ACC session as NDJSON events, for acc-ui's session fixtures.

Runs `acc run <harness> <cwd> --json`, sends each prompt once the previous
turn completes, answers every permission request with its first allowing
option (or the first option), and writes the events it printed to OUT.

    scripts/record-fixture.py --acc ../acc/target/release/acc \\
        claude /tmp/scratch crates/acc-ui/tests/sessions/claude-fix.ndjson \\
        "first prompt" "second prompt"

Afterwards, scrub OUT: home paths become `/home/user`, the scratch path
becomes `/work`.
"""

import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time

ALLOW = ("allow_once", "allow_always")


def pick_option(event):
    options = event.get("options") or []
    # Keep the user's own MCP servers (notes, trackers) out of fixtures.
    if "mcp" in (event.get("title") or "").lower() or (
        (event.get("detail") or {}).get("kind") == "mcp"
    ):
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--acc", default="acc")
    ap.add_argument("--turn-timeout", type=float, default=900)
    ap.add_argument("--extra", action="append", default=[], help="extra arg for acc run")
    ap.add_argument("harness")
    ap.add_argument("cwd")
    ap.add_argument("out")
    ap.add_argument("prompts", nargs="+")
    args = ap.parse_args()

    cmd = [args.acc, "run", args.harness, args.cwd, "--json", *args.extra]
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

    events = []
    prompts = list(args.prompts)
    send(prompts.pop(0))
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
            option = pick_option(event)
            print(f"acc-record: answer {event.get('title')!r} -> {option}", file=sys.stderr)
            send(json.dumps({
                "type": "answer_permission",
                "request_id": event["request_id"],
                "option_id": option,
            }))
        elif kind == "turn_completed":
            if not prompts:
                break
            send(prompts.pop(0))
            deadline = time.monotonic() + args.turn_timeout

    proc.stdin.close()
    try:
        proc.wait(timeout=20)
    except subprocess.TimeoutExpired:
        proc.kill()

    with open(args.out, "w") as f:
        f.write("\n".join(events) + "\n")
    print(f"acc-record: {len(events)} events -> {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
