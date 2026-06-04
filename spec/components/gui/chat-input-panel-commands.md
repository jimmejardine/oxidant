```yaml
id: chat-input-panel-commands
kind: component
parent: chat-input-panel
order: 1
implements: []
depends_on:
  - components/core/conversation
code:
  - crates/oxidant-gui/src/panels/chat_input.rs
status: active
responsibility: |
  Slash command parsing and execution: ChatCommand enum, /clear, /compact,
  and unknown command handling.
```

## ChatCommand enum

```rust
pub enum ChatCommand<'a> {
    Clear,
    Compact,
    Unknown(&'a str),  // bare command name, without the slash
    Prompt(&'a str),   // pass-through to the LLM
}

pub fn parse(input: &str) -> ChatCommand<'_> {
    // Parses draft text into a ChatCommand variant.
    // Returns Prompt(input) for any input not starting with /.
}
```

## Parser rules

1. `input.trim_start()` then look for leading `/`. Absent → `Prompt(input)`.
2. Split the rest on the first whitespace; lowercase the head; match against known names.
3. Unknown head → `Unknown(head)`. Bare `/` (empty head) → `Prompt(input)`.

## `/clear`

Wipes the conversation immediately. `Conversation.id` is preserved (session persistence keys on it). Resets `messages`, `compaction_at`, `last_outcome`, `live_turn`. No confirmation dialog — typing `/clear` and submitting is itself the explicit signal.

## `/compact`

Asks the model to summarise the live conversation so future requests carry a compact context. Dispatch is a one-shot provider call (no `agent_loop::run`, no tools, no iteration); the request body is `live_messages()` + a hard-coded compaction system prompt asking for a ~500-word handover note in plain prose. The streamed assistant text appears in `live_turn` like any other turn.

On `Finish`, the panel calls `conv.install_compaction_summary(text)`: appends a new User message whose body is `[CONTEXT SUMMARY]\n\n{text}` and advances `Conversation.compaction_at` to point at that message. Messages before `compaction_at` stay visible in the transcript but are not sent on subsequent requests — see [[components/core/conversation]]. The transcript draws a muted `── context compacted ──` divider at that point.

Compacting again later collapses the latest live-tail into a new summary; only the most recent `compaction_at` is honoured.

## Unknown commands

`Unknown("foo")` does not send anything to the provider. The panel:
- Sets `self.command_feedback = Some("unknown command: /foo")` (a per-panel `Option<String>` field rendered as a muted one-line label below the input).
- Restores `self.draft` to the original input so the user can fix the typo.
- Clears `command_feedback` on the next keystroke that modifies the draft.