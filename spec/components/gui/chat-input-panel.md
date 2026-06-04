```yaml
---
id: chat-input-panel
kind: component
parent: overview
order: 4
implements: []
depends_on:
  - components/core/agent-loop
  - chat-input-panel-commands
code:
  - crates/oxidant-gui/src/panels/chat_input.rs
status: active
responsibility: |
  Bottom-docked multi-line text input panel with send, cancel, slash commands,
  model picker, and "continue iterating" flow..
---
```

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
- Raise/focus the [[components/gui/dock-layout]] Transcript tab (push `DockTab::Transcript` onto `SharedState::pending_centre_tabs`, drained into `open_in_centre`) so the streaming response is immediately visible even if the centre was showing a file/preview tab. Regular prompts only — `/clear` and `/compact` don't steal focus.
- Disable the Send button and show a spinner until the loop emits `Finish`.

## Cancel semantics

- ESC (while a turn is streaming) or the **Cancel** button trips the per-turn `CancellationToken` held in `SharedState::cancellation` (`token.cancel()`).
- The agent loop notices cooperatively and short-circuits at its next yield — a stream chunk or a tool boundary (see [[components/core/agent-loop]] "Cancellation"). In-flight tool tasks are aborted.
- The turn ends with `TurnOutcome { cancelled: true, .. }`; the transcript shows a muted `⊘ turn cancelled` line for that turn.

## Model picker

Per-exploration override of the default model. Choices come from [[components/config/settings]]. Switching mid-conversation is allowed; the next turn uses the new model.

## External prompt fill

Other panels can replace the chat input's draft via a side channel on `SharedState`:

```rust
pub pending_chat_prompt: Option<PendingChatPrompt>,

pub struct PendingChatPrompt {
    pub prompt: String,
    pub mode: AgentMode,
}
```

The current caller is the [[components/gui/health-check-panel]] — double-clicking an issue queues a structured prompt with `mode = Plan`. The same channel is open to any future panel that wants to "start a conversation about this".

At the top of `ChatInputPanel::render`, before any drawing, the panel drains the field:

1. Replace `self.draft` with `prompt`.
2. Set `self.mode` to the requested mode.
3. Request focus on the multi-line `TextEdit` via `ui.memory_mut(|m| m.request_focus(text_edit_id))`, where `text_edit_id = ui.make_persistent_id("oxidant-chat-input")` — the same id the TextEdit already uses.
4. Clear `pending_chat_prompt = None`.

**No auto-send.** The panel never triggers the agent on the user's behalf — Ctrl+Enter remains the only path to send. Auto-sending from external context would be surprising; the user retains the final keystroke.

Subsequent fills overwrite: only the most recent `Some(...)` value is honoured, since the panel drains every frame.

## Slash commands

Slash command parsing and execution is specified in [[chat-input-panel-commands]]. The chat input panel drains `/clear`, `/compact`, and unknown commands from the draft before dispatch.

## Continue iterating

When the most recent `TurnOutcome` has `hit_max_iterations == true`, the [[components/gui/transcript-tab]] renders a `▶ Continue iterating (+N)` button beneath the red error label. Clicking it resumes the *same* conversation — **no new user message is appended** — with `max_iterations + N` (`N = 20`). Repeat clicks compound: second click 50, third 70, …

The transcript can't write back to `SharedState` directly (its render is read-only), so the click returns a `TranscriptAction::ContinueIterating { new_max }` that the panel host (`app.rs`) writes into `SharedState.pending_continue: Option<usize>`. The chat input panel drains that field at the top of its next `render` — mirroring how `pending_chat_prompt` is drained — and dispatches a continuation via the shared spawn path with `prompt = None` and `max_iter = new_max`.

The continuation uses the panel's current `self.mode`: if the user flipped PLAN/IMPLEMENT between cap-hit and click, the new mode wins. The base `max_iterations` for fresh prompts is `DEFAULT_TURN_MAX_ITERATIONS` (currently 30), defined as a `const` in the panel for easy tuning.

A turn that fails for any *other* reason — provider error, cancellation, tool panic — does not show the button. The detection is anchored to the verbatim "agent loop exceeded max_iterations" prefix that `agent_loop::run` returns; `drive_agent` translates that match into `hit_max_iterations: true` on the outcome.