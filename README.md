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
| `crates/acc-ui` | The render model (`acc-ui/1` components), validation and flattening, component fences, optional formatting conventions. The session model (ACC events in, regions out) is next. | No |
| `crates/acc-ui-agent` | Opt-in: agents writing components themselves through the `acc_ui` MCP server or a spec skill | Yes, when enabled |
| `crates/acc-ui-ffi` | UniFFI bindings for Swift and Kotlin, covering both | No |

Renderers (the `AccUI` Swift package first) take the normalised render model and send
actions out through a delegate. Clients such as Sessh wire it up.

## The render model

The components are defined in [`crates/acc-ui/src/spec.rs`](crates/acc-ui/src/spec.rs):
- layout: `stack`, `row`, `card`
- data: `kv`, `table`
- status: `badge`, `progress`, `checklist`, `callout`
- links: `file`, `pr`, `commit`
- content: `chart`, `mermaid`, `image`, `markdown`, `html`
- inputs: `button`, `chips`, `form`, with actions

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
`<name>.out.json`.

Bindings:

```sh
scripts/swift-bindings.sh OUT_DIR    # Swift sources, for apps linking acc-ui-ffi into their own Rust lib
scripts/kotlin-bindings.sh OUT_DIR
scripts/build-xcframework.sh         # macOS: bindings/swift with AccUIFFI.xcframework + AccUICore
```

Sessh already links one Rust static library (acc-ffi), so it should depend on
`crates/acc-ui-ffi` from that library rather than adding the xcframework. Two Rust
static libraries in one binary duplicate std.
