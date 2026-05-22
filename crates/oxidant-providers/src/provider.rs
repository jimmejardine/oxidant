// Realises spec/contracts/provider.md.
//
// Trait surface that every LLM backend implements; supporting request/event
// types are provider-agnostic, with backend-specific concerns (cache markers,
// SSE framing, native tool-call shapes) confined to each impl in this crate.

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;

#[async_trait]
pub trait Provider: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<BoxStream<'static, ChatEvent>>;
    fn capabilities(&self) -> ProviderCapabilities;
    fn name(&self) -> &str;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderCapabilities {
    pub tool_use: bool,
    pub prompt_cache: bool,
    pub extended_thinking: bool,
    pub vision: bool,
    pub max_context_tokens: usize,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<RequestMessage>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub thinking: Option<ThinkingConfig>,
}

#[derive(Debug, Clone)]
pub struct RequestMessage {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub enum ContentPart {
    Text(String),
    Thinking(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Copy)]
pub struct ThinkingConfig {
    pub budget_tokens: u32,
}

#[derive(Debug, Clone)]
pub enum ChatEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolUseStart { id: String, name: String },
    ToolUseInputDelta { id: String, json_delta: String },
    ToolUseEnd { id: String },
    Finish { stop_reason: StopReason, usage: Usage },
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    StopSequence,
    MaxTokens,
    ToolUse,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens: u32,
}
