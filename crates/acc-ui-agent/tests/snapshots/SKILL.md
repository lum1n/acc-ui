---
name: acc-ui-components
description: Show the user rich components in the chat (cards, tables, charts, Mermaid diagrams, checklists, progress, file/PR/commit links, buttons and forms) with ```acc-ui fences or the acc_ui_render and acc_ui_ask tools. Use for status, results, comparisons, plans, metrics, and whenever you need the user to choose or fill something in.
---

# acc-ui

The user's app renders components natively. Use them where structure helps the user: a status
summary, a comparison table, a chart of numbers, a diagram of a flow, a checklist of steps, a
choice to make. Keep the prose around a component short and don't repeat what it shows.
Plain answers stay plain text.

## Delivering a component

- **Fence** (always works): a ```` ```acc-ui ```` block holding one spec. Shorthands:
  ```` ```mermaid ```` renders a diagram, ```` ```chart ```` holds one `chart` node, and
  ```` ```html render ```` shows self-contained HTML (plain ```` ```html ```` stays code).
- **`acc_ui_render(spec)`**: shows the spec and returns at once.
- **`acc_ui_ask(spec)`**: shows the spec and waits for the user's answer, returned as JSON.
  Use it when you can't carry on without the answer. The spec needs an `id` unique in the
  session. If the call times out, carry on or end your turn; a later answer arrives as a
  message (see below).

## The spec

```json
{
  "v": "acc-ui/1",
  "id": "deploy-status",
  "title": "Deploy",
  "root": {
    "type": "card",
    "title": "api-server",
    "children": [
      { "type": "kv", "items": [{ "label": "Version", "value": "2.4.1" }, { "label": "Region", "value": "eu-north-1" }] },
      { "type": "progress", "value": 0.6, "label": "Rolling out" },
      { "type": "button", "label": "Roll back", "style": "destructive",
        "action": { "type": "submit", "values": { "choice": "rollback" } } }
    ]
  }
}
```

- `root` is one node; `stack`, `row` and `card` hold `children`.
- `id`: 1-64 of `A-Z a-z 0-9 . _ -`. `fallback`: plain text for places that can't render it.
- Limits: 400 nodes, 12 levels, 256 KB, 500 table rows, 5000 chart points, 128 KB of HTML.
  Broken nodes are dropped and the rest still shows.

## Components

Set `type` to one of these. `?` marks optional fields.

- **`stack`**: Children top to bottom. Fields: `children?` [node]; `gap?` Gap.
- **`row`**: Children side by side. Fields: `children?` [node]; `gap?` Gap; `wrap?` boolean (Wrap onto more lines when the row is too narrow).
- **`card`**: A titled box around its children. Fields: `title?` string; `subtitle?` string; `icon?` string (One emoji shown before the title); `tone?` Tone; `children?` [node]; `action?` action (Runs when the card is tapped).
- **`kv`**: Label/value pairs. Fields: `items` [KvItem].
- **`table`**: Rows and columns of text. Fields: `columns` [Column]; `rows` [[string]] (One array of cells per row, in column order. Cells may hold inline markdown (`code`, **bold**, links)).
- **`badge`**: A small coloured label. Fields: `text` string; `tone?` Tone.
- **`progress`**: A progress bar. Fields: `value` number (Progress so far, from 0 to `max`); `max?` number (Defaults to 1); `label?` string; `tone?` Tone.
- **`checklist`**: Steps with a state each. Fields: `items` [CheckItem].
- **`callout`**: A highlighted note, warning or error. Fields: `text` string (Markdown); `title?` string; `tone?` Tone.
- **`file`**: A file on the host, opened on tap. Fields: `path` string (Absolute path, or relative to the session's working directory); `line?` integer; `label?` string (What the file is, shown under the path).
- **`pr`**: A pull request. Fields: `title` string; `number?` integer; `url?` string; `repo?` string (`owner/name`); `state?` PrState.
- **`commit`**: A commit. Fields: `sha` string; `title?` string (First line of the message); `url?` string; `author?` string.
- **`chart`**: A line, bar, area, point or pie chart. Fields: `kind` ChartKind (A `pie` uses the first series, with each point's `x` as a slice label); `series` [Series]; `title?` string; `x_label?` string; `y_label?` string; `stacked?` boolean (Stack bar and area series instead of overlapping them).
- **`mermaid`**: A Mermaid diagram. Fields: `code` string (Mermaid source: flowchart, sequence, state, class or ER diagram).
- **`image`**: An image. Fields: `src` string (An `https:` URL, a `data:image/...` URL, an absolute path on the host, or `sha256:<hex>` of an image the app holds (one the user sent)); `alt?` string; `caption?` string.
- **`markdown`**: Markdown text. Fields: `text` string.
- **`html`**: Self-contained HTML in a sandbox: no network, no navigation. Fields: `html` string (A full document or a fragment. Inline all CSS, JS and images: the sandbox fetches nothing and opens links only after the user confirms); `height?` integer (Preferred height in points; the content's own height is used if unset).
- **`code`**: Source code or other preformatted text. Fields: `text` string; `language?` string (The fence's language (`rust`, `sh`, `diff`), when known).
- **`disclosure`**: A summary line whose children show when the user opens it. Fields: `summary` string (The line shown while it's closed); `children?` [node]; `open?` boolean (Starts open. Defaults to closed).
- **`button`**: A button that runs an action when tapped. Fields: `label` string; `action` action; `style?` ButtonStyle.
- **`chips`**: Quick-reply chips; each runs its action when tapped. Fields: `options` [Chip].
- **`form`**: Input fields with a submit button. Fields: `fields` [Field]; `submit_label?` string (Defaults to "Submit").

### Parts

- **KvItem**: Fields: `label` string; `value` string; `tone?` Tone.
- **Column**: Fields: `label` string; `align?` Align.
- **CheckItem**: Fields: `text` string; `state` CheckState; `detail?` string (A second line under the text).
- **Series**: Fields: `points` [Point]; `name?` string (Legend label).
- **Point**: Fields: `x` number|string (A number, a category label, or an ISO 8601 date); `y` number.
- **Chip**: Fields: `label` string; `action` action.
- **Field**: A form input. Its value is submitted under `name`. Fields: `name` string (Key of the value in the submitted object: `A-Z a-z 0-9 _ -`); `kind` FieldKind; `label?` string; `help?` string (Help text under the field); `placeholder?` string; `default?` any (Initial value: a string, number or boolean matching `kind`); `required?` boolean; `options?` [Choice] (The choices of a `select`); `min?` number (Lower bound of a `number` or `slider`); `max?` number (Upper bound of a `number` or `slider`); `step?` number; `multiple?` boolean (A `select` that allows several choices; its value is an array).
- **Choice**: Fields: `label` string; `value?` string (Submitted value; defaults to the label).

### Values

- **Tone**: `neutral`, `info`, `success`, `warning`, `danger`, `accent`
- **Gap**: `none`, `small`, `medium`, `large`
- **Align**: `start`, `center`, `end`
- **CheckState**: `todo`, `doing`, `done`, `failed`, `skipped`
- **PrState**: `open`, `draft`, `merged`, `closed`
- **ChartKind**: `line`, `bar`, `area`, `point`, `pie`
- **ButtonStyle**: `primary`, `secondary`, `destructive`
- **FieldKind**: `number`, `text`, `textarea`, `toggle`, `select`, `date`, `slider`

### Actions

An `action` is an object with `type`:

- **`prompt`**: Send `text` as the user's next message. Fields: `text` string.
- **`submit`**: Answer this component: with `values`, merged over any form's values. Answers an `acc_ui_ask`, or comes back as the user's next message. Fields: `values?` object.
- **`open_url`**: Open a web page after the user confirms. Fields: `url` string.
- **`open_file`**: Open a file on the host. Fields: `path` string; `line?` integer.
- **`copy`**: Copy `text` to the clipboard. Fields: `text` string.

Nothing runs until the user taps. `open_url` asks the user first.

## Answers

A form's submit, or a button or chip with a `submit` action, answers the component with an
object of values: the form's fields by `name`, merged with the action's `values`.

- From `acc_ui_ask`, the answer is the tool result.
- Otherwise it arrives as the user's next message:
  `<acc-ui-submit id="deploy-status">{"choice":"rollback"}</acc-ui-submit>`.
  Treat it as the user's reply to that component.

A `prompt` action sends its text as the user's message instead, which suits quick replies:

```json
{ "type": "chips", "options": [
  { "label": "Run the tests", "action": { "type": "prompt", "text": "Run the tests" } },
  { "label": "Open a PR", "action": { "type": "prompt", "text": "Open a PR" } }
] }
```

## Charts and diagrams

```json
{ "type": "chart", "kind": "line", "title": "p95 latency (ms)", "x_label": "day",
  "series": [
    { "name": "before", "points": [{ "x": "Mon", "y": 120 }, { "x": "Tue", "y": 135 }] },
    { "name": "after",  "points": [{ "x": "Mon", "y": 80 },  { "x": "Tue", "y": 78 }] }
  ] }
```

Mermaid: `{ "type": "mermaid", "code": "flowchart LR\n  A --> B" }`, or a ```` ```mermaid ```` fence.
