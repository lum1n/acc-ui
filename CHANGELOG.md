# Changelog

All notable changes to acc-ui are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) as described under Stability in the
README (before 1.0, a minor version may break the Rust and FFI API).

## [Unreleased]

## [0.1.0] - 2026-11-16

The first release: a headless library that turns ACC's events into a session
model and agent output into `acc-ui/1` components, with Swift and Kotlin
bindings.

### Added

- `acc-ui/1` component spec (layout, data, status, links, content, inputs),
  its JSON Schema, and `normalize`: validation and flattening into pre-order
  nodes with a generated plain-text `fallback`, size limits, and shorthands
  rewritten. Component fences: ` ```mermaid `, ` ```chart ` (CSV or JSON),
  ` ```acc-ui ` and ` ```html render `; a plain ` ```html ` stays code.
- Session model (`SessionView`): ACC event JSON in, regions out (header, now,
  plan, asks, files, commands, subagents, turns), with the `Changes` each
  event made and restore from an ACC transcript or snapshot (#1).
- Turns: the agent's answer, digests as parts and as a line, outcomes and
  durations (#2).
- Replay prototype: recorded Claude Code and Codex sessions played back
  through the session model, with golden session files and a throwaway web
  viewer (`cargo run -p acc-ui --example replay`) (#3).
- Content: agent markdown as component blocks (tables, checklists, alerts,
  closed component fences), link detection (files, GitHub PRs and commits,
  URLs), and specs for any item and pending ask (#5).
- Presentation logic in Rust: command headlines and outcomes, file display
  paths, turn steps and digest parts; `SessionView` over FFI as an object
  with typed records (#7).
- Interactive asks: questions with choices, multi-select, free text and
  secrets, plan approvals, and `AnswerDraft`, which encodes answers as the
  option id every ACC driver decodes (#8).
- Command recognisers: test runs, builds, lints, installs, git, reads and
  searches, with test counts and diagnostics from the tool's own summary;
  completed simple Claude Code commands count as succeeded (#9).
- Formatting conventions for ACC's system prompt (`conventions`,
  `with_conventions`, `has_conventions`) and the `acc-ui-formatting` skill (#10).
- Images by reference: prompt attachments on items and turns, image tools by
  path, and `image` nodes with `sha256:<hex>` sources (#11).
- `acc-ui-agent` (opt-in): the `acc_ui` MCP server in POSIX `sh` and `awk`
  (`acc_ui_render`, `acc_ui_ask`), its setup and answer scripts, submit
  prompts, and finding `acc_ui` calls in a `SessionView` with where their
  answer stands; a slimmer tool list (#12).
- `acc-ui-ffi`: UniFFI bindings for Swift and Kotlin, an xcframework build for
  iOS and macOS, and a root `Package.swift`.
- CI (formatting, clippy, tests, MSRV, `cargo audit`, the MCP server under
  dash and busybox, Swift and Kotlin smoke programs) and a release workflow.

[Unreleased]: https://github.com/lum1n/acc-ui/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/lum1n/acc-ui/releases/tag/v0.1.0
