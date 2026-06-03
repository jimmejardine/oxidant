// Realises spec/components/core/conversation.md (message types portion).
//
// Append-only message graph: User and Assistant carry ordered ContentBlocks;
// ToolResult is its own top-level variant referencing the prior assistant's
// ToolUse by call_id. The agent loop and request-builder respect this shape.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use oxidant_providers::{StopReason, Usage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    User {
        content: Vec<ContentBlock>,
    },
    Assistant {
        content: Vec<ContentBlock>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_reason: Option<StopReason>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    ToolResult {
        call_id: String,
        content: ToolResultContent,
        #[serde(default)]
        is_error: bool,
        /// Wall-clock duration between agent_loop entering registry.invoke
        /// and that future resolving. `#[serde(default)]` keeps old
        /// persisted `.jsonl` lines loadable (they get `0`). Surfaced by
        /// the transcript renderer in the tool_result header.
        #[serde(default)]
        elapsed_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentBlock {
    Text(String),
    Thinking(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Image content for vision-capable providers. Not wired into the local
    /// provider path yet — drops gracefully if a non-vision provider sees it.
    Image {
        source: ImageSource,
        media_type: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImageSource {
    Base64(String),
    Url(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolResultContent {
    Text(String),
    Json(Value),
}

impl ToolResultContent {
    /// Render this result as a string for providers that only accept text
    /// tool results (OpenAI Chat Completions). Json values are compact-encoded.
    pub fn as_string(&self) -> String {
        match self {
            ToolResultContent::Text(s) => s.clone(),
            ToolResultContent::Json(v) => v.to_string(),
        }
    }
}
