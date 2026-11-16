# acc-ui

Agent sessions as native UI, not a scrolling transcript. ACC turns each harness's
output into typed events; acc-ui folds them into a live session model (what the
agent is doing now, its plan, what needs you, files touched, commands, subagents,
turn digests) that apps lay out natively. Content inside it (messages, tool output,
diffs) becomes components: cards, tables, checklists, diagrams, charts, callouts,
file/PR/commit links. The full transcript stays available as one view among others.

acc-ui only reads what the agent already produced, so it costs the agent's context
nothing.

| Crate | What it does | Costs the agent context? |
|---|---|---|
| `crates/acc-ui` | The render model (`acc-ui/1` components), validation and flattening, component fences, optional formatting conventions. The session model: ACC events in, regions out. | No |
| `crates/acc-ui-agent` | Opt-in: agents writing components themselves through the `acc_ui` MCP server or a spec skill | Yes, when enabled |
| `crates/acc-ui-ffi` | UniFFI bindings for Swift and Kotlin, covering both | No |

acc-ui is headless: it ships data, behaviour and semantics, never views or styling. How
the agent view looks is up to each app. Apps read the session regions and the
normalised components, lay them out and style them in their own toolkit (SwiftUI,
GPUI, Compose, the web), and send actions back. Anything that isn't visual (labels,
ordering, truncation, relative paths, what counts as failed, what changed) belongs in
acc-ui, so every app gets the same behaviour while drawing it its own way.

## The session model

`SessionView` folds a session's ACC events into the regions a session screen shows,
instead of a top-to-bottom transcript
([`crates/acc-ui/src/session`](crates/acc-ui/src/session/mod.rs)):

| Region | What it holds |
|---|---|
| `header()` | harness, model, title, run state, latest usage and quota, root, turn count |
| `now()` | what the agent and its subagents are doing this second (`Running cargo test`, `Editing src/a.rs`) |
| `plan()` | the to-do list with completed/total and the current entry |
| `asks()` | approvals and questions waiting on the user: options with what each does, the tool call to approve, every question (choices with descriptions, multi-select, free text, secret, required) and, for a plan approval, the plan |
| `files()` | every file read or changed: net change, +/-, each edit's hunks, the turns that touched it |
| `commands()` | each command unwrapped from its `zsh -lc '...'`, with a one-line headline, exit code, outcome (succeeded, failed, or unknown when the harness gives no exit code) and an output tail. Commands are recognised conservatively: a kind (test, build, lint, format, install, git, search, list, read, or other when any part is unclear) and tool (`cargo test`, `pytest`), test counts and compiler/linter errors from the tool's own summary, and files shown with `cat`/`head`/`sed -n`, which land in `files()`. Failed tests behind `\| tail` fail the command, and a passing summary decides it when there is no exit code. Claude Code reports no exit codes but marks non-zero exits as failed, so a completed Claude command that is one simple command (no pipe, `;`, `&&`, `||`) succeeded. `outcome_inferred` marks outcomes decided either way |
| `subagents()` | each run of another agent: task, status, report, and how much it did |
| `turns()` | each prompt with the agent's answer (its last message), a digest as structured parts (one chip each) and as a line (`Edited 3 files (+19 −3) · ran 3 commands · 12 tests passed`), outcome, duration, the items it led to and how many fold away, and the turn's diff where the harness sends one |

```rust
let mut view = SessionView::new();
view.set_root("/home/me/project");          // the client knows the session's cwd
for line in events { let changes = view.apply_json(line)?; /* redraw what changed */ }
view.answered(request_id);                  // after answering an ask

let mut draft = AnswerDraft::new(view.ask(request_id).unwrap()); // a question's answers
draft.pick(0, 1); draft.set_text(1, "acme");
if let Some(option_id) = draft.option_id() { /* send AnswerPermission { request_id, option_id } */ }

let view = SessionView::from_transcript(&saved)?; // an ACC session snapshot or transcript
```

`apply_json` takes ACC's event JSON (a line of `acc run --json`) and returns the
`Changes` it made; `file(path)`, `command(id)`, `subagent(id)`, `turn(index)` and
`ask(request_id)` fetch just what changed. `from_transcript` rebuilds the same state from what ACC saved, for
history and for sessions picked up without their events. acc-ui reads ACC's events with its own tolerant types, so unknown
events and fields are ignored rather than failing.

The recorded sessions in `crates/acc-ui/tests/sessions` (Claude Code and Codex fixing
a bug, then delegating a review to a subagent) replay into golden
`<name>.session.json` files. Record new ones with `scripts/record-fixture.py` against
a built `acc`.

### Replay

```sh
cargo run -p acc-ui --example replay -- crates/acc-ui/tests/sessions/claude-fix-and-review.ndjson \
    --root /work --html /tmp/replay.html
```

Plays a recorded session back through `SessionView` in a throwaway web viewer. Each frame
carries only the regions its event changed. Space plays and pauses, the arrow keys step,
and `?frame=N` or `?frame=end` opens at a frame. Without `--html`, the frames go to stdout
as NDJSON. The viewer is a development tool for looking at the session model on real
sessions. Its look is not part of acc-ui.

## The render model

The components are defined in [`crates/acc-ui/src/spec.rs`](crates/acc-ui/src/spec.rs):
- layout: `stack`, `row`, `card`
- data: `kv`, `table`
- status: `badge`, `progress`, `checklist`, `callout`
- links: `file`, `pr`, `commit`
- content: `chart`, `mermaid`, `image`, `markdown`, `html`, `code`, `disclosure`
- inputs: `button`, `chips`, `form`, with actions
- made only by acc-ui: `output` (a command's output), `diff` (file changes), `request`
  (a harness approval or question) and the `answer` action. These are left out of the
  schema agents see, and `normalize` drops `request` and `answer` from any spec that
  didn't come from acc-ui, so an agent can't dress up a prompt that answers a real
  request.

The nodes are semantic (a table, a checklist, a warning callout, a diff); how they look is
up to the app.

The JSON Schema ([`schema.json`](crates/acc-ui/tests/snapshots/schema.json)) is generated
from the same types.

`normalize` validates a tree and flattens it into what renderers take:

```json
{ "v": "acc-ui/1", "id": "deploy", "title": "Deploy", "fallback": "Deploy\napi-server\n...",
  "interactive": true, "root": 0,
  "nodes": [
    { "type": "card", "title": "api-server", "children": [1, 2] },
    { "type": "progress", "value": 0.6, "max": 1.0, "label": "Rolling out" },
    { "type": "button", "label": "Roll back", "action": { "type": "submit", "values": { "choice": "rollback" } } }
  ] }
```

- Nodes are in pre-order: the root is 0, and children always come after their parent.
- `fallback` is always set; it is generated from the nodes when none is given.
- `interactive` says whether any node takes input.
- A broken node is dropped with an error, and the rest still renders. Common
  shorthands are rewritten.
- Limits: 400 nodes, 12 levels, 256 KB, 500 table rows, 5000 chart points, 128 KB of
  HTML.

`parse_fence(info, body)` turns these fences into components: ` ```mermaid `,
` ```chart ` (CSV, or one chart node as JSON), ` ```acc-ui ` (a spec) and
` ```html render `. A plain ` ```html ` stays code.

## Content: agent text and items as components

- `markdown_blocks(text)`: an agent message as a `stack` of blocks. GFM tables become
  `table` (cells keep inline markdown), all-task lists `checklist`, `> [!NOTE]`-style
  alerts `callout`, closed component fences their component, other fences `code`, and
  the rest stays `markdown` as written. A component fence still streaming stays `code`
  until it closes, so re-parse the whole message on each delta.
- `detect_links(text, root)`: files (`[label](path)`, `path:line`, absolute paths under
  the root), GitHub PRs and commits, and other URLs, as byte ranges with what they
  point at.
- `SessionView::item_spec(id)`: any item as a spec, for a transcript view or an expanded
  step: messages as blocks; reasoning, commands (with `output`), file changes (with
  `diff`), tool calls and subagent runs as a `disclosure` whose summary says what
  happened (`$ python3 -m unittest`, `Edited inventory.py +4 −3`).
- `SessionView::ask_spec(ask)`: a pending approval or question as a `request` whose
  options carry `answer` actions, with its questions and the plan to approve.

Questions are answered through an `AnswerDraft`: it knows that picking a single-choice
option replaces the earlier pick, that multi-choice picks toggle, which questions take
typed text and which are still missing, and it encodes the answers as the option id
every ACC driver decodes. Apps only draw it and send ACC's `AnswerPermission`.

## Conventions: optional, plain markdown

Agents don't need to know about acc-ui. If you want to nudge them toward output that
renders best, there are two texts. Both teach plain markdown that reads fine anywhere:

- `conventions()`: about 90 tokens for the system prompt, sent every turn.
  ([snapshot](crates/acc-ui/tests/snapshots/conventions.txt))
- `conventions_skill()`: the `acc-ui-formatting` skill, with details and examples,
  loaded only when the agent finds it relevant.
  ([snapshot](crates/acc-ui/tests/snapshots/SKILL.md))

Formatting matters on every reply, and a skill only loads when the model thinks to
load it. So the short text does the steady work and the skill covers the details.

## Opt-in: agents writing components (`acc-ui-agent`)

For clients that want agents to make components on purpose and accept the context
cost:

- The `acc-ui-components` skill and instructions teach the spec.
- The `acc_ui` MCP server is POSIX `sh` plus `awk`, so nothing is installed on hosts.
  - `acc_ui_render(spec)` returns at once.
  - `acc_ui_ask(spec)` waits for `~/.acc-ui/<session>/<spec id>/answer.json`, which
    the client writes with `answer_script`.
- `setup_script()` installs the server under a content-hashed path, and
  `mcp_server(path, session)` gives the launch command for ACC.
- `recognize_tool` finds the calls under every harness's naming. `submit_prompt` and
  `parse_submit` handle answers sent as a user message:
  `<acc-ui-submit id="deploy">{"choice":"rollback"}</acc-ui-submit>`.

The full tool schema is about 12.8 KB and is sent with both tools.

## Development

```sh
cargo test                       # golden specs, snapshots, the MCP server under the local sh
UPDATE_GOLDEN=1 cargo test       # rewrite golden files and snapshots; review the diff
scripts/test-shells.sh           # the MCP server under dash+mawk and busybox too (Docker)
cargo run -p acc-ui-agent --example dump -- conventions|conventions-skill|schema|skill|instructions|server|setup|tools
```

The golden specs are in `crates/acc-ui/tests/specs`: `<name>.json` normalises to
`<name>.out.json`. Recorded sessions are in `crates/acc-ui/tests/sessions`:
`<name>.ndjson` replays into `<name>.session.json`.

Bindings:

```sh
scripts/swift-bindings.sh OUT_DIR    # Swift sources, for apps linking acc-ui-ffi into their own Rust lib
scripts/kotlin-bindings.sh OUT_DIR
scripts/build-xcframework.sh         # macOS: bindings/swift with AccUIFFI.xcframework + AccUICore
```

In Swift and Kotlin the session model is a `SessionView` object with the same methods
(`applyJson`, `snapshot`, the region getters, `itemSpec`, `askSpec`, `answerDraft`)
returning typed records, an `AnswerDraft` object, and the free functions
`markdownBlocks`, `detectLinks` and `encodeAnswers`. Links carry their
range as UTF-8 bytes and as UTF-16 units, for `NSRange` and Kotlin strings.

Sessh already links one Rust static library (acc-ffi), so it should depend on
`crates/acc-ui-ffi` from that library rather than adding the xcframework. Two Rust
static libraries in one binary duplicate std.
