// Realises spec/components/core/conversation.md (Conversation aggregate).
//
// Append-only ordered list of messages for one exploration. Persistence and
// compaction live elsewhere (session-persistence, agent-loop) — this type is
// just the data plus a handful of append helpers the agent loop reaches for.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use oxidant_providers::{StopReason, Usage};

use crate::message::{ContentBlock, Message, ToolResultContent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Uuid,
    pub messages: Vec<Message>,
    /// Messages at indices [0, compaction_at) are historical — shown
    /// in the transcript but excluded from the next provider request.
    /// Messages at [compaction_at, len) are the live context. 0 = no
    /// compaction. See spec/components/core/conversation.md and
    /// spec/components/gui/chat-input-panel.md "Slash commands".
    #[serde(default)]
    pub compaction_at: usize,
}

impl Conversation {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            messages: Vec::new(),
            compaction_at: 0,
        }
    }

    pub fn with_id(id: Uuid) -> Self {
        Self {
            id,
            messages: Vec::new(),
            compaction_at: 0,
        }
    }

    pub fn push_user_text(&mut self, text: impl Into<String>) {
        self.messages.push(Message::User {
            content: vec![ContentBlock::Text(text.into())],
        });
    }

    pub fn push_user_content(&mut self, content: Vec<ContentBlock>) {
        self.messages.push(Message::User { content });
    }

    pub fn push_assistant(
        &mut self,
        content: Vec<ContentBlock>,
        stop_reason: Option<StopReason>,
        usage: Option<Usage>,
    ) {
        self.messages.push(Message::Assistant {
            content,
            stop_reason,
            usage,
        });
    }

    pub fn push_tool_result(
        &mut self,
        call_id: impl Into<String>,
        content: ToolResultContent,
        is_error: bool,
        elapsed_ms: u64,
    ) {
        self.messages.push(Message::ToolResult {
            call_id: call_id.into(),
            content,
            is_error,
            elapsed_ms,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Reset to an empty live context, preserving `id`. Driven by
    /// `/clear` in the chat input panel.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.compaction_at = 0;
    }

    /// Append `summary` as a new starting User message and advance the
    /// compaction divider to point at it. Future provider requests
    /// will see only the summary + anything appended after. Driven by
    /// `/compact` in the chat input panel.
    pub fn install_compaction_summary(&mut self, summary: impl Into<String>) {
        let new_index = self.messages.len();
        self.messages.push(Message::User {
            content: vec![ContentBlock::Text(format!(
                "[CONTEXT SUMMARY]\n\n{}",
                summary.into()
            ))],
        });
        self.compaction_at = new_index;
    }

    /// The slice of messages the provider should see on the next
    /// request — i.e. everything from `compaction_at` onward. The
    /// transcript renders the full `messages` vec; only `build_request`
    /// in the agent loop walks this slice.
    pub fn live_messages(&self) -> &[Message] {
        let start = self.compaction_at.min(self.messages.len());
        &self.messages[start..]
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_messages_returns_tail_after_compaction_point() {
        let mut conv = Conversation::new();
        conv.push_user_text("first");
        conv.push_user_text("second");
        conv.install_compaction_summary("the gist");
        // 3 messages now; compaction_at = 2 (the new summary).
        assert_eq!(conv.messages.len(), 3);
        assert_eq!(conv.compaction_at, 2);
        let live = conv.live_messages();
        assert_eq!(live.len(), 1);
        assert!(matches!(&live[0], Message::User { .. }));
    }

    #[test]
    fn clear_resets_messages_and_compaction_preserving_id() {
        let mut conv = Conversation::new();
        let id = conv.id;
        conv.push_user_text("hi");
        conv.install_compaction_summary("recap");
        conv.clear();
        assert_eq!(conv.id, id);
        assert!(conv.messages.is_empty());
        assert_eq!(conv.compaction_at, 0);
    }

    #[test]
    fn compaction_at_serde_default_on_missing_field() {
        // Older .jsonl had no compaction_at; deserialise it as 0.
        let json = r#"{"id":"00000000-0000-0000-0000-000000000000","messages":[]}"#;
        let conv: Conversation = serde_json::from_str(json).unwrap();
        assert_eq!(conv.compaction_at, 0);
    }
}
