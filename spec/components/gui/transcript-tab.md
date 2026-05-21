---
id: transcript-tab
kind: component
parent: overview
order: 3
implements: []
depends_on:
  - components/core/conversation
  - components/gui/dock-layout
code:
  - crates/oxidant-gui/src/panels/transcript.rs
status: active
responsibility: |
  Render an exploration's conversation as a scrollable centre tab with markdown, tool-call cards, and streaming token updates.
---

The home centre tab in every exploration window.

## Render structure

A vertical scroll area, scroll-anchored at the bottom (auto-follow during streaming, releases when the user scrolls up).

Each turn:
- **User**: text with markdown rendering.
- **Assistant**: streaming text, thinking block (collapsed by default), and tool-use blocks rendered as expandable cards.
- **Tool result**: a card under its tool-use header, with structured JSON (`json` syntect highlight) or a custom renderer per tool type (e.g. diff view for `apply_edits` results).

## Streaming integration

The viewport holds an `mpsc::UnboundedReceiver<ChatEvent>` per exploration. Each frame: drain the receiver, apply to conversation state, call `ctx.request_repaint()` if any event was received.

While streaming, the assistant's last text block is appended to in place — no list churn.

## Markdown via egui_commonmark

`egui_commonmark::CommonMarkViewer` instance per exploration (state-bearing for image caches). Code fences render with syntect.

## Tool-call card actions

Each card has:
- Copy-as-JSON button
- "Open in editor" — opens the tool's primary file (e.g. for `fs_read`) as a new centre tab via [[components/gui/file-tabs]]
- "Re-run" — re-issues the same tool call (with confirmation if `Mutating`)

## Selection and copy

Standard egui text selection within text blocks. Multi-message selection deferred to v2.
