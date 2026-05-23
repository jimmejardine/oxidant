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

## Collapsible line items

Every text-bearing "line item" in the transcript — user message text blocks, assistant message text blocks, tool-result bodies, thinking blocks — is rendered inside a collapsible container so the conversation stays scannable when individual messages run long.

Behaviour:
- **Summary line**: the collapsed view shows the first sentence (or the first ~120 characters, whichever ends sooner) of the block, with a trailing `…` when content was truncated. Sentence boundary = first `.`, `!`, `?`, or newline followed by whitespace or end-of-string.
- **Default state**: any block whose full text exceeds a single line collapses by default. Blocks that already fit (short user messages, single-line tool results) render as-is with no collapse affordance — there is nothing to hide.
- **Expanded state**: clicking the header swaps the summary for the full content (markdown for user/assistant, code for tool-result JSON). The expansion persists for the lifetime of the panel.
- **Streaming**: the assistant's live turn never collapses while a token stream is in flight. It snaps to "expanded" until the turn finishes; the next render decides whether the now-final block should collapse.
- **Tool-use cards** keep their existing `CollapsingHeader` (collapsed by default) — the rule above applies to their *body* once expanded.

Why a sentence rather than a fixed-line count: a wrapped paragraph's line count depends on the panel's width, so collapsed height would jitter as the user resizes the dock. A sentence-based summary stays stable.

## Selection and copy

Standard egui text selection within text blocks. Multi-message selection deferred to v2.
