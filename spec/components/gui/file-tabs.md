---
id: file-tabs
kind: component
parent: overview
order: 7
implements: []
depends_on:
  - components/gui/dock-layout
code:
  - crates/oxidant-gui/src/panels/file_tab.rs
status: active
responsibility: |
  Render an opened code or spec file as a centre-area dock tab with syntax highlighting and live diagnostic markers.
---

Opened files dock as siblings of the [[components/gui/transcript-tab]] in the centre tab group.

## File sources

- **Code**: `crates/**/*.rs`, `Cargo.toml`, etc. Read from disk; live-reloaded on `notify` events. Read-only in MVP; edit happens via the agent (`apply_edits` / `edit_string`).
- **Spec**: `spec/**/*.md`. Same read-only treatment; edits via the agent.

## Render

- Code: `syntect`-tokenised via the `egui_syntax_highlight` (or in-house) helper. Line numbers in a gutter. Diagnostic markers (red squiggle / yellow underline) overlaid from [[components/gui/diagnostic-panel]] data.
- Spec markdown: rendered via `egui_commonmark`. A toggle switches to raw view for cases where the user wants to read the source.

## Navigation actions

- Click on a diagnostic squiggle → jumps to the matching diagnostic in the right panel.
- Click on a `[[ref]]` in spec markdown → opens the referenced spec as another tab.
- Click on a `code:` link in spec markdown → opens that code file.

## Tab title

`<filename>` with a unicode marker `●` when there's an unread diagnostic on this file. Path on hover.

## Read-only justification

Letting the user hand-edit while the agent is also editing creates contention with `expected_text` checks and obscures who changed what. v2 may add a "manual edit mode" with a soft lock on the agent.
