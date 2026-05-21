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
    ToolResult { call_id: String, content: ToolResultContent, is_error: bool },
}

pub enum ContentBlock {
    Text(String),
    Image { ... },
    Thinking(String),
    ToolUse { id: String, name: String, input: serde_json::Value },
}
```

## Append-only

A conversation is never edited in place. Corrections happen by appending. This makes persistence trivial (`.jsonl` append) and undo trivial (truncate).

## Persistence

Persisted to `<worktree>/.oxidant/sessions/<exploration_id>.jsonl` as newline-delimited JSON, one `Message` per line. Restored on app launch by [[components/vcs/session-persistence]].

## Token accounting

Each `Assistant` message records its `Usage` (input/output/cache tokens). Conversation-level totals are derived; no separate counter to keep in sync.

## Compaction (deferred)

Long conversations eventually exceed context windows. v1 throws — relies on the user to start fresh. Future: a compaction step that summarises older turns. Tracked in [[components/core/agent-loop]] open work, not implemented here.
