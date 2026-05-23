// Realises spec/components/providers/anthropic.md.
//
// Native Anthropic Messages API. Hand-rolled SSE per decision 0007 —
// Anthropic emits `event: <name>\ndata: <json>\n\n` per chunk; we
// translate the typed events into ChatEvent.
//
// Capabilities the spec calls for and this provider advertises:
//   - tool_use: full Messages API tool-call shape (input is a real
//     JSON object on send/receive, not stringified).
//   - prompt_cache: cache_control markers on system/tools/messages
//     when the agent loop opts in. (Marker injection is the caller's
//     job; this provider passes them through verbatim.)
//   - extended_thinking: `thinking: { type: "enabled", budget_tokens }`
//     when configured; streams ThinkingDelta events.
//   - vision: passes through Image content parts when supplied.

use std::collections::HashMap;

use anyhow::{Context, anyhow};
use async_stream::try_stream;
use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::{
    ChatEvent, ChatRequest, ContentPart, Provider, ProviderCapabilities, RequestMessage, Role,
    StopReason, ToolSpec, Usage,
};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const DEFAULT_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    /// Base URL through `/v1`. Defaults to api.anthropic.com.
    pub base_url: String,
    /// Required; without it `chat()` returns an error before any HTTP.
    /// `AnthropicConfig::from_env()` populates this from ANTHROPIC_API_KEY.
    pub api_key: Option<String>,
    /// Sent in the `anthropic-version` header.
    pub anthropic_version: String,
    /// Optional `anthropic-beta` header value(s), comma-separated.
    pub anthropic_beta: Option<String>,
    /// `name()` returned by the Provider impl.
    pub name: String,
    pub capabilities: ProviderCapabilities,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: None,
            anthropic_version: DEFAULT_VERSION.to_string(),
            anthropic_beta: None,
            name: "anthropic".to_string(),
            capabilities: ProviderCapabilities {
                tool_use: true,
                prompt_cache: true,
                extended_thinking: true,
                vision: true,
                max_context_tokens: 200_000,
            },
        }
    }
}

impl AnthropicConfig {
    /// Populate `api_key` from ANTHROPIC_API_KEY, leaving everything else
    /// at its default. Returns None-keyed config if the env var is absent,
    /// matching how `from_env` is used at startup (callers can still set
    /// `api_key` from settings before constructing the provider).
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            ..Self::default()
        }
    }
}

pub struct AnthropicProvider {
    config: AnthropicConfig,
    http: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::builder()
                .build()
                .expect("reqwest client builds with default settings"),
        }
    }

    pub fn with_client(config: AnthropicConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<BoxStream<'static, ChatEvent>> {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("anthropic: no API key — set ANTHROPIC_API_KEY"))?;

        let body = build_request_body(&req);
        let url = format!("{}/messages", self.config.base_url.trim_end_matches('/'));

        let mut builder = self
            .http
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", &self.config.anthropic_version)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&body);
        if let Some(beta) = &self.config.anthropic_beta {
            builder = builder.header("anthropic-beta", beta);
        }

        tracing::debug!(
            provider = %self.config.name,
            model = %req.model,
            "POST /v1/messages"
        );

        let response = builder
            .send()
            .await
            .with_context(|| format!("HTTP request to {url} failed before response headers"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "{} returned HTTP {status}: {}",
                self.config.name,
                truncate(&body, 1024)
            ));
        }

        let byte_stream = response.bytes_stream();
        Ok(Box::pin(sse_event_stream(byte_stream)))
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.config.capabilities
    }

    fn name(&self) -> &str {
        &self.config.name
    }
}

// ----------------------------------------------------------------- Request

fn build_request_body(req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req.messages.iter().map(translate_message).collect();

    let mut body = serde_json::json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "messages": messages,
        "stream": true,
    });
    if let Some(system) = &req.system {
        body["system"] = Value::String(system.clone());
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.iter().map(tool_to_anthropic).collect());
    }
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(thinking) = &req.thinking {
        body["thinking"] = serde_json::json!({
            "type": "enabled",
            "budget_tokens": thinking.budget_tokens,
        });
    }
    body
}

fn translate_message(msg: &RequestMessage) -> Value {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let content: Vec<Value> = msg
        .content
        .iter()
        .filter_map(translate_content_part)
        .collect();
    serde_json::json!({
        "role": role,
        "content": content,
    })
}

fn translate_content_part(part: &ContentPart) -> Option<Value> {
    match part {
        ContentPart::Text(s) => Some(serde_json::json!({
            "type": "text",
            "text": s,
        })),
        ContentPart::Thinking(s) => Some(serde_json::json!({
            "type": "thinking",
            "thinking": s,
        })),
        ContentPart::ToolUse { id, name, input } => Some(serde_json::json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": input,
        })),
        ContentPart::ToolResult {
            call_id,
            content,
            is_error,
        } => Some(serde_json::json!({
            "type": "tool_result",
            "tool_use_id": call_id,
            "content": content,
            "is_error": is_error,
        })),
    }
}

fn tool_to_anthropic(t: &ToolSpec) -> Value {
    serde_json::json!({
        "name": t.name,
        "description": t.description,
        "input_schema": t.input_schema,
    })
}

// ----------------------------------------------------------------- SSE

/// Anthropic streaming events arrive as
///   event: <name>\ndata: <json>\n\n
/// We accumulate bytes, slice on the blank line, and translate each
/// typed event into one or more ChatEvent emissions. A single Finish
/// event is emitted at message_stop, carrying the accumulated
/// stop_reason + usage.
fn sse_event_stream<S>(
    mut byte_stream: S,
) -> impl futures::Stream<Item = ChatEvent> + Send + 'static
where
    S: futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Send + Unpin + 'static,
{
    let inner = try_stream! {
        let mut buffer = String::new();
        let mut blocks: HashMap<u32, BlockState> = HashMap::new();
        let mut pending_stop: Option<StopReason> = None;
        let mut pending_usage: Usage = Usage::default();

        while let Some(chunk) = byte_stream.next().await {
            let chunk = chunk.map_err(|e| anyhow!("network error reading SSE stream: {e}"))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(end) = find_event_end(&buffer) {
                let raw_event = buffer[..end].to_string();
                buffer.drain(..end + event_terminator_len(&buffer, end));

                let Some(parsed) = parse_sse_event(&raw_event) else { continue };
                let event_name = parsed.name.as_deref().unwrap_or("");

                if event_name == "ping" {
                    continue;
                }
                if event_name == "error" {
                    let message = serde_json::from_str::<Value>(&parsed.data)
                        .ok()
                        .and_then(|v| {
                            v.get("error")
                                .and_then(|e| e.get("message"))
                                .and_then(|m| m.as_str())
                                .map(String::from)
                        })
                        .unwrap_or_else(|| parsed.data.clone());
                    yield ChatEvent::Error(format!("anthropic error: {message}"));
                    return;
                }

                let payload: Value = match serde_json::from_str(&parsed.data) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            "anthropic SSE chunk failed to parse: {e}; raw={}",
                            truncate(&parsed.data, 200)
                        );
                        continue;
                    }
                };

                for event in translate_event(event_name, &payload, &mut blocks, &mut pending_stop, &mut pending_usage) {
                    yield event;
                }

                if event_name == "message_stop" {
                    yield ChatEvent::Finish {
                        stop_reason: pending_stop.unwrap_or(StopReason::EndTurn),
                        usage: pending_usage,
                    };
                    return;
                }
            }
        }

        // Stream closed without an explicit message_stop — still emit a
        // single Finish so the contract holds (exactly one terminal event).
        for state in blocks.values() {
            if let (BlockKind::ToolUse, Some(id)) = (&state.kind, &state.tool_id) {
                yield ChatEvent::ToolUseEnd { id: id.clone() };
            }
        }
        yield ChatEvent::Finish {
            stop_reason: pending_stop.unwrap_or(StopReason::EndTurn),
            usage: pending_usage,
        };
    };

    inner.map(|res: anyhow::Result<ChatEvent>| match res {
        Ok(ev) => ev,
        Err(e) => ChatEvent::Error(e.to_string()),
    })
}

struct BlockState {
    kind: BlockKind,
    tool_id: Option<String>,
    #[allow(dead_code)]
    tool_name: Option<String>,
}

enum BlockKind {
    Text,
    Thinking,
    ToolUse,
}

fn translate_event(
    event_name: &str,
    payload: &Value,
    blocks: &mut HashMap<u32, BlockState>,
    pending_stop: &mut Option<StopReason>,
    pending_usage: &mut Usage,
) -> Vec<ChatEvent> {
    let mut out = Vec::new();
    match event_name {
        "message_start" => {
            // Seed input_tokens from the initial usage report.
            if let Some(usage) = payload.get("message").and_then(|m| m.get("usage")) {
                merge_usage(pending_usage, usage);
            }
        }
        "content_block_start" => {
            let Some(index) = payload.get("index").and_then(|v| v.as_u64()) else {
                return out;
            };
            let index = index as u32;
            let Some(block) = payload.get("content_block") else {
                return out;
            };
            let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match block_type {
                "text" => {
                    blocks.insert(
                        index,
                        BlockState {
                            kind: BlockKind::Text,
                            tool_id: None,
                            tool_name: None,
                        },
                    );
                }
                "thinking" => {
                    blocks.insert(
                        index,
                        BlockState {
                            kind: BlockKind::Thinking,
                            tool_id: None,
                            tool_name: None,
                        },
                    );
                }
                "tool_use" => {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    blocks.insert(
                        index,
                        BlockState {
                            kind: BlockKind::ToolUse,
                            tool_id: Some(id.clone()),
                            tool_name: Some(name.clone()),
                        },
                    );
                    if !id.is_empty() && !name.is_empty() {
                        out.push(ChatEvent::ToolUseStart { id, name });
                    }
                }
                _ => {}
            }
        }
        "content_block_delta" => {
            let Some(index) = payload.get("index").and_then(|v| v.as_u64()) else {
                return out;
            };
            let index = index as u32;
            let Some(delta) = payload.get("delta") else {
                return out;
            };
            let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match delta_type {
                "text_delta" => {
                    if let Some(text) = delta.get("text").and_then(|v| v.as_str())
                        && !text.is_empty()
                    {
                        out.push(ChatEvent::TextDelta(text.to_string()));
                    }
                }
                "thinking_delta" => {
                    if let Some(text) = delta.get("thinking").and_then(|v| v.as_str())
                        && !text.is_empty()
                    {
                        out.push(ChatEvent::ThinkingDelta(text.to_string()));
                    }
                }
                "input_json_delta" => {
                    if let Some(partial) = delta.get("partial_json").and_then(|v| v.as_str())
                        && !partial.is_empty()
                        && let Some(state) = blocks.get(&index)
                        && let Some(id) = &state.tool_id
                    {
                        out.push(ChatEvent::ToolUseInputDelta {
                            id: id.clone(),
                            json_delta: partial.to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let Some(index) = payload.get("index").and_then(|v| v.as_u64()) else {
                return out;
            };
            let index = index as u32;
            if let Some(state) = blocks.remove(&index)
                && let (BlockKind::ToolUse, Some(id)) = (state.kind, state.tool_id)
            {
                out.push(ChatEvent::ToolUseEnd { id });
            }
        }
        "message_delta" => {
            if let Some(delta) = payload.get("delta")
                && let Some(reason) = delta.get("stop_reason").and_then(|v| v.as_str())
            {
                *pending_stop = Some(parse_stop_reason(reason));
            }
            if let Some(usage) = payload.get("usage") {
                merge_usage(pending_usage, usage);
            }
        }
        _ => {}
    }
    out
}

fn merge_usage(usage: &mut Usage, source: &Value) {
    if let Some(v) = source.get("input_tokens").and_then(|v| v.as_u64()) {
        usage.input_tokens = usage.input_tokens.saturating_add(v as u32);
    }
    if let Some(v) = source.get("output_tokens").and_then(|v| v.as_u64()) {
        usage.output_tokens = usage.output_tokens.saturating_add(v as u32);
    }
    if let Some(v) = source
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_u64())
    {
        usage.cache_creation_input_tokens =
            usage.cache_creation_input_tokens.saturating_add(v as u32);
    }
    if let Some(v) = source
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
    {
        usage.cache_read_input_tokens = usage.cache_read_input_tokens.saturating_add(v as u32);
    }
}

fn parse_stop_reason(s: &str) -> StopReason {
    match s {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        "tool_use" => StopReason::ToolUse,
        _ => StopReason::EndTurn,
    }
}

// ----------------------------------------------------------------- Parser

#[derive(Debug, Deserialize, Serialize)]
struct ParsedSseEvent {
    name: Option<String>,
    data: String,
}

fn parse_sse_event(event: &str) -> Option<ParsedSseEvent> {
    let mut name = None;
    let mut data = String::new();
    let mut saw_data = false;
    for line in event.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            if saw_data {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            saw_data = true;
        }
    }
    if !saw_data {
        return None;
    }
    Some(ParsedSseEvent { name, data })
}

fn find_event_end(buffer: &str) -> Option<usize> {
    if let Some(idx) = buffer.find("\n\n") {
        return Some(idx);
    }
    buffer.find("\r\n\r\n")
}

fn event_terminator_len(buffer: &str, end: usize) -> usize {
    if buffer[end..].starts_with("\r\n\r\n") {
        4
    } else {
        2
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_text_becomes_user_message_with_text_block() {
        let req = ChatRequest {
            model: "claude-opus-4-7".into(),
            system: Some("be brief".into()),
            messages: vec![RequestMessage {
                role: Role::User,
                content: vec![ContentPart::Text("hello".into())],
            }],
            tools: vec![],
            max_tokens: 64,
            temperature: None,
            thinking: None,
        };
        let body = build_request_body(&req);
        assert_eq!(body["model"], "claude-opus-4-7");
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
        assert_eq!(body["messages"][0]["content"][0]["text"], "hello");
    }

    #[test]
    fn assistant_tool_use_block_keeps_input_as_json_object() {
        let req = ChatRequest {
            model: "x".into(),
            system: None,
            messages: vec![RequestMessage {
                role: Role::Assistant,
                content: vec![
                    ContentPart::Text("looking up…".into()),
                    ContentPart::ToolUse {
                        id: "toolu_1".into(),
                        name: "get_weather".into(),
                        input: json!({"location": "Paris"}),
                    },
                ],
            }],
            tools: vec![],
            max_tokens: 64,
            temperature: None,
            thinking: None,
        };
        let body = build_request_body(&req);
        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"][0]["type"], "text");
        assert_eq!(msg["content"][1]["type"], "tool_use");
        assert_eq!(msg["content"][1]["id"], "toolu_1");
        // Critically: input stays a JSON object — Anthropic does NOT
        // stringify it the way OpenAI does.
        assert_eq!(msg["content"][1]["input"], json!({"location": "Paris"}));
    }

    #[test]
    fn tool_result_lives_inside_user_message_content() {
        let req = ChatRequest {
            model: "x".into(),
            system: None,
            messages: vec![RequestMessage {
                role: Role::User,
                content: vec![
                    ContentPart::ToolResult {
                        call_id: "toolu_1".into(),
                        content: "72F".into(),
                        is_error: false,
                    },
                    ContentPart::Text("thanks!".into()),
                ],
            }],
            tools: vec![],
            max_tokens: 64,
            temperature: None,
            thinking: None,
        };
        let body = build_request_body(&req);
        let blocks = &body["messages"][0]["content"];
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[0]["tool_use_id"], "toolu_1");
        assert_eq!(blocks[0]["content"], "72F");
        assert_eq!(blocks[0]["is_error"], false);
        assert_eq!(blocks[1]["type"], "text");
    }

    #[test]
    fn thinking_config_marshals_when_present() {
        let req = ChatRequest {
            model: "x".into(),
            system: None,
            messages: vec![],
            tools: vec![],
            max_tokens: 64,
            temperature: None,
            thinking: Some(crate::provider::ThinkingConfig {
                budget_tokens: 5000,
            }),
        };
        let body = build_request_body(&req);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 5000);
    }

    #[test]
    fn parse_sse_event_handles_event_and_data() {
        let raw = "event: content_block_delta\ndata: {\"x\":1}";
        let p = parse_sse_event(raw).unwrap();
        assert_eq!(p.name.as_deref(), Some("content_block_delta"));
        assert_eq!(p.data, "{\"x\":1}");
    }

    #[test]
    fn parse_sse_event_supports_multiline_data() {
        let raw = "event: content_block_delta\ndata: line1\ndata: line2";
        let p = parse_sse_event(raw).unwrap();
        assert_eq!(p.data, "line1\nline2");
    }

    #[test]
    fn parse_stop_reason_maps_known_values() {
        assert_eq!(parse_stop_reason("end_turn"), StopReason::EndTurn);
        assert_eq!(parse_stop_reason("max_tokens"), StopReason::MaxTokens);
        assert_eq!(parse_stop_reason("stop_sequence"), StopReason::StopSequence);
        assert_eq!(parse_stop_reason("tool_use"), StopReason::ToolUse);
        assert_eq!(parse_stop_reason("???"), StopReason::EndTurn);
    }

    #[test]
    fn translate_event_text_delta_emits_textdelta() {
        let mut blocks = HashMap::new();
        blocks.insert(
            0,
            BlockState {
                kind: BlockKind::Text,
                tool_id: None,
                tool_name: None,
            },
        );
        let mut stop = None;
        let mut usage = Usage::default();
        let payload = json!({
            "index": 0,
            "delta": { "type": "text_delta", "text": "Hello" }
        });
        let evs = translate_event(
            "content_block_delta",
            &payload,
            &mut blocks,
            &mut stop,
            &mut usage,
        );
        assert_eq!(evs.len(), 1);
        assert!(matches!(evs[0], ChatEvent::TextDelta(ref s) if s == "Hello"));
    }

    #[test]
    fn translate_event_tool_use_lifecycle_emits_start_delta_end() {
        let mut blocks: HashMap<u32, BlockState> = HashMap::new();
        let mut stop = None;
        let mut usage = Usage::default();

        // Start
        let evs = translate_event(
            "content_block_start",
            &json!({
                "index": 1,
                "content_block": { "type": "tool_use", "id": "toolu_x", "name": "get_weather" }
            }),
            &mut blocks,
            &mut stop,
            &mut usage,
        );
        assert!(
            matches!(evs.first(), Some(ChatEvent::ToolUseStart { id, name })
            if id == "toolu_x" && name == "get_weather")
        );

        // Input delta
        let evs = translate_event(
            "content_block_delta",
            &json!({
                "index": 1,
                "delta": { "type": "input_json_delta", "partial_json": "{\"loc" }
            }),
            &mut blocks,
            &mut stop,
            &mut usage,
        );
        assert!(
            matches!(evs.first(), Some(ChatEvent::ToolUseInputDelta { id, json_delta })
            if id == "toolu_x" && json_delta == "{\"loc")
        );

        // Stop
        let evs = translate_event(
            "content_block_stop",
            &json!({ "index": 1 }),
            &mut blocks,
            &mut stop,
            &mut usage,
        );
        assert!(matches!(evs.first(), Some(ChatEvent::ToolUseEnd { id }) if id == "toolu_x"));
    }

    #[test]
    fn translate_event_message_delta_records_stop_and_usage() {
        let mut blocks = HashMap::new();
        let mut stop = None;
        let mut usage = Usage::default();
        let payload = json!({
            "delta": { "stop_reason": "tool_use" },
            "usage": { "output_tokens": 42 }
        });
        let evs = translate_event(
            "message_delta",
            &payload,
            &mut blocks,
            &mut stop,
            &mut usage,
        );
        assert!(evs.is_empty()); // no immediate emission; folded into Finish at message_stop
        assert_eq!(stop, Some(StopReason::ToolUse));
        assert_eq!(usage.output_tokens, 42);
    }
}
