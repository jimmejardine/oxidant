// Agent loop unit tests with a scripted mock Provider.
//
// MockProvider takes a Vec<Vec<ChatEvent>> — one inner Vec per turn. Each
// call to chat() pops the next turn's script and yields its events as a
// stream. This lets us assert the loop's behaviour across multi-turn
// scenarios without needing a real LLM.

use std::sync::Mutex;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::json;

use oxidant_core::{
    AgentLoopConfig, ContentBlock, Conversation, Message, Tool, ToolCategory, ToolContext,
    ToolRegistry, ToolResult, ToolResultContent, run,
};
use oxidant_providers::{
    ChatEvent, ChatRequest, Provider, ProviderCapabilities, StopReason, Usage,
};
use tokio_util::sync::CancellationToken;

struct MockProvider {
    turns: Mutex<Vec<Vec<ChatEvent>>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl MockProvider {
    fn new(turns: Vec<Vec<ChatEvent>>) -> Self {
        Self {
            turns: Mutex::new(turns),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn captured_requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<BoxStream<'static, ChatEvent>> {
        self.requests.lock().unwrap().push(req);
        let events = self
            .turns
            .lock()
            .unwrap()
            .pop()
            .unwrap_or_default();
        let stream = futures::stream::iter(events);
        Ok(Box::pin(stream))
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_use: true,
            ..Default::default()
        }
    }
    fn name(&self) -> &str {
        "mock"
    }
}

fn ctx() -> ToolContext {
    ToolContext {
        workspace_root: camino::Utf8PathBuf::from("."),
        exploration_id: "test".to_string(),
        cancellation: CancellationToken::new(),
    }
}

#[tokio::test]
async fn text_only_response_ends_after_one_iteration() {
    // turns are popped from the back; one turn here, ending with Finish.
    let provider = MockProvider::new(vec![vec![
        ChatEvent::TextDelta("Hello".into()),
        ChatEvent::TextDelta(", world!".into()),
        ChatEvent::Finish {
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 10, output_tokens: 3, ..Default::default() },
        },
    ]]);

    let registry = ToolRegistry::new();
    let mut conv = Conversation::new();
    conv.push_user_text("hi");

    let outcome = run(
        &provider,
        &registry,
        &ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
    )
    .await
    .unwrap();

    assert_eq!(outcome.iterations, 1);
    assert_eq!(outcome.stop_reason, Some(StopReason::EndTurn));
    assert_eq!(outcome.total_usage.input_tokens, 10);
    assert_eq!(outcome.tool_calls_dispatched, 0);

    // Conversation now has: user, assistant.
    assert_eq!(conv.messages.len(), 2);
    let Message::Assistant { content, .. } = &conv.messages[1] else {
        panic!("expected assistant message")
    };
    assert!(matches!(content.first(), Some(ContentBlock::Text(s)) if s == "Hello, world!"));
}

struct CurrentTimeStub {
    fixed: String,
}

#[async_trait]
impl Tool for CurrentTimeStub {
    fn name(&self) -> &str {
        "current_time"
    }
    fn description(&self) -> &str {
        "Return the current UTC time"
    }
    fn schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::Ok(json!({ "now_utc": self.fixed }))
    }
}

#[tokio::test]
async fn tool_call_is_dispatched_and_results_feed_next_turn() {
    // turns popped from back: turn 2 first (pushed last), then turn 1.
    let provider = MockProvider::new(vec![
        // turn 2: assistant uses the tool result to respond with text + EndTurn
        vec![
            ChatEvent::TextDelta("It is 2026.".into()),
            ChatEvent::Finish {
                stop_reason: StopReason::EndTurn,
                usage: Usage { input_tokens: 40, output_tokens: 4, ..Default::default() },
            },
        ],
        // turn 1: assistant emits a tool call and finishes with StopReason::ToolUse
        vec![
            ChatEvent::ToolUseStart {
                id: "tc1".into(),
                name: "current_time".into(),
            },
            ChatEvent::ToolUseInputDelta {
                id: "tc1".into(),
                json_delta: "{}".into(),
            },
            ChatEvent::ToolUseEnd { id: "tc1".into() },
            ChatEvent::Finish {
                stop_reason: StopReason::ToolUse,
                usage: Usage { input_tokens: 20, output_tokens: 5, ..Default::default() },
            },
        ],
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(CurrentTimeStub {
        fixed: "2026-05-22T12:00:00Z".into(),
    }));

    let mut conv = Conversation::new();
    conv.push_user_text("what year is it?");

    let outcome = run(
        &provider,
        &registry,
        &ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
    )
    .await
    .unwrap();

    assert_eq!(outcome.iterations, 2);
    assert_eq!(outcome.tool_calls_dispatched, 1);
    assert_eq!(outcome.total_usage.input_tokens, 60);
    assert_eq!(outcome.total_usage.output_tokens, 9);

    // conv: user, assistant(tool_use), tool_result, assistant(text)
    assert_eq!(conv.messages.len(), 4);
    assert!(matches!(&conv.messages[2], Message::ToolResult { call_id, .. } if call_id == "tc1"));

    // Second request should have the tool result in its messages
    let reqs = provider.captured_requests();
    assert_eq!(reqs.len(), 2);
    assert!(reqs[1].messages.iter().any(|m| {
        m.content.iter().any(|p| matches!(p, oxidant_providers::ContentPart::ToolResult { call_id, .. } if call_id == "tc1"))
    }));
}

#[tokio::test]
async fn malformed_tool_args_fall_back_to_empty_object() {
    let provider = MockProvider::new(vec![
        vec![
            ChatEvent::TextDelta("done".into()),
            ChatEvent::Finish {
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ],
        vec![
            ChatEvent::ToolUseStart { id: "tc1".into(), name: "current_time".into() },
            ChatEvent::ToolUseInputDelta {
                id: "tc1".into(),
                json_delta: "{not valid json".into(),
            },
            ChatEvent::ToolUseEnd { id: "tc1".into() },
            ChatEvent::Finish {
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            },
        ],
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(CurrentTimeStub {
        fixed: "2026-05-22T12:00:00Z".into(),
    }));

    let mut conv = Conversation::new();
    conv.push_user_text("time?");

    let outcome = run(
        &provider,
        &registry,
        &ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
    )
    .await
    .unwrap();

    // The malformed JSON should not panic; tool dispatched with empty input.
    assert_eq!(outcome.tool_calls_dispatched, 1);
    // Verify the tool result landed (the stub doesn't care about its args)
    let Message::ToolResult { content, is_error, .. } = &conv.messages[2] else {
        panic!("expected tool result")
    };
    assert!(!is_error);
    assert!(matches!(content, ToolResultContent::Json(_)));
}

#[tokio::test]
async fn error_event_terminates_with_err() {
    let provider = MockProvider::new(vec![vec![
        ChatEvent::TextDelta("part".into()),
        ChatEvent::Error("connection reset".into()),
    ]]);
    let registry = ToolRegistry::new();
    let mut conv = Conversation::new();
    conv.push_user_text("hi");

    let err = run(
        &provider,
        &registry,
        &ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("connection reset"));
}

#[tokio::test]
async fn max_iterations_bound_returns_error() {
    // Both turns end with ToolUse so the loop never reaches EndTurn.
    let provider = MockProvider::new(vec![
        vec![
            ChatEvent::ToolUseStart { id: "b".into(), name: "current_time".into() },
            ChatEvent::ToolUseInputDelta { id: "b".into(), json_delta: "{}".into() },
            ChatEvent::ToolUseEnd { id: "b".into() },
            ChatEvent::Finish { stop_reason: StopReason::ToolUse, usage: Usage::default() },
        ],
        vec![
            ChatEvent::ToolUseStart { id: "a".into(), name: "current_time".into() },
            ChatEvent::ToolUseInputDelta { id: "a".into(), json_delta: "{}".into() },
            ChatEvent::ToolUseEnd { id: "a".into() },
            ChatEvent::Finish { stop_reason: StopReason::ToolUse, usage: Usage::default() },
        ],
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(CurrentTimeStub {
        fixed: "x".into(),
    }));

    let mut conv = Conversation::new();
    conv.push_user_text("loop!");

    let mut config = AgentLoopConfig::new("m");
    config.max_iterations = 2;

    let err = run(
        &provider,
        &registry,
        &ctx(),
        &mut conv,
        &config,
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("max_iterations"));
}
