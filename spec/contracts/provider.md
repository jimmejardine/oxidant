```yaml
id: provider
kind: contract
parent: overview
order: 2
status: active
depends_on: []
code:
  - crates/oxidant-providers/src/provider.rs
responsibility: |
  The trait every LLM backend implements: async streaming chat with tool use, plus a capabilities probe.
```

`Provider` is the uniform interface oxidant uses to talk to LLM backends. The agent loop never knows which concrete backend it has; it has a `dyn Provider` and calls `chat`. Per-backend quirks (tool-call payload shapes, streaming framing) are normalised behind impls.

## Trait

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> Result<BoxStream<'static, ChatEvent>>;
    fn capabilities(&self) -> ProviderCapabilities;
    fn name(&self) -> &str;
}

pub struct ProviderCapabilities {
    pub tool_use: bool,
    pub prompt_cache: bool,
    pub extended_thinking: bool,
    pub vision: bool,
    pub max_context_tokens: usize,
}
```

## Request shape

The request is provider-agnostic; impls translate into native API payloads. Caching markers and other backend-specific concerns are applied inside the impl based on `capabilities()`, not the caller.

```rust
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<RequestMessage>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub thinking: Option<ThinkingConfig>,
}

pub struct RequestMessage {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

pub enum Role { User, Assistant }

pub enum ContentPart {
    Text(String),
    Thinking(String),
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { call_id: String, content: String, is_error: bool },
    // Image/vision content lands when the vision capability is wired through.
}

pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

pub struct ThinkingConfig {
    pub budget_tokens: u32,
}
```

## Event shape

```rust
pub enum ChatEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolUseStart { id: String, name: String },
    ToolUseInputDelta { id: String, json_delta: String },
    ToolUseEnd { id: String },
    Finish { stop_reason: StopReason, usage: Usage },
    Error(String),
}

pub enum StopReason {
    EndTurn,
    StopSequence,
    MaxTokens,
    ToolUse,
}

pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens: u32,
}
```

## Methods

| Method | Returns | Contract |
|---|---|---|
| `chat` | `Stream<ChatEvent>` | Streams the assistant's response. Tool use is interleaved as `ToolUseStart` → `*Delta`s → `ToolUseEnd`. Provider impls translate native API events into this normalised stream. |
| `capabilities` | `ProviderCapabilities` | Cheap, deterministic — used by the agent loop to decide whether to include `cache_control` markers, thinking budgets, etc. |
| `name` | `&str` | Stable identifier for logs and config (`"anthropic"`, `"openai"`, `"ollama"`). |

## Invariants

- `chat` never blocks; the returned stream may yield error events but must not panic.
- Streams complete with exactly one `Finish` or one terminal `Error` — never both, never neither.
- Tool-use deltas form valid concatenated JSON for the tool call's `input` argument.

## Implementors

- [[components/providers/anthropic]] — native Claude API
- [[components/providers/openai]] — Chat Completions (Responses API behind a feature flag)
- [[components/providers/ollama]] — OpenAI-compatible local endpoint; same code path as llama.cpp's server

See [[decisions/0001-multi-provider-llm]] and [[decisions/0007-roll-own-llm-provider-layer]] for the rationale.
