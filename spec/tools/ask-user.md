```yaml
---
id: ask-user
kind: tool
parent: overview
order: 9
implements:
  - contracts/tool
depends_on:
  - contracts/tool
code:
  - crates/oxidant-tools/src/ask_user.rs
tests:
  - crates/oxidant-tools/src/ask_user.rs
status: active
responsibility: |
  Ask the user a multiple-choice question (with an optional free-form fallback) from within an agent turn. Blocks the calling tool task on a oneshot until the user clicks Submit; returns the chosen text.
---
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["question", "options"],
  "properties": {
    "question": {
      "type": "string",
      "minLength": 1,
      "description": "The question. Shows in a modal with no other context — be specific."
    },
    "options": {
      "type": "array",
      "items": { "type": "string", "minLength": 1 },
      "minItems": 1,
      "description": "Pre-canned answers. Each renders as a clickable button; clicking returns that string verbatim."
    },
    "allow_freeform": {
      "type": "boolean",
      "default": true,
      "description": "When true (default), an N+1th free-form text field is rendered below the option buttons; submitting it returns the typed text. Set false to force a choice among the listed options."
    }
  }
}
```

## Result

```json
{ "answer": "<the chosen option's text or the user's typed answer>" }
```

The `answer` string is exactly one of the supplied `options`, or — when `allow_freeform` is on and the user typed into the free-form field — the user's trimmed text. No extra structure; the agent reads `.answer` and proceeds.

## Semantics

The tool reaches the user via `ToolContext.ui` (the `UiBridge` from [[contracts/tool]] "UiBridge"). Implementation flow:

1. The tool posts a `PendingUserQuestion` into the matching window's `PerWindowState` and awaits a `tokio::sync::oneshot::Receiver<String>`. The agent loop is otherwise free — other concurrent tools keep running; the model's stream is already committed by this point in the turn.
2. The GUI's `UserQuestionPanel` (`crates/oxidant-gui/src/panels/user_question.rs`) renders a centred `egui::Window` overlay with the question, each option as a full-width button, and the optional free-form input. Clicking an option or submitting the free-form field calls `oneshot::Sender::send(answer)`. See [[components/gui/chat-input-panel]] for how the bridge is constructed per agent-loop spawn.
3. The tool's future resumes; `invoke` returns `ToolResult::Ok({ "answer": ... })`.

The bridge runs once per agent-loop spawn and is keyed by the *spawning window*'s `view_id` — sub-windows post into their own modal, not the main's.

## When to use

Call when the considered branches genuinely depend on user preference and you cannot reasonably pick on their behalf:

- a design choice with no objectively-better option (token-bucket vs. leaky-bucket);
- a name the user must decide (the new module's name);
- a tradeoff only they can evaluate (faster but bespoke vs. slower but standard).

Don't call for trivia the user shouldn't have to answer, and don't use it to ask permission for a tool call you're already authorised to make.

## Error modes

- **No interactive UI host** (`ctx.ui.is_none()`): the tool returns `ToolResult::Err("ask_user requires an interactive UI host; …")`. Surfaces unambiguously to the model so it can describe the choice in text instead.
- **User cancels** (ESC on the turn, window closes): the held `Sender` drops, the Receiver errors, and the tool returns `ToolResult::Err("ask_user failed: user cancelled or window closed")`. The agent sees this and can fall back to a default or stop.
- **Bad arguments**: missing/empty `question`, empty `options` — `ToolResult::Err` with a specific message. The registry's schema validation catches the structural cases; the tool itself catches semantic emptiness.
