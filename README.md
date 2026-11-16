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

## Install

acc-ui isn't on crates.io. Releases are tags, with prebuilt Apple binaries on
[GitHub Releases](https://github.com/lum1n/acc-ui/releases); see
[CHANGELOG.md](CHANGELOG.md) for what each one holds.

Rust, as a dependency pinned to a tag:

```toml
[dependencies]
acc-ui = { git = "https://github.com/lum1n/acc-ui", tag = "v0.1.0" }
acc-ui-agent = { git = "https://github.com/lum1n/acc-ui", tag = "v0.1.0" } # opt-in
acc-ui-ffi = { git = "https://github.com/lum1n/acc-ui", tag = "v0.1.0" }   # to link the bindings into your own Rust lib
```

Swift (iOS 16, macOS 13), with SwiftPM. The root `Package.swift` pulls the prebuilt
`AccUIFFI.xcframework` from the release and builds the `AccUICore` wrapper over it:

```swift
.package(url: "https://github.com/lum1n/acc-ui", from: "0.1.0"),
// in your target's dependencies:
.product(name: "AccUICore", package: "acc-ui"),
```

Then `import AccUICore`. Apps that already link a Rust static library should depend
on `acc-ui-ffi` from it and generate the bindings with `scripts/swift-bindings.sh`
instead (see [Development](#development)).

Kotlin: each release has `acc-ui-kotlin-<tag>.tar.gz`, the generated bindings
(package `io.github.lum1n.accui`). Add `io/github/lum1n/accui/acc_ui_ffi.kt` to your
sources, depend on [JNA](https://github.com/java-native-access/jna)
(`net.java.dev.jna:jna`, `@aar` on Android), and ship `libacc_ui_ffi` built for your
targets: `cargo build -p acc-ui-ffi --release` for the JVM (put it on
`jna.library.path`), or [cargo-ndk](https://github.com/bbqsrc/cargo-ndk) for Android.
Releases don't include native libraries for Kotlin yet.

## Stability

acc-ui is pre-1.0.

- **The `acc-ui/1` spec** only grows: new optional fields and new node types. Every
  normalised spec carries a plain-text `fallback`, so a renderer that doesn't know a
  new node type can show that instead of failing. A change that would break an
  existing renderer or spec becomes `acc-ui/2`, and the `v` field says which one a
  spec is.
- **The Rust and FFI API** follows semver as Cargo reads it before 1.0: a patch
  release (0.1.x) doesn't break anything, a minor release (0.2.0) may, and the
  changelog says how. The Swift and Kotlin bindings are generated from the Rust API
  and move with it; take a library and its bindings from the same release.
- **Fences**: only ` ```html render ` renders HTML. A plain ` ```html ` fence stays a
  code block, so showing HTML source never turns into rendering it.

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
| `turns()` | each prompt (and its images) with the agent's answer (its last message), a digest as structured parts (one chip each) and as a line (`Edited 3 files (+19 −3) · ran 3 commands · 12 tests passed`), outcome, duration, the items it led to and how many fold away, and the turn's diff where the harness sends one |

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
history and for sessions picked up without their events. Without a snapshot's spec it
infers the harness only where the items leave no doubt (Claude Code's `toolu_` calls to
its own tools, Codex's `commandExecution` items). acc-ui reads ACC's events with its own tolerant types, so unknown
events and fields are ignored rather than failing.

Images: ACC keeps image bytes out of its events and sends references instead (media
type, size, SHA-256, or a path on the host), in a user message's `native.images`.
acc-ui puts them on the item (`images`) and the turn (`prompt_images`), drops ACC's
`[image]` stand-in for an image-only prompt, and gives Codex's `imageView` and
`imageGeneration` items the image's path. In item specs they become `image` nodes whose
`src` is the path or `sha256:<hex>`: the app shows an image the user sent from its own
copy, matched by hash. With an ACC that doesn't send references, the `[image]` text
stays. A restored transcript has no references; that waits for a media store in ACC.

The recorded sessions in `crates/acc-ui/tests/sessions` (Claude Code and Codex fixing
a bug, then delegating a review to a subagent; and one feature each: questions, a plan
approval, test runners and type checkers, image prompts, Cursor and Copilot over ACP,
the `acc_ui` server) replay into golden `<name>.session.json` files. Record new ones
with `scripts/record-fixture.py` against a built `acc`; it can answer questions and plan
approvals (`--answer`), attach images (`--image`), and give Claude the `acc_ui` server
(`--acc-ui`, `--ui-answer`).

### Replay

```sh
cargo run -p acc-ui --example replay -- crates/acc-ui/tests/sessions/claude-fix-and-review.ndjson \
    --root /work --html /tmp/replay.html
```

Plays a recorded session back through `SessionView` in a throwaway web viewer. Each frame
carries only the regions its event changed. Space plays and pauses, the arrow keys step,
and `?frame=N` or `?frame=end` opens at a frame. Without `--html`, the frames go to stdout
as NDJSON. Frames also carry the agent's own `acc_ui` calls with their specs (the
example alone uses `acc-ui-agent`). The viewer is a development tool for looking at the session model on real
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
` ```html render `. A plain ` ```html ` fence stays a code block; only ` ```html render `
renders.

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
  happened (`$ python3 -m unittest`, `Edited inventory.py +4 −3`). A file read shows
  the `file` (at its first line) and its content as code in the file's language;
  file searches (Claude's Glob, Grep listing files) show a `file` per match. Reasoning
  without text (Codex with summaries off) has no spec and isn't a step.
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

Wiring them into an ACC session:

- Launch with `system_prompt: with_conventions(own_instructions)`. ACC appends it to
  the harness's system prompt on every harness (Claude and pi `--append-system-prompt`,
  Codex developer instructions, ACP `systemPrompt`) and keeps it for relaunches.
  `with_conventions` leaves text that already holds the conventions alone, and
  `has_conventions(spec.system_prompt)` tells whether a stored session got them.
- Install the skill once per host with `acc-loadout`'s skill store, as
  `CONVENTIONS_SKILL` (`~/.agents/skills/acc-ui-formatting/SKILL.md`), and link it for
  the harnesses you use.

## Opt-in: agents writing components (`acc-ui-agent`)

For clients that want agents to make components on purpose and accept the context
cost:

- The `acc-ui-components` skill and instructions teach the spec.
- The `acc_ui` MCP server is POSIX `sh` plus `awk`, so nothing is installed on hosts.
  - `acc_ui_render(spec)` returns at once.
  - `acc_ui_ask(spec)` waits for `~/.acc-ui/<session>/<spec id>/answer.json`, which
    the client writes with `answer_script`. A spec id asks once per session: asking
    again with an id that waits or was answered is a tool error.
- `setup_script()` installs the server under a content-hashed path, and
  `mcp_server(path, session)` gives the server for ACC's `LaunchRequest.mcp_servers`,
  which adds it to that one session next to the harness's own servers (Claude, Codex
  and ACP agents; pi has no MCP).
- `recognize_tool` finds the calls under every harness's naming. `submit_prompt` and
  `parse_submit` handle answers sent as a user message:
  `<acc-ui-submit id="deploy">{"choice":"rollback"}</acc-ui-submit>`.

In a `SessionView` the calls are ordinary tool items: core doesn't know the agent
layer. `agent_calls(view)`, `agent_call(view, item_id)` and `open_agent_asks(view)`
(methods of `SessionView` in the bindings) find them with their spec and, for asks,
where the answer stands. `acc_ui_agent::item_spec(view, id)` is `item_spec` with a
call's own spec (normalised as untrusted) in place of the raw tool call; the bindings'
`itemSpec` is this one. `AgentCall::answer(session, values)` picks
the delivery: the answer script while the call waits, or a submit prompt for the next
turn once it timed out.

The spec schema (about 13.7 KB) is sent once, with `acc_ui_render`; `acc_ui_ask` takes
the same spec and points there, so the tool list is about 15 KB.

## Known limitations

Deliberate, for now:

- Claude's Edit hunks come with `old_start` 0 (no line numbers), so apps show them as
  snippets rather than located hunks.
- Read detection is conservative: only `cat`, `head`, `tail`, `sed -n` and the like on
  literal paths count. `cat < file`, `awk`, `wc`, `xargs` and `find -exec` aren't reads.
- `make -C dir test` and other commands with unclear parts are `Other`.
- `CommandKind::Git` doesn't say whether the command changed the repository; `tool` holds
  the subcommand (`git commit`).
- `AnswerDraft` doesn't cover declining or cancelling an MCP elicitation; apps show those
  as plain options.
- Secret answers live in memory as plain strings. Apps must not log or persist them.
- The agent's own asks aren't in core's `asks()`; merge `open_agent_asks()` (`openAgentAsks()`
  in the bindings).
- A user message sent mid-turn starts a new turn.

## Development

```sh
cargo test                       # golden specs, snapshots, the MCP server under the local sh
UPDATE_GOLDEN=1 cargo test       # rewrite golden files and snapshots; review the diff
scripts/test-shells.sh           # the MCP server under dash+mawk, busybox and macOS-like bash 3.2 + BWK awk (Docker)
cargo audit                      # advisories; the allow-list is .cargo/audit.toml
cargo run -p acc-ui-agent --example dump -- conventions|conventions-skill|schema|skill|instructions|server|setup|tools
cargo run -p acc-ui-agent --example dump -- answer SESSION ID VALUES_JSON   # the answer script
```

The golden specs are in `crates/acc-ui/tests/specs`: `<name>.json` normalises to
`<name>.out.json`. Recorded sessions are in `crates/acc-ui/tests/sessions`:
`<name>.ndjson` replays into `<name>.session.json`.

Bindings:

```sh
scripts/swift-bindings.sh OUT_DIR    # Swift sources, for apps linking acc-ui-ffi into their own Rust lib
scripts/kotlin-bindings.sh OUT_DIR
scripts/build-xcframework.sh         # macOS: bindings/swift with AccUIFFI.xcframework + AccUICore
scripts/ffi-smoke.sh swift|kotlin    # build and run a small program over the bindings (Kotlin: JNA_JAR=...)
scripts/release.sh swift-sources     # regenerate Sources/AccUICore after an FFI change (CI checks it)
```

Releases: bump `[workspace.package] version` in `Cargo.toml`, add the version's
`CHANGELOG.md` section, merge, and push a `v*` tag from main. The release workflow
([`.github/workflows/release.yml`](.github/workflows/release.yml)) builds the
xcframework on macOS, rewrites the url and checksum in `Package.swift`
(`scripts/release.sh package`), moves the tag onto that commit, and publishes the
xcframework zip, the Kotlin sources and `SHA256SUMS` with the changelog section as
notes.

In Swift and Kotlin the session model is a `SessionView` object with the same methods
(`applyJson`, `snapshot`, the region getters, `itemSpec`, `askSpec`, `answerDraft`)
returning typed records, an `AnswerDraft` object, and the free functions
`markdownBlocks`, `detectLinks` and `encodeAnswers`. Links carry their
range as UTF-8 bytes and as UTF-16 units, for `NSRange` and Kotlin strings.

Sessh already links one Rust static library (acc-ffi), so it should depend on
`crates/acc-ui-ffi` from that library rather than adding the xcframework. Two Rust
static libraries in one binary duplicate std.
