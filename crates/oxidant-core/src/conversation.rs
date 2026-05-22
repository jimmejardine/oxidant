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
}

impl Conversation {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            messages: Vec::new(),
        }
    }

    pub fn with_id(id: Uuid) -> Self {
        Self { id, messages: Vec::new() }
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
    ) {
        self.messages.push(Message::ToolResult {
            call_id: call_id.into(),
            content,
            is_error,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}
