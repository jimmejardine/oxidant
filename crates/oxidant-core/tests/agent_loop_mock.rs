// Agent loop unit tests with a scripted mock Provider.
//
// MockProvider takes a Vec<Vec<ChatEvent>> â€” one inner Vec per turn. Each
// call to chat() pops the next turn's script and yields its events as a
// stream. This lets us assert the loop's behaviour across multi-turn
// scenarios without needing a real LLM.

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
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
        let events = self.turns.lock().unwrap().pop().unwrap_or_default();
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
            usage: Usage {
                input_tokens: 10,
                output_tokens: 3,
                ..Default::default()
            },
        },
    ]]);

    let registry = ToolRegistry::new();
    let mut conv = Conversation::new();
    conv.push_user_text("hi");

    let outcome = run(
        &provider,
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
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
                usage: Usage {
                    input_tokens: 40,
                    output_tokens: 4,
                    ..Default::default()
                },
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
                usage: Usage {
                    input_tokens: 20,
                    output_tokens: 5,
                    ..Default::default()
                },
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
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
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
            ChatEvent::ToolUseStart {
                id: "tc1".into(),
                name: "current_time".into(),
            },
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
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
        |_| {},
    )
    .await
    .unwrap();

    // The malformed JSON should not panic; tool dispatched with empty input.
    assert_eq!(outcome.tool_calls_dispatched, 1);
    // Verify the tool result landed (the stub doesn't care about its args)
    let Message::ToolResult {
        content, is_error, ..
    } = &conv.messages[2]
    else {
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
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("connection reset"));
}

// ---- post-edit hook tests -----------------------------------------------

/// A mutating tool we can inject so the post-edit hook fires.
struct ScratchWrite;

#[async_trait]
impl Tool for ScratchWrite {
    fn name(&self) -> &str {
        "scratch_write"
    }
    fn schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mutating
    }
    async fn invoke(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::Ok(json!({ "wrote": "scratch" }))
    }
}

/// The check tool â€” ReadOnly. Counts how often it was invoked via shared state.
struct DriftCheck {
    invocations: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl Tool for DriftCheck {
    fn name(&self) -> &str {
        "drift_check"
    }
    fn schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.invocations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ToolResult::Ok(json!({ "count": 0, "drifts": [] }))
    }
}

fn config_with_hook() -> AgentLoopConfig {
    let mut cfg = AgentLoopConfig::new("m");
    cfg.post_edit_check_tool = Some("drift_check".into());
    cfg
}

#[tokio::test]
async fn post_edit_hook_fires_after_mutating_tool() {
    // Turn 2 is text-only (terminates). Turn 1 calls scratch_write (Mutating).
    let provider = MockProvider::new(vec![
        vec![
            ChatEvent::TextDelta("done".into()),
            ChatEvent::Finish {
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ],
        vec![
            ChatEvent::ToolUseStart {
                id: "m1".into(),
                name: "scratch_write".into(),
            },
            ChatEvent::ToolUseInputDelta {
                id: "m1".into(),
                json_delta: "{}".into(),
            },
            ChatEvent::ToolUseEnd { id: "m1".into() },
            ChatEvent::Finish {
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            },
        ],
    ]);

    let invocations = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(ScratchWrite));
    registry.register(std::sync::Arc::new(DriftCheck {
        invocations: invocations.clone(),
    }));

    let mut conv = Conversation::new();
    conv.push_user_text("write something");

    let outcome = run(
        &provider,
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &config_with_hook(),
        |_| {},
        |_| {},
    )
    .await
    .unwrap();

    assert_eq!(outcome.post_edit_checks_fired, 1);
    assert_eq!(invocations.load(std::sync::atomic::Ordering::SeqCst), 1);

    // The second request to the provider should include the synthetic User
    // message carrying the drift_check result.
    let reqs = provider.captured_requests();
    assert_eq!(reqs.len(), 2);
    let second_turn_messages = &reqs[1].messages;
    let has_post_edit_marker = second_turn_messages.iter().any(|m| {
        m.content.iter().any(|p| match p {
            oxidant_providers::ContentPart::Text(s) => s.contains("[oxidant post-edit check"),
            _ => false,
        })
    });
    assert!(
        has_post_edit_marker,
        "expected the post-edit check result in the second turn's messages"
    );
}

#[tokio::test]
async fn post_edit_hook_skipped_when_only_readonly_tools_used() {
    // Single turn calling a ReadOnly tool then finishing.
    struct PeekTool;
    #[async_trait]
    impl Tool for PeekTool {
        fn name(&self) -> &str {
            "peek"
        }
        fn schema(&self) -> serde_json::Value {
            json!({})
        }
        fn category(&self) -> ToolCategory {
            ToolCategory::ReadOnly
        }
        async fn invoke(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
            ToolResult::Ok(json!({}))
        }
    }

    let provider = MockProvider::new(vec![
        vec![
            ChatEvent::TextDelta("done".into()),
            ChatEvent::Finish {
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ],
        vec![
            ChatEvent::ToolUseStart {
                id: "p1".into(),
                name: "peek".into(),
            },
            ChatEvent::ToolUseInputDelta {
                id: "p1".into(),
                json_delta: "{}".into(),
            },
            ChatEvent::ToolUseEnd { id: "p1".into() },
            ChatEvent::Finish {
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            },
        ],
    ]);

    let invocations = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(PeekTool));
    registry.register(std::sync::Arc::new(DriftCheck {
        invocations: invocations.clone(),
    }));

    let mut conv = Conversation::new();
    conv.push_user_text("look");

    let outcome = run(
        &provider,
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &config_with_hook(),
        |_| {},
        |_| {},
    )
    .await
    .unwrap();

    assert_eq!(outcome.post_edit_checks_fired, 0);
    assert_eq!(invocations.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn post_edit_hook_silent_when_unconfigured() {
    let provider = MockProvider::new(vec![
        vec![
            ChatEvent::TextDelta("ok".into()),
            ChatEvent::Finish {
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ],
        vec![
            ChatEvent::ToolUseStart {
                id: "m1".into(),
                name: "scratch_write".into(),
            },
            ChatEvent::ToolUseInputDelta {
                id: "m1".into(),
                json_delta: "{}".into(),
            },
            ChatEvent::ToolUseEnd { id: "m1".into() },
            ChatEvent::Finish {
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            },
        ],
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(ScratchWrite));

    let mut conv = Conversation::new();
    conv.push_user_text("write");

    // No post_edit_check_tool configured.
    let outcome = run(
        &provider,
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(outcome.post_edit_checks_fired, 0);
}

#[tokio::test]
async fn max_iterations_bound_returns_error() {
    // Both turns end with ToolUse so the loop never reaches EndTurn.
    let provider = MockProvider::new(vec![
        vec![
            ChatEvent::ToolUseStart {
                id: "b".into(),
                name: "current_time".into(),
            },
            ChatEvent::ToolUseInputDelta {
                id: "b".into(),
                json_delta: "{}".into(),
            },
            ChatEvent::ToolUseEnd { id: "b".into() },
            ChatEvent::Finish {
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            },
        ],
        vec![
            ChatEvent::ToolUseStart {
                id: "a".into(),
                name: "current_time".into(),
            },
            ChatEvent::ToolUseInputDelta {
                id: "a".into(),
                json_delta: "{}".into(),
            },
            ChatEvent::ToolUseEnd { id: "a".into() },
            ChatEvent::Finish {
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            },
        ],
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(CurrentTimeStub { fixed: "x".into() }));

    let mut conv = Conversation::new();
    conv.push_user_text("loop!");

    let mut config = AgentLoopConfig::new("m");
    config.max_iterations = 2;

    let err = run(
        &provider,
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &config,
        |_| {},
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("max_iterations"));
}

// ---------------------------------------------------------------- timing

/// Provider that yields a tool_use, then sleeps before emitting Finish
/// on the FIRST call; on subsequent calls returns plain EndTurn so the
/// agent loop terminates. Exercises the "eager dispatch on ToolUseEnd"
/// path: the tool's spawned future should be running during the sleep,
/// so its elapsed_ms reflects the delay rather than zero.
struct DelayedFinishProvider {
    delay: Duration,
    calls: Mutex<u32>,
}

#[async_trait]
impl Provider for DelayedFinishProvider {
    async fn chat(&self, _req: ChatRequest) -> anyhow::Result<BoxStream<'static, ChatEvent>> {
        let n = {
            let mut g = self.calls.lock().unwrap();
            *g += 1;
            *g
        };
        if n > 1 {
            // Second turn: model "replies" to the tool result and ends.
            let s = stream::iter(vec![
                ChatEvent::TextDelta("done".into()),
                ChatEvent::Finish {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                },
            ]);
            return Ok(s.boxed());
        }
        let delay = self.delay;
        // First turn: hand-roll the stream so we can sleep between events.
        let s = stream::unfold(0u8, move |step| async move {
            match step {
                0 => Some((
                    ChatEvent::ToolUseStart {
                        id: "tc1".into(),
                        name: "current_time".into(),
                    },
                    1,
                )),
                1 => Some((
                    ChatEvent::ToolUseInputDelta {
                        id: "tc1".into(),
                        json_delta: "{}".into(),
                    },
                    2,
                )),
                2 => Some((ChatEvent::ToolUseEnd { id: "tc1".into() }, 3)),
                3 => {
                    tokio::time::sleep(delay).await;
                    Some((
                        ChatEvent::Finish {
                            stop_reason: StopReason::EndTurn,
                            usage: Usage::default(),
                        },
                        4,
                    ))
                }
                _ => None,
            }
        });
        Ok(s.boxed())
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_use: true,
            ..Default::default()
        }
    }
    fn name(&self) -> &str {
        "delayed-finish-mock"
    }
}

#[tokio::test]
async fn tool_dispatches_eagerly_during_stream_so_elapsed_reflects_real_wait() {
    let provider = DelayedFinishProvider {
        delay: Duration::from_millis(60),
        calls: Mutex::new(0),
    };
    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(CurrentTimeStub {
        fixed: "2026-06-03T12:00:00Z".into(),
    }));
    let mut conv = Conversation::new();
    conv.push_user_text("what time is it?");

    // The model emits ToolUseEnd up front, then sleeps for 60ms before
    // emitting Finish. With eager dispatch, the tool's future started
    // running ~immediately on ToolUseEnd; only the await for the result
    // happens after Finish. The captured elapsed_ms is the wall clock
    // from ToolUseEnd â†’ result, which includes the 60ms sleep.
    let mut config = AgentLoopConfig::new("m");
    config.max_iterations = 2;
    let _ = run(
        &provider,
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &config,
        |_| {},
        |_| {},
    )
    .await
    .unwrap();

    // First message is the user, second is the assistant, third is the
    // tool result. Pull elapsed_ms off the latter.
    let Message::ToolResult { elapsed_ms, .. } = &conv.messages[2] else {
        panic!(
            "expected tool result at index 2, got {:?}",
            conv.messages[2]
        );
    };
    // Allow generous slack for CI jitter; the floor proves the spawn
    // happened on ToolUseEnd rather than after Finish.
    assert!(
        *elapsed_ms >= 50,
        "expected elapsed_ms >= 50ms (sleep was 60ms), got {elapsed_ms}ms â€” dispatch is not eager"
    );
}

// ---------------------------------------------------------------- text-extracted timing

/// Same as DelayedFinishProvider but emits the tool call as a text
/// envelope (Qwen / Hermes style) inside a TextDelta, then sleeps,
/// then Finish. Verifies the incremental scanner in agent_loop dispatches
/// the extracted call eagerly during the stream rather than waiting
/// until after Finish.
/// A ReadOnly tool that sleeps for `delay` before returning. Lets tests
/// observe `elapsed_ms` without relying on stream-side timing — which
/// matters because the agent loop now cuts the stream the moment a
/// text-extracted envelope closes. See
/// spec/components/core/agent-loop.md "Tool dispatch concurrency".
struct SlowCurrentTimeStub {
    fixed: String,
    delay: Duration,
}

#[async_trait]
impl Tool for SlowCurrentTimeStub {
    fn name(&self) -> &str {
        "current_time"
    }
    fn description(&self) -> &str {
        "Return the current UTC time after an artificial delay"
    }
    fn schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        tokio::time::sleep(self.delay).await;
        ToolResult::Ok(json!({ "now_utc": self.fixed }))
    }
}

struct TextEnvelopeThenFinishProvider {
    calls: Mutex<u32>,
}

#[async_trait]
impl Provider for TextEnvelopeThenFinishProvider {
    async fn chat(&self, _req: ChatRequest) -> anyhow::Result<BoxStream<'static, ChatEvent>> {
        let n = {
            let mut g = self.calls.lock().unwrap();
            *g += 1;
            *g
        };
        if n > 1 {
            // Second turn: model replies to the tool result and ends.
            let s = stream::iter(vec![
                ChatEvent::TextDelta("done".into()),
                ChatEvent::Finish {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                },
            ]);
            return Ok(s.boxed());
        }
        // Stream emits: prose, complete envelope, then speculative text
        // and Finish. The loop should cut at the envelope close, so the
        // speculative tail is queued on the stream but never consumed.
        let s = stream::unfold(0u8, move |step| async move {
            match step {
                0 => Some((ChatEvent::TextDelta("Let me check. ".into()), 1)),
                // Whole envelope arrives in one delta â€” incremental
                // scan should find Complete on this delta.
                1 => Some((
                    ChatEvent::TextDelta(
                        "<tool_call>{\"name\":\"current_time\",\"arguments\":{}}</tool_call>"
                            .into(),
                    ),
                    2,
                )),
                2 => Some((ChatEvent::TextDelta(" The time is 12:34 UTC.".into()), 3)),
                3 => Some((
                    ChatEvent::TextDelta(
                        "<tool_call>{\"name\":\"current_time\",\"arguments\":{}}</tool_call>"
                            .into(),
                    ),
                    4,
                )),
                4 => Some((
                    ChatEvent::Finish {
                        stop_reason: StopReason::EndTurn,
                        usage: Usage::default(),
                    },
                    5,
                )),
                _ => None,
            }
        });
        Ok(s.boxed())
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_use: true,
            ..Default::default()
        }
    }
    fn name(&self) -> &str {
        "text-envelope-then-finish-mock"
    }
}

#[tokio::test]
async fn text_extracted_tool_dispatches_eagerly_during_stream() {
    // Eagerness is now demonstrated via a slow tool (the stream gets
    // cut at the envelope close, so any stream-side delay would be
    // unobservable). `elapsed_ms >= 50` proves the spawn happened the
    // moment the envelope landed rather than being inlined as a post-
    // Finish blocking invoke.
    let provider = TextEnvelopeThenFinishProvider {
        calls: Mutex::new(0),
    };
    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(SlowCurrentTimeStub {
        fixed: "2026-06-03T12:00:00Z".into(),
        delay: Duration::from_millis(60),
    }));
    let mut conv = Conversation::new();
    conv.push_user_text("what time is it?");

    let mut config = AgentLoopConfig::new("m");
    config.max_iterations = 2;
    let _ = run(
        &provider,
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &config,
        |_| {},
        |_| {},
    )
    .await
    .unwrap();

    // user, assistant, tool_result.
    let Message::ToolResult { elapsed_ms, .. } = &conv.messages[2] else {
        panic!(
            "expected tool result at index 2, got {:?}",
            conv.messages[2]
        );
    };
    // The 60ms sleep between TextDelta (envelope) and Finish should be
    // captured in elapsed_ms because the tool's spawn happened on the
    // delta carrying the envelope's close tag â€” well before Finish.
    assert!(
        *elapsed_ms >= 50,
        "expected elapsed_ms >= 50ms (tool slept 60ms), got {elapsed_ms}ms — text-extracted dispatch is not eager"
    );

    // The committed assistant message must NOT carry the raw
    // <tool_call> envelope text â€” it was stripped post-stream.
    let Message::Assistant { content, .. } = &conv.messages[1] else {
        panic!("expected assistant at index 1, got {:?}", conv.messages[1]);
    };
    let text_blocks: String = content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !text_blocks.contains("<tool_call>"),
        "extracted envelope should have been stripped, but assistant text still contains it: {text_blocks:?}"
    );
}

#[tokio::test]
async fn text_extracted_tool_call_cuts_stream_so_speculative_continuation_is_dropped() {
    // The provider emits a complete <tool_call> envelope and THEN
    // continues with hallucinated tool output and a second speculative
    // envelope. The loop must cut the stream at the first envelope's
    // close so the speculative tail never reaches the conversation.
    // See spec/components/core/agent-loop.md "Tool dispatch concurrency".
    let provider = MockProvider::new(vec![
        // Turn 2 (popped last): the loop's response after the real tool
        // result feeds back. Just finishes.
        vec![
            ChatEvent::TextDelta("done".into()),
            ChatEvent::Finish {
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ],
        // Turn 1 (popped first): prose, real envelope, then SPECULATIVE
        // continuation (hallucinated result + second tool_call) that
        // the loop MUST discard by cutting the stream at the first
        // envelope's close.
        vec![
            ChatEvent::TextDelta("Let me check. ".into()),
            ChatEvent::TextDelta(
                "<tool_call>{\"name\":\"current_time\",\"arguments\":{}}</tool_call>".into(),
            ),
            ChatEvent::TextDelta(" The time is 12:34 UTC.".into()),
            ChatEvent::TextDelta(
                "<tool_call>{\"name\":\"current_time\",\"arguments\":{}}</tool_call>".into(),
            ),
            ChatEvent::Finish {
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ],
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(CurrentTimeStub {
        fixed: "2026-06-03T12:00:00Z".into(),
    }));
    let mut conv = Conversation::new();
    conv.push_user_text("what time is it?");

    let outcome = run(
        &provider,
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
        |_| {},
    )
    .await
    .unwrap();

    // Exactly one tool call was dispatched. The second (speculative)
    // envelope must NOT have been picked up.
    assert_eq!(
        outcome.tool_calls_dispatched, 1,
        "speculative second envelope must not dispatch; got {} total",
        outcome.tool_calls_dispatched
    );

    // Conversation shape: user, assistant(turn 1), tool_result, assistant(turn 2).
    assert_eq!(
        conv.messages.len(),
        4,
        "expected 4 messages, got {}: {:?}",
        conv.messages.len(),
        conv.messages
    );

    // The first assistant turn's text must include the prose that came
    // BEFORE the envelope, and must NOT include the speculative
    // continuation that came AFTER the envelope close.
    let Message::Assistant { content, .. } = &conv.messages[1] else {
        panic!("expected assistant at index 1, got {:?}", conv.messages[1]);
    };
    let assistant_text: String = content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        assistant_text.contains("Let me check."),
        "pre-envelope prose should survive, got: {assistant_text:?}"
    );
    assert!(
        !assistant_text.contains("The time is 12:34"),
        "speculative continuation must be dropped (stream was not cut), got: {assistant_text:?}"
    );
}

#[tokio::test]
async fn on_commit_fires_after_every_conversation_push() {
    // Two-iteration scenario:
    //   turn 1: assistant calls current_time → push_assistant + push_tool_result
    //   turn 2: assistant emits text and finishes → push_assistant
    //
    // We expect on_commit to fire 3 times, observing message counts
    // 2, 3, 4 (the initial conversation already has the user message).
    let provider = MockProvider::new(vec![
        vec![
            ChatEvent::TextDelta("ok".into()),
            ChatEvent::Finish {
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ],
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

    let commit_snapshots = std::sync::Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
    let commit_snapshots_for_cb = commit_snapshots.clone();

    let _ = run(
        &provider,
        std::sync::Arc::new(registry),
        ctx(),
        &mut conv,
        &AgentLoopConfig::new("m"),
        |_| {},
        move |c: &Conversation| {
            commit_snapshots_for_cb
                .lock()
                .unwrap()
                .push(c.messages.len());
        },
    )
    .await
    .unwrap();

    // After turn 1 push_assistant: 2 messages (user, assistant_with_tool_use).
    // After turn 1 push_tool_result: 3 messages.
    // After turn 2 push_assistant: 4 messages.
    let snapshots = commit_snapshots.lock().unwrap().clone();
    assert_eq!(
        snapshots,
        vec![2, 3, 4],
        "on_commit must fire after each push_assistant and push_tool_result \
         so the GUI sees in-flight tool results before run() returns"
    );
    assert_eq!(conv.messages.len(), 4);
}
