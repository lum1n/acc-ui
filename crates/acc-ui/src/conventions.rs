//! What to tell the agent, if anything: markdown conventions the translator
//! renders as native UI.
//!
//! Everything here is ordinary markdown that reads fine in a terminal, on
//! GitHub or in a log. Nothing is meaningful only to acc-ui, so output
//! written this way never breaks where acc-ui isn't rendering it. Sending
//! either text is optional: acc-ui renders what agents already write, and
//! these only nudge them toward what renders best.
//!
//! - [`conventions`]: a few lines for the session's system prompt, every turn.
//! - [`conventions_skill`]: a skill with the details and examples, loaded
//!   only when the agent decides it's relevant.
//!
//! Wiring them into an ACC session: pass [`with_conventions`] of the app's
//! own instructions as `LaunchRequest.system_prompt` (ACC appends it to the
//! harness's system prompt on every harness and keeps it for relaunches),
//! and install the skill as [`CONVENTIONS_SKILL`] with `acc-loadout`'s skill
//! store, so every harness on the host can load it.

/// GitHub alert kinds rendered as callouts: `> [!WARNING]`.
pub const ALERT_KINDS: &[&str] = &["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"];

/// The name of the skill [`conventions_skill`] holds, for its directory in
/// a skill store (`~/.agents/skills/acc-ui-formatting/SKILL.md`).
pub const CONVENTIONS_SKILL: &str = "acc-ui-formatting";

/// About 90 tokens for the system prompt.
pub fn conventions() -> String {
    "Your replies are shown in an app that renders markdown as native UI. Where it helps the \
reader: markdown tables for comparisons, `- [ ]`/`- [x]` task lists for steps, ```mermaid for \
flows, a ```chart block of CSV (header row: the x column, then one column per series) for \
numbers, `> [!NOTE]` or `> [!WARNING]` for callouts, files as `path:line`, and PRs or commits \
as full URLs. Keep ordinary prose as prose."
        .to_owned()
}

/// A session's system prompt with the conventions in it: the app's own
/// instructions, a blank line, then [`conventions`]. Text that already
/// holds them comes back unchanged (trimmed), so a relaunch that passes the
/// stored prompt through again doesn't repeat them.
pub fn with_conventions(system_prompt: Option<&str>) -> String {
    let own = system_prompt.map(str::trim).unwrap_or_default();
    if has_conventions(own) {
        return own.to_owned();
    }
    let conventions = conventions();
    if own.is_empty() {
        conventions
    } else {
        format!("{own}\n\n{conventions}")
    }
}

/// Whether a system prompt holds [`conventions`], as a session's stored
/// `spec.system_prompt` tells after [`with_conventions`].
pub fn has_conventions(system_prompt: &str) -> bool {
    system_prompt.contains(&conventions())
}

/// The `SKILL.md` of the `acc-ui-formatting` skill.
pub fn conventions_skill() -> String {
    SKILL.to_owned()
}

const SKILL: &str = r#"---
name: acc-ui-formatting
description: How to format replies so the user's app renders them as native UI (tables, checklists, diagrams, charts, callouts, file and PR links). Use when a reply has structured content such as a comparison, steps, a flow, numbers to plot, or a warning.
---

# Formatting for the user's app

The app renders plain markdown as native components. Use these where they help the reader,
and keep ordinary prose as prose. Everything here is standard markdown, so it reads fine
anywhere else too.

| Write | Shown as |
|---|---|
| A markdown table | A native table |
| `- [ ] step` / `- [x] step` | A checklist |
| A ```` ```mermaid ```` block | A diagram |
| A ```` ```chart ```` block of CSV | A chart |
| `> [!NOTE]`, `> [!TIP]`, `> [!IMPORTANT]`, `> [!WARNING]`, `> [!CAUTION]` | A callout |
| `src/main.rs:42` | A file link that opens the file |
| A PR, issue or commit URL | A card with its number or short SHA |

## Tables

Use them for comparisons and lists of records, not for prose. Put the thing being compared
in the first column. Keep cells short.

| Option | Speed | Cost |
|---|---|---|
| Redis | fast | $$ |
| SQLite | ok | free |

## Checklists

For steps in a plan or the state of work. `[x]` is done, `[ ]` is not.

- [x] Reproduce the bug
- [x] Add a failing test
- [ ] Fix the parser

## Diagrams

Mermaid flowcharts, sequence, state, class and ER diagrams. Keep labels short.

```mermaid
flowchart LR
  client --> api --> db
```

## Charts

A ```` ```chart ```` block holds CSV. The header row names the x column, then one column per
series. Optional `key: value` lines go first: `kind` (line, bar, area, point, pie), `title`,
`x`, `y` (axis labels), `stacked: true`. Without `kind`, text x values make a bar chart and
numbers make a line chart. A pie uses the first series.

```chart
kind: line
title: p95 latency (ms)
day,before,after
Mon,120,80
Tue,135,78
Wed,128,75
```

Use a chart when the shape of the numbers matters. When the exact values matter, use a table.

## Callouts

```markdown
> [!WARNING]
> This drops the production table.
```

Use them sparingly, for what the user must not miss.

## Links

- Files: `path:line` (`src/server.rs:118`), relative to the working directory or absolute.
- PRs, issues and commits: the full URL (`https://github.com/owner/repo/pull/128`), not
  just `#128`.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_conventions_appends_once() {
        let c = conventions();
        assert_eq!(with_conventions(None), c);
        assert_eq!(with_conventions(Some("  ")), c);
        let p = with_conventions(Some("Be brief.\n"));
        assert_eq!(p, format!("Be brief.\n\n{c}"));
        assert!(has_conventions(&p));
        assert_eq!(with_conventions(Some(&p)), p);
        assert!(!has_conventions("Be brief."));
    }

    #[test]
    fn skill_name_matches_front_matter() {
        assert!(conventions_skill().contains(&format!("\nname: {CONVENTIONS_SKILL}\n")));
    }
}
