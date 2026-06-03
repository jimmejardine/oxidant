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

A draft whose trimmed leading character is `/` is parsed by a small `panels/slash_commands` module before reaching the agent loop:

```rust
pub enum ChatCommand<'a> {
    Clear,
    Compact,
    Unknown(&'a str),  // bare command name, without the slash
    Prompt(&'a str),   // pass-through to the LLM
}

pub fn parse(input: &str) -> ChatCommand<'_>;
```

Parser rules:
- `input.trim_start()` then look for leading `/`. Absent → `Prompt(input)`.
- Split the rest on the first whitespace; lowercase the head; match against known names.
- Unknown head → `Unknown(head)`. Bare `/` (empty head) → `Prompt(input)`.

### `/clear`

Wipes the conversation immediately. `Conversation.id` is preserved (session persistence keys on it). Resets `messages`, `compaction_at`, `last_outcome`, `live_turn`. No confirmation dialog — typing `/clear` and submitting is itself the explicit signal.

### `/compact`

Asks the model to summarise the live conversation so future requests carry a compact context. Dispatch is a one-shot provider call (no `agent_loop::run`, no tools, no iteration); the request body is `live_messages()` + a hard-coded compaction system prompt asking for a ~500-word handover note in plain prose. The streamed assistant text appears in `live_turn` like any other turn.

On `Finish`, the panel calls `conv.install_compaction_summary(text)`: appends a new User message whose body is `[CONTEXT SUMMARY]\n\n{text}` and advances `Conversation.compaction_at` to point at that message. Messages before `compaction_at` stay visible in the transcript but are not sent on subsequent requests — see [[components/core/conversation]]. The transcript draws a muted `── context compacted ──` divider at that point.

Compacting again later collapses the latest live-tail into a new summary; only the most recent `compaction_at` is honoured.

### Unknown commands

`Unknown("foo")` does not send anything to the provider. The panel:
- Sets `self.command_feedback = Some("unknown command: /foo")` (a per-panel `Option<String>` field rendered as a muted one-line label below the input).
- Restores `self.draft` to the original input so the user can fix the typo.
- Clears `command_feedback` on the next keystroke that modifies the draft.
