---
id: conversation
kind: component
parent: overview
order: 2
implements: []
depends_on: []
code:
  - crates/oxidant-core/src/conversation.rs
  - crates/oxidant-core/src/message.rs
status: active
responsibility: |
  An append-only ordered list of messages (user, assistant, tool-result) representing one exploration's interaction history.
---

## Types

```rust
pub struct Conversation {
    pub id: Uuid,
    pub messages: Vec<Message>,
}

pub enum Message {
    User { content: Vec<ContentBlock> },
    Assistant { content: Vec<ContentBlock>, stop_reason: Option<StopReason>, usage: Option<Usage> },
    ToolResult { call_id: String, content: ToolResultContent, is_error: bool, elapsed_ms: u64 },
}

pub enum ContentBlock {
    Text(String),
    Image { ... },
    Thinking(String),
    ToolUse { id: String, name: String, input: serde_json::Value },
}
```

## Tool-call timing

`Message::ToolResult.elapsed_ms` is the wall-clock duration between [[components/core/agent-loop]] entering `registry.invoke(name, input, ctx)` and that future resolving. The agent loop measures with `Instant::now()` and passes the result through to `Conversation::push_tool_result`. The field is `#[serde(default)]` so older persisted `.jsonl` lines load with `elapsed_ms = 0`. Surfaced by [[components/gui/transcript-tab]] in the tool_result header alongside the rendered byte count (which is computed at render time, not persisted).

## Append-only

A conversation is never edited in place. Corrections happen by appending. This makes persistence trivial (`.jsonl` append) and undo trivial (truncate).

## Persistence

Persisted to `<worktree>/.oxidant/sessions/<exploration_id>.jsonl` as newline-delimited JSON, one `Message` per line. Restored on app launch by [[components/vcs/session-persistence]].

## Token accounting

Each `Assistant` message records its `Usage` (input/output/cache tokens). Conversation-level totals are derived; no separate counter to keep in sync.

## Compaction (deferred)

Long conversations eventually exceed context windows. v1 throws — relies on the user to start fresh. Future: a compaction step that summarises older turns. Tracked in [[components/core/agent-loop]] open work, not implemented here.
