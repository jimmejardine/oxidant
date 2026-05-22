// Realises spec/components/providers/openai.md.
//
// The workhorse Provider impl. Talks to OpenAI Chat Completions and any
// OpenAI-compatible endpoint (Azure OpenAI, Ollama, llama.cpp, LM Studio).
// SSE handling is hand-rolled per decision 0007.

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

#[derive(Debug, Clone)]
pub struct OpenAIConfig {
    /// Base URL through `/v1`. For OpenAI: `https://api.openai.com/v1`.
    pub base_url: String,
    /// Optional bearer token. None disables the Authorization header (local servers).
    pub api_key: Option<String>,
    /// `name()` returned by the Provider impl. Defaults to "openai".
    pub name: String,
    pub capabilities: ProviderCapabilities,
}

impl Default for OpenAIConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: None,
            name: "openai".to_string(),
            capabilities: ProviderCapabilities {
                tool_use: true,
                prompt_cache: false,
                extended_thinking: false,
                vision: true,
                max_context_tokens: 128_000,
            },
        }
    }
}

pub struct OpenAIProvider {
    config: OpenAIConfig,
    http: reqwest::Client,
}

impl OpenAIProvider {
    pub fn new(config: OpenAIConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::builder()
                .build()
                .expect("reqwest client builds with default settings"),
        }
    }

    pub fn with_client(config: OpenAIConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<BoxStream<'static, ChatEvent>> {
        let body = build_request_body(&req);
        let url = format!("{}/chat/completions", self.config.base_url.trim_end_matches('/'));

        let mut builder = self.http.post(&url).json(&body);
        if let Some(key) = &self.config.api_key {
            builder = builder.bearer_auth(key);
        }

        tracing::debug!(provider = %self.config.name, model = %req.model, "POST /chat/completions");

        let response = builder.send().await.with_context(|| {
            format!("HTTP request to {url} failed before response headers")
        })?;
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
        let event_stream = sse_event_stream(byte_stream);

        Ok(Box::pin(event_stream))
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.config.capabilities
    }

    fn name(&self) -> &str {
        &self.config.name
    }
}

/// Hand-rolled SSE parser → translator to ChatEvent.
///
/// The OpenAI streaming format emits `data: {json}\n\n` events terminated by
/// `data: [DONE]`. We accumulate bytes, split events on `\n\n`, take each
/// `data:` value, parse the JSON, and translate to one or more ChatEvents.
fn sse_event_stream<S>(
    mut byte_stream: S,
) -> impl futures::Stream<Item = ChatEvent> + Send + 'static
where
    S: futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Send + Unpin + 'static,
{
    let inner = try_stream! {
        let mut buffer = String::new();
        let mut tool_calls: HashMap<u32, ToolCallState> = HashMap::new();
        let mut sent_finish = false;

        while let Some(chunk) = byte_stream.next().await {
            let chunk = chunk.map_err(|e| anyhow!("network error reading SSE stream: {e}"))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(end) = find_event_end(&buffer) {
                let raw_event = buffer[..end].to_string();
                buffer.drain(..end + event_terminator_len(&buffer, end));

                let Some(data) = extract_data_field(&raw_event) else {
                    continue;
                };
                if data.trim() == "[DONE]" {
                    if !sent_finish {
                        // Some local servers (Ollama) omit usage entirely and
                        // skip a final chunk with finish_reason. Synthesise.
                        for state in tool_calls.values() {
                            yield ChatEvent::ToolUseEnd { id: state.id.clone() };
                        }
                        yield ChatEvent::Finish {
                            stop_reason: StopReason::EndTurn,
                            usage: Usage::default(),
                        };
                    }
                    return;
                }

                let chunk_json: ChatCompletionChunk = match serde_json::from_str(&data) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("could not parse SSE chunk: {e}; raw={}", truncate(&data, 200));
                        continue;
                    }
                };
                for event in translate_chunk(&chunk_json, &mut tool_calls) {
                    if matches!(event, ChatEvent::Finish { .. }) {
                        sent_finish = true;
                    }
                    yield event;
                }
            }
        }
    };

    // Convert errors yielded inside try_stream! into ChatEvent::Error so the
    // outer stream is `Stream<Item = ChatEvent>`, never panicking.
    inner.map(|res: anyhow::Result<ChatEvent>| match res {
        Ok(ev) => ev,
        Err(e) => ChatEvent::Error(e.to_string()),
    })
}

fn find_event_end(buffer: &str) -> Option<usize> {
    // SSE events end at a blank line: either \n\n or \r\n\r\n.
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

fn extract_data_field(event: &str) -> Option<String> {
    // An SSE event may have multiple data: lines; concat with '\n'.
    let mut data = String::new();
    let mut saw_data = false;
    for line in event.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            if saw_data {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            saw_data = true;
        }
    }
    if saw_data { Some(data) } else { None }
}

#[derive(Debug, Default)]
struct ToolCallState {
    id: String,
    #[allow(dead_code)]
    name: String,
}

fn translate_chunk(
    chunk: &ChatCompletionChunk,
    tool_calls: &mut HashMap<u32, ToolCallState>,
) -> Vec<ChatEvent> {
    let mut out = Vec::new();
    let Some(choice) = chunk.choices.first() else {
        // Some servers emit a final usage-only chunk with no choices.
        if let Some(usage) = chunk.usage.as_ref() {
            out.push(ChatEvent::Finish {
                stop_reason: StopReason::EndTurn,
                usage: usage.into_oxidant(),
            });
        }
        return out;
    };

    let delta = &choice.delta;
    if let Some(text) = delta.content.as_deref() {
        if !text.is_empty() {
            out.push(ChatEvent::TextDelta(text.to_string()));
        }
    }

    if let Some(tcs) = &delta.tool_calls {
        for tc in tcs {
            let index = tc.index;
            let state = tool_calls.entry(index).or_default();
            let is_new = state.id.is_empty();
            if let Some(id) = &tc.id {
                state.id = id.clone();
            }
            if let Some(func) = &tc.function {
                if let Some(name) = &func.name {
                    state.name = name.clone();
                }
                if is_new && !state.id.is_empty() && !state.name.is_empty() {
                    out.push(ChatEvent::ToolUseStart {
                        id: state.id.clone(),
                        name: state.name.clone(),
                    });
                }
                if let Some(args) = &func.arguments {
                    if !args.is_empty() && !state.id.is_empty() {
                        out.push(ChatEvent::ToolUseInputDelta {
                            id: state.id.clone(),
                            json_delta: args.clone(),
                        });
                    }
                }
            }
        }
    }

    if let Some(reason) = &choice.finish_reason {
        // Emit ToolUseEnd for every active tool call before Finish.
        for state in tool_calls.values() {
            out.push(ChatEvent::ToolUseEnd { id: state.id.clone() });
        }
        let usage = chunk.usage.as_ref().map(|u| u.into_oxidant()).unwrap_or_default();
        out.push(ChatEvent::Finish {
            stop_reason: parse_finish_reason(reason),
            usage,
        });
    }

    out
}

fn parse_finish_reason(s: &str) -> StopReason {
    match s {
        "stop" | "end_turn" => StopReason::EndTurn,
        "length" => StopReason::MaxTokens,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "content_filter" | "stop_sequence" => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n { s.to_string() } else { format!("{}…", &s[..n]) }
}

// ---------- Request body construction ----------

fn build_request_body(req: &ChatRequest) -> Value {
    let mut messages = Vec::<Value>::new();
    if let Some(system) = &req.system {
        messages.push(serde_json::json!({ "role": "system", "content": system }));
    }
    for msg in &req.messages {
        translate_message(msg, &mut messages);
    }

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
        "max_tokens": req.max_tokens,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if !req.tools.is_empty() {
        body["tools"] = serde_json::Value::Array(req.tools.iter().map(tool_to_openai).collect());
    }
    body
}

fn translate_message(msg: &RequestMessage, out: &mut Vec<Value>) {
    match msg.role {
        Role::User => translate_user_message(msg, out),
        Role::Assistant => translate_assistant_message(msg, out),
    }
}

fn translate_user_message(msg: &RequestMessage, out: &mut Vec<Value>) {
    let mut text_buf = String::new();
    let flush_text = |buf: &mut String, out: &mut Vec<Value>| {
        if !buf.is_empty() {
            out.push(serde_json::json!({
                "role": "user",
                "content": std::mem::take(buf),
            }));
        }
    };
    for part in &msg.content {
        match part {
            ContentPart::Text(s) | ContentPart::Thinking(s) => {
                if !text_buf.is_empty() {
                    text_buf.push('\n');
                }
                text_buf.push_str(s);
            }
            ContentPart::ToolResult { call_id, content, is_error: _ } => {
                flush_text(&mut text_buf, out);
                out.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": content,
                }));
            }
            ContentPart::ToolUse { .. } => {
                // Invalid on user role; drop with a debug log.
                tracing::debug!("dropping ContentPart::ToolUse on user role");
            }
        }
    }
    flush_text(&mut text_buf, out);
}

fn translate_assistant_message(msg: &RequestMessage, out: &mut Vec<Value>) {
    let mut text_buf = String::new();
    let mut tool_calls = Vec::<Value>::new();
    for part in &msg.content {
        match part {
            ContentPart::Text(s) => {
                if !text_buf.is_empty() {
                    text_buf.push('\n');
                }
                text_buf.push_str(s);
            }
            ContentPart::Thinking(_) => {
                // OpenAI/Ollama don't surface reasoning back as input; dropping
                // is the spec-aligned choice (extended_thinking: false).
            }
            ContentPart::ToolUse { id, name, input } => {
                tool_calls.push(serde_json::json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(input).unwrap_or_default(),
                    },
                }));
            }
            ContentPart::ToolResult { .. } => {
                tracing::debug!("dropping ContentPart::ToolResult on assistant role");
            }
        }
    }
    let mut msg = serde_json::json!({ "role": "assistant" });
    if !text_buf.is_empty() {
        msg["content"] = serde_json::Value::String(text_buf);
    } else {
        msg["content"] = serde_json::Value::Null;
    }
    if !tool_calls.is_empty() {
        msg["tool_calls"] = serde_json::Value::Array(tool_calls);
    }
    out.push(msg);
}

fn tool_to_openai(t: &ToolSpec) -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": t.name,
            "description": t.description,
            "parameters": t.input_schema,
        },
    })
}

// ---------- Wire types ----------

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<OpenAIUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    delta: ChatDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ToolCallFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct ToolCallFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OpenAIUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    #[allow(dead_code)]
    total_tokens: u32,
}

impl OpenAIUsage {
    fn into_oxidant(&self) -> Usage {
        Usage {
            input_tokens: self.prompt_tokens,
            output_tokens: self.completion_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_text_becomes_user_message() {
        let req = ChatRequest {
            model: "x".into(),
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
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "hello");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn assistant_tool_use_emits_tool_calls() {
        let req = ChatRequest {
            model: "x".into(),
            system: None,
            messages: vec![RequestMessage {
                role: Role::Assistant,
                content: vec![
                    ContentPart::Text("looking up…".into()),
                    ContentPart::ToolUse {
                        id: "call_1".into(),
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
        let m = &body["messages"][0];
        assert_eq!(m["role"], "assistant");
        assert_eq!(m["content"], "looking up…");
        assert_eq!(m["tool_calls"][0]["id"], "call_1");
        assert_eq!(m["tool_calls"][0]["function"]["name"], "get_weather");
        let args: serde_json::Value =
            serde_json::from_str(m["tool_calls"][0]["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args, json!({"location": "Paris"}));
    }

    #[test]
    fn user_tool_result_splits_into_tool_message() {
        let req = ChatRequest {
            model: "x".into(),
            system: None,
            messages: vec![RequestMessage {
                role: Role::User,
                content: vec![
                    ContentPart::ToolResult {
                        call_id: "call_1".into(),
                        content: "72F".into(),
                        is_error: false,
                    },
                    ContentPart::Text("thanks, what about Tokyo?".into()),
                ],
            }],
            tools: vec![],
            max_tokens: 64,
            temperature: None,
            thinking: None,
        };
        let body = build_request_body(&req);
        assert_eq!(body["messages"][0]["role"], "tool");
        assert_eq!(body["messages"][0]["tool_call_id"], "call_1");
        assert_eq!(body["messages"][0]["content"], "72F");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "thanks, what about Tokyo?");
    }

    #[test]
    fn sse_data_field_extraction_handles_prefix_space_and_multiline() {
        assert_eq!(extract_data_field("data: hello"), Some("hello".into()));
        assert_eq!(extract_data_field("data:hello"), Some("hello".into()));
        assert_eq!(
            extract_data_field("data: line1\ndata: line2"),
            Some("line1\nline2".into())
        );
        assert_eq!(extract_data_field("id: 5\nevent: msg"), None);
    }

    #[test]
    fn parse_finish_reason_maps_known_values() {
        assert_eq!(parse_finish_reason("stop"), StopReason::EndTurn);
        assert_eq!(parse_finish_reason("length"), StopReason::MaxTokens);
        assert_eq!(parse_finish_reason("tool_calls"), StopReason::ToolUse);
        assert_eq!(parse_finish_reason("content_filter"), StopReason::StopSequence);
    }
}
