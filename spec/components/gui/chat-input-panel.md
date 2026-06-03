---
id: chat-input-panel
kind: component
parent: overview
order: 4
implements: []
depends_on:
  - components/core/agent-loop
code:
  - crates/oxidant-gui/src/panels/chat_input.rs
status: active
responsibility: |
  Bottom-docked multi-line text input panel with send, cancel, and per-exploration model picker.
---

## Layout

```
┌────────────────────────────────────────────────────┐
│ [PLAN] model: claude-opus-4-7 ▼   [Cancel] [Send ⏎] │
│ ┌────────────────────────────────────────────────┐ │
│ │ <multi-line text area, expandable>             │ │
│ └────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────┘
```

## Keybindings

- Enter: insert newline by default; Ctrl+Enter sends. (Configurable: swap to Enter-sends in [[components/config/settings]].)
- Esc: cancel the in-flight agent turn if any.
- **Shift+Tab** (while the text edit owns focus): flip the agent between Plan and Implement mode. See [[components/core/agent-mode]]. The header chip updates on the same frame; the next Send uses the new mode. The handler MUST consume the key event (`ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab))`) so Tab focus-traversal and tab-character insertion are suppressed only when Shift is held.

## Mode chip

The leftmost element of the header row is a coloured chip showing the current `AgentMode`:

- `[PLAN]`     — yellow (matches the spec-tree's `Draft` colour band).
- `[IMPLEMENT]` — green (matches `kind: component`).

Hovering shows the tooltip "**Shift+Tab** to toggle." Clicking the chip toggles the mode too — mouse-only users get a path. The chip is read-only while a turn is streaming (it visually dims but its label still reflects the mode the in-flight request was sent with, not the in-progress toggle the user might want for the next turn).
- Up arrow on empty input: cycle prior user messages (per-exploration history).

## Send semantics

- Build a `User` message containing the text.
- Append to the exploration's conversation.
- Trigger the agent loop ([[components/core/agent-loop]]) for that exploration via its task handle.
- Disable the Send button and show a spinner until the loop emits `Finish`.

## Cancel semantics

- Calls `exploration.cancellation.cancel()`.
- The agent loop short-circuits at the next yield. In-flight tool calls return cancellation results.
- Cancellation is recorded as a system message in the transcript.

## Model picker

Per-exploration override of the default model. Choices come from [[components/config/settings]]. Switching mid-conversation is allowed; the next turn uses the new model.
