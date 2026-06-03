// Realises spec/components/core/agent-loop.md.
//
// Drives one Conversation: send → stream → accumulate → dispatch tools →
// append results → repeat. One `run()` call processes a turn from the user's
// last message to either an end-of-turn assistant reply (no tool calls) or
// `max_iterations` exhausted. Provider, ToolRegistry, and ToolContext are
// passed in by reference — the loop owns nothing it didn't get from the
// caller, matching the spec's "loop runs on a tokio task" model.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::anyhow;
use futures::StreamExt;
use serde_json::Value;
use tokio::task::JoinHandle;

use oxidant_providers::{
    ChatEvent, ChatRequest, ContentPart, Provider, RequestMessage, Role, StopReason,
    ThinkingConfig, ToolSpec, Usage,
};

use crate::conversation::Conversation;
use crate::message::{ContentBlock, ImageSource, Message, ToolResultContent};
use crate::registry::{ToolCategory, ToolContext, ToolRegistry, ToolResult};
use crate::text_tool_calls::{self, ExtractedToolCall};

#[derive(Debug, Clone)]
pub struct AgentLoopConfig {
    pub model: String,
    pub system_prompt: Option<String>,
    pub max_tokens: u32,
    pub max_iterations: usize,
    pub temperature: Option<f32>,
    pub thinking: Option<ThinkingConfig>,
    /// Name of a ReadOnly tool to invoke after any turn that dispatched
    /// at least one Mutating-category tool. The tool's result is appended
    /// to the conversation as a synthetic User message so the model sees
    /// it on the next iteration. Typically "spec_diff" — see
    /// spec/components/core/agent-loop.md and spec/tools/spec/spec-diff.md.
    pub post_edit_check_tool: Option<String>,
    /// Plan vs Implement. See spec/components/core/agent-mode.md.
    /// Plan filters the registry to ReadOnly tools and appends a
    /// describe-don't-do suffix to the system prompt.
    pub mode: AgentMode,
}

impl AgentLoopConfig {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system_prompt: None,
            max_tokens: 4096,
            max_iterations: 16,
            temperature: None,
            thinking: None,
            post_edit_check_tool: None,
            mode: AgentMode::default(),
        }
    }
}

/// The two interaction modes for the chat agent. See
/// spec/components/core/agent-mode.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentMode {
    /// Read-only tools only; the agent describes what it would do.
    /// Default — the safer side of "I'm not sure what you'll do next".
    #[default]
    Plan,
    /// Full tool access; the agent acts on the workspace.
    Implement,
}

impl AgentMode {
    pub fn flip(self) -> Self {
        match self {
            AgentMode::Plan => AgentMode::Implement,
            AgentMode::Implement => AgentMode::Plan,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgentMode::Plan => "PLAN",
            AgentMode::Implement => "IMPLEMENT",
        }
    }
}

/// Verbatim plan-mode system-prompt suffix. Implementations MUST use this
/// exact text so behaviour is stable across providers — the spec at
/// spec/components/core/agent-mode.md pins the wording.
pub const PLAN_MODE_SYSTEM_PROMPT_SUFFIX: &str = "\n\nYou are currently in PLAN MODE.\n\nUse read-only tools (read files, grep, spec lookups, cargo_check, LSP queries, git log/diff/status, etc.) to investigate as much as you need. Then DESCRIBE the change you would make:\n- the files you'd touch\n- the substantive edits\n- the order you'd do them in\n- and why\n\nDo NOT attempt to mutate files, git state, or the workspace — those tools are not exposed to you in this mode. If you reach for one, the call will fail. The user will switch you to IMPLEMENT mode when they are ready for you to act.";

#[derive(Debug, Clone, Copy, Default)]
pub struct AgentLoopOutcome {
    pub iterations: usize,
    pub stop_reason: Option<StopReason>,
    pub total_usage: Usage,
    pub tool_calls_dispatched: usize,
    /// Number of times the configured post-edit check tool fired across
    /// this run (one per turn that contained any Mutating tool call).
    pub post_edit_checks_fired: usize,
}

/// Run the agent loop until the model returns an end-of-turn response with
/// no pending tool calls, or until `max_iterations` is reached.
///
/// `on_event` is invoked synchronously for every ChatEvent — use it to
/// surface streaming output to the GUI, log, or terminal. The function
/// returns when the loop terminates; cancellation is implicit (drop the
/// future) per the spec.
pub async fn run<F>(
    provider: &dyn Provider,
    registry: Arc<ToolRegistry>,
    ctx: ToolContext,
    conv: &mut Conversation,
    config: &AgentLoopConfig,
    mut on_event: F,
) -> anyhow::Result<AgentLoopOutcome>
where
    F: FnMut(&ChatEvent),
{
    let mut outcome = AgentLoopOutcome::default();

    for iteration in 0..config.max_iterations {
        outcome.iterations = iteration + 1;

        let request = build_request(conv, &registry, config);
        tracing::debug!(
            iter = iteration,
            messages = request.messages.len(),
            tools = request.tools.len(),
            "agent_loop: sending request"
        );

        let mut stream = provider.chat(request).await?;
        let mut acc = TurnAccumulator::default();
        let mut error_text: Option<String> = None;
        // Per-tool join handles, populated on ToolUseEnd and awaited in
        // acc.order after Finish. See "Tool dispatch concurrency" in
        // spec/components/core/agent-loop.md.
        let mut pending: HashMap<String, (Instant, JoinHandle<ToolResult>)> = HashMap::new();
        let mut any_mutating = false;
        // Per-turn state for the incremental text-tool-call scanner.
        // See spec/components/core/text-tool-call-extraction.md.
        let mut text_scan_cursor: usize = 0;
        let mut text_extracted_count: usize = 0;
        let mut extracted_ranges: Vec<std::ops::Range<usize>> = Vec::new();

        while let Some(event) = stream.next().await {
            on_event(&event);
            match event {
                ChatEvent::TextDelta(s) => {
                    acc.text.push_str(&s);
                    // Incremental envelope scan — dispatch text-extracted
                    // tool calls eagerly the same way native ToolUseEnd
                    // does. Loops until find_next reports NoOpen or
                    // Incomplete; advances text_scan_cursor as we go.
                    loop {
                        use text_tool_calls::FindResult;
                        match text_tool_calls::find_next(&acc.text, text_scan_cursor) {
                            FindResult::NoOpen => {
                                text_scan_cursor = acc.text.len();
                                break;
                            }
                            FindResult::Incomplete { open_at } => {
                                text_scan_cursor = open_at;
                                break;
                            }
                            FindResult::Complete { range, parsed: None } => {
                                // Parse failure — advance past, leave
                                // the bytes in acc.text so the user
                                // sees something went wrong.
                                text_scan_cursor = range.end;
                            }
                            FindResult::Complete {
                                range,
                                parsed: Some(call),
                            } => {
                                let id = format!("text_extracted_{text_extracted_count}");
                                text_extracted_count += 1;
                                acc.order.push(id.clone());
                                acc.tool_calls.insert(
                                    id.clone(),
                                    PendingToolCall {
                                        name: call.name.clone(),
                                        input_buffer: call.arguments_json.clone(),
                                    },
                                );
                                if tool_is_mutating(&registry, &call.name) {
                                    any_mutating = true;
                                }
                                let input = parse_tool_input(&call.arguments_json);
                                tracing::debug!(
                                    tool = %call.name,
                                    id = %id,
                                    "dispatching tool (eager text-extracted)"
                                );
                                let registry_for_task = registry.clone();
                                let ctx_for_task = ctx.clone();
                                let name = call.name.clone();
                                let handle = tokio::spawn(async move {
                                    registry_for_task
                                        .invoke(&name, input, &ctx_for_task)
                                        .await
                                });
                                pending.insert(id, (Instant::now(), handle));
                                extracted_ranges.push(range.clone());
                                text_scan_cursor = range.end;
                            }
                        }
                    }
                }
                ChatEvent::ThinkingDelta(s) => acc.thinking.push_str(&s),
                ChatEvent::ToolUseStart { id, name } => {
                    acc.order.push(id.clone());
                    acc.tool_calls.insert(
                        id.clone(),
                        PendingToolCall {
                            name,
                            input_buffer: String::new(),
                        },
                    );
                }
                ChatEvent::ToolUseInputDelta { id, json_delta } => {
                    if let Some(tc) = acc.tool_calls.get_mut(&id) {
                        tc.input_buffer.push_str(&json_delta);
                    }
                }
                ChatEvent::ToolUseEnd { id } => {
                    // Inputs are fully accumulated — kick off the tool
                    // NOW rather than waiting for Finish. The future runs
                    // concurrently with the rest of the stream.
                    if let Some(tc) = acc.tool_calls.get(&id) {
                        let input = parse_tool_input(&tc.input_buffer);
                        if tool_is_mutating(&registry, &tc.name) {
                            any_mutating = true;
                        }
                        tracing::debug!(tool = %tc.name, id = %id, "dispatching tool");
                        let registry_for_task = registry.clone();
                        let ctx_for_task = ctx.clone();
                        let name = tc.name.clone();
                        let handle = tokio::spawn(async move {
                            registry_for_task.invoke(&name, input, &ctx_for_task).await
                        });
                        pending.insert(id, (Instant::now(), handle));
                    }
                }
                ChatEvent::Finish { stop_reason, usage } => {
                    acc.stop_reason = Some(stop_reason);
                    acc.usage = usage;
                }
                ChatEvent::Error(e) => {
                    error_text = Some(e);
                    break;
                }
            }
        }

        if let Some(e) = error_text {
            return Err(anyhow!("provider stream error: {e}"));
        }

        outcome.stop_reason = acc.stop_reason;
        outcome.total_usage.input_tokens += acc.usage.input_tokens;
        outcome.total_usage.output_tokens += acc.usage.output_tokens;
        outcome.total_usage.cache_creation_input_tokens += acc.usage.cache_creation_input_tokens;
        outcome.total_usage.cache_read_input_tokens += acc.usage.cache_read_input_tokens;

        // Strip extracted text-tool-call envelope byte ranges from
        // acc.text so the committed Message::Assistant doesn't carry
        // the literal XML. We sort descending by start so earlier
        // indices stay valid as we splice.
        if !extracted_ranges.is_empty() {
            extracted_ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
            for r in &extracted_ranges {
                if r.end <= acc.text.len() {
                    acc.text.replace_range(r.clone(), "\n");
                }
            }
        }

        // Safety net: if the incremental scanner didn't catch any
        // envelope (e.g. the model only emitted a half-formed one that
        // closed in the final delta, AND the loop's last incremental
        // scan happened before that close arrived), fall back to the
        // whole-text extractor. In practice rare — `extracted_ranges`
        // is non-empty for every Qwen / Hermes turn.
        // See spec/components/core/text-tool-call-extraction.md.
        if acc.order.is_empty() && text_tool_calls::looks_like_text_tool_call(&acc.text) {
            absorb_text_tool_calls(&mut acc);
        }

        let assistant_content = build_assistant_blocks(&acc);
        let has_tool_calls = !acc.order.is_empty();
        conv.push_assistant(assistant_content, acc.stop_reason, Some(acc.usage));

        if !has_tool_calls {
            return Ok(outcome);
        }

        // Tool dispatch: spawned-on-ToolUseEnd above. If text-tool-call
        // extraction produced tool calls AFTER the stream ended, those
        // weren't spawned during the loop — fall back to inline dispatch
        // for any id in acc.order without a pending handle.
        for id in &acc.order {
            let (start, handle) = if let Some(entry) = pending.remove(id) {
                entry
            } else {
                // Text-tool-call fallback: never went through ToolUseEnd
                // because the provider emitted as plain text. Spawn now.
                let tc = acc.tool_calls.get(id).expect("tool call in acc.order");
                let input = parse_tool_input(&tc.input_buffer);
                if tool_is_mutating(&registry, &tc.name) {
                    any_mutating = true;
                }
                tracing::debug!(tool = %tc.name, id = %id, "dispatching (text-tool-call) tool");
                let registry_for_task = registry.clone();
                let ctx_for_task = ctx.clone();
                let name = tc.name.clone();
                let handle = tokio::spawn(async move {
                    registry_for_task.invoke(&name, input, &ctx_for_task).await
                });
                (Instant::now(), handle)
            };
            let result = match handle.await {
                Ok(r) => r,
                Err(join_err) => ToolResult::Err(format!("tool task panicked: {join_err}")),
            };
            let elapsed_ms = start.elapsed().as_millis() as u64;
            outcome.tool_calls_dispatched += 1;
            let (content, is_error) = match result {
                ToolResult::Ok(v) => (ToolResultContent::Json(v), false),
                ToolResult::Err(e) => (ToolResultContent::Text(e), true),
            };
            conv.push_tool_result(id, content, is_error, elapsed_ms);
        }

        // Post-edit hook — see spec/components/core/agent-loop.md.
        if any_mutating && let Some(check_tool) = &config.post_edit_check_tool {
            if registry.iter().any(|t| t.name() == check_tool.as_str()) {
                tracing::debug!(tool = %check_tool, "post-edit check");
                let result = registry
                    .invoke(
                        check_tool,
                        serde_json::Value::Object(Default::default()),
                        &ctx,
                    )
                    .await;
                let message = format_post_edit_check(check_tool, &result);
                conv.push_user_text(message);
                outcome.post_edit_checks_fired += 1;
            } else {
                tracing::warn!("post_edit_check_tool {check_tool:?} not registered; skipping");
            }
        }
    }

    Err(anyhow!(
        "agent loop exceeded max_iterations ({})",
        config.max_iterations
    ))
}

#[derive(Default)]
struct TurnAccumulator {
    text: String,
    thinking: String,
    tool_calls: HashMap<String, PendingToolCall>,
    order: Vec<String>,
    stop_reason: Option<StopReason>,
    usage: Usage,
}

struct PendingToolCall {
    name: String,
    input_buffer: String,
}

/// Scan `acc.text` for text-style tool-call envelopes, replace each
/// recognised envelope with a synthesised `PendingToolCall`, and strip
/// the envelope text so it doesn't leak into the transcript. Per the
/// spec, parse failures leave the offending block in the text rather
/// than dropping it silently.
fn absorb_text_tool_calls(acc: &mut TurnAccumulator) {
    let result = text_tool_calls::extract(&acc.text);
    if result.calls.is_empty() {
        return;
    }
    acc.text = result.stripped_text;
    for (
        i,
        ExtractedToolCall {
            name,
            arguments_json,
        },
    ) in result.calls.into_iter().enumerate()
    {
        let id = format!("text_extracted_{i}");
        acc.order.push(id.clone());
        acc.tool_calls.insert(
            id,
            PendingToolCall {
                name,
                input_buffer: arguments_json,
            },
        );
    }
    tracing::debug!(
        "text_tool_calls: absorbed {} call(s) from assistant text",
        acc.order.len()
    );
}

fn tool_is_mutating(registry: &ToolRegistry, name: &str) -> bool {
    registry
        .iter()
        .any(|t| t.name() == name && matches!(t.category(), ToolCategory::Mutating))
}

fn format_post_edit_check(tool_name: &str, result: &ToolResult) -> String {
    match result {
        ToolResult::Ok(v) => format!(
            "[oxidant post-edit check via `{tool_name}`]\n{}",
            serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
        ),
        ToolResult::Err(e) => format!("[oxidant post-edit check `{tool_name}` failed]\n{e}"),
    }
}

fn parse_tool_input(buf: &str) -> Value {
    if buf.trim().is_empty() {
        return Value::Object(serde_json::Map::new());
    }
    match serde_json::from_str(buf) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "malformed tool input JSON ({e}); falling back to empty object. raw={buf}"
            );
            Value::Object(serde_json::Map::new())
        }
    }
}

fn build_assistant_blocks(acc: &TurnAccumulator) -> Vec<ContentBlock> {
    let mut content = Vec::new();
    if !acc.thinking.is_empty() {
        content.push(ContentBlock::Thinking(acc.thinking.clone()));
    }
    if !acc.text.is_empty() {
        content.push(ContentBlock::Text(acc.text.clone()));
    }
    for id in &acc.order {
        let tc = acc.tool_calls.get(id).expect("tool call we just inserted");
        content.push(ContentBlock::ToolUse {
            id: id.clone(),
            name: tc.name.clone(),
            input: parse_tool_input(&tc.input_buffer),
        });
    }
    content
}

/// Translate a Conversation + Registry + Config into a provider-agnostic ChatRequest.
///
/// Internal Conversation::Message::ToolResult variants don't have a direct
/// equivalent in our normalized RequestMessage (which only has User|Assistant
/// roles). The translator batches consecutive ToolResult messages into a
/// single User RequestMessage carrying ContentPart::ToolResult parts — the
/// provider layer then splits those back out into "tool" role messages
/// (OpenAI) or `tool_result` content blocks (Anthropic).
pub fn build_request(
    conv: &Conversation,
    registry: &ToolRegistry,
    config: &AgentLoopConfig,
) -> ChatRequest {
    let mut messages = Vec::<RequestMessage>::new();
    let mut tool_result_buf = Vec::<ContentPart>::new();

    for msg in &conv.messages {
        match msg {
            Message::User { content } => {
                flush_tool_results(&mut tool_result_buf, &mut messages);
                let parts: Vec<ContentPart> = content.iter().filter_map(block_to_part).collect();
                if !parts.is_empty() {
                    messages.push(RequestMessage {
                        role: Role::User,
                        content: parts,
                    });
                }
            }
            Message::Assistant { content, .. } => {
                flush_tool_results(&mut tool_result_buf, &mut messages);
                let parts: Vec<ContentPart> = content.iter().filter_map(block_to_part).collect();
                messages.push(RequestMessage {
                    role: Role::Assistant,
                    content: parts,
                });
            }
            Message::ToolResult {
                call_id,
                content,
                is_error,
                elapsed_ms: _,
            } => {
                tool_result_buf.push(ContentPart::ToolResult {
                    call_id: call_id.clone(),
                    content: content.as_string(),
                    is_error: *is_error,
                });
            }
        }
    }
    flush_tool_results(&mut tool_result_buf, &mut messages);

    // Per spec/components/core/agent-mode.md: Plan mode hides every
    // non-ReadOnly tool from the model. The model literally can't
    // pick a tool it doesn't see, and a text-extracted call to a
    // hidden tool falls through to the unknown-tool path.
    let tools: Vec<ToolSpec> = registry
        .iter()
        .filter(|tool| match config.mode {
            AgentMode::Plan => matches!(tool.category(), ToolCategory::ReadOnly),
            AgentMode::Implement => true,
        })
        .map(|tool| ToolSpec {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            input_schema: tool.schema(),
        })
        .collect();

    // In Plan mode, append the describe-don't-do suffix to whatever
    // system prompt the caller supplied.
    let system = match config.mode {
        AgentMode::Plan => Some(match &config.system_prompt {
            Some(base) => format!("{base}{PLAN_MODE_SYSTEM_PROMPT_SUFFIX}"),
            None => PLAN_MODE_SYSTEM_PROMPT_SUFFIX.trim_start().to_string(),
        }),
        AgentMode::Implement => config.system_prompt.clone(),
    };

    ChatRequest {
        model: config.model.clone(),
        system,
        messages,
        tools,
        max_tokens: config.max_tokens,
        temperature: config.temperature,
        thinking: config.thinking,
    }
}

fn flush_tool_results(buf: &mut Vec<ContentPart>, out: &mut Vec<RequestMessage>) {
    if !buf.is_empty() {
        out.push(RequestMessage {
            role: Role::User,
            content: std::mem::take(buf),
        });
    }
}

fn block_to_part(block: &ContentBlock) -> Option<ContentPart> {
    match block {
        ContentBlock::Text(s) => Some(ContentPart::Text(s.clone())),
        ContentBlock::Thinking(s) => Some(ContentPart::Thinking(s.clone())),
        ContentBlock::ToolUse { id, name, input } => Some(ContentPart::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        }),
        ContentBlock::Image {
            source: _,
            media_type: _,
        } => {
            // ImageSource isn't supported by the local provider path; drop with a debug log.
            // Vision will be wired through when oxidant-providers gains a vision content variant.
            let _ = ImageSource::Base64(String::new()); // keep ImageSource in scope
            tracing::debug!(
                "dropping ContentBlock::Image (vision not wired through MVP provider path)"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_batches_tool_results_into_user_message() {
        let mut conv = Conversation::new();
        conv.push_user_text("hi");
        conv.push_assistant(
            vec![
                ContentBlock::Text("calling".into()),
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "x".into(),
                    input: serde_json::json!({}),
                },
                ContentBlock::ToolUse {
                    id: "t2".into(),
                    name: "x".into(),
                    input: serde_json::json!({}),
                },
            ],
            None,
            None,
        );
        conv.push_tool_result("t1", ToolResultContent::Text("a".into()), false, 0);
        conv.push_tool_result("t2", ToolResultContent::Text("b".into()), false, 0);

        let registry = ToolRegistry::new();
        let req = build_request(&conv, &registry, &AgentLoopConfig::new("test-model"));
        assert_eq!(req.messages.len(), 3);
        assert_eq!(req.messages[0].role, Role::User);
        assert_eq!(req.messages[1].role, Role::Assistant);
        assert_eq!(req.messages[2].role, Role::User);
        // Two ToolResult parts batched into the trailing user message
        assert_eq!(req.messages[2].content.len(), 2);
        assert!(matches!(
            req.messages[2].content[0],
            ContentPart::ToolResult { ref call_id, .. } if call_id == "t1"
        ));
    }

    #[test]
    fn build_request_omits_empty_user_message() {
        // A user message containing only an Image block (currently dropped)
        // should not produce an empty user message in the request.
        let mut conv = Conversation::new();
        conv.push_user_content(vec![ContentBlock::Image {
            source: ImageSource::Base64("xxx".into()),
            media_type: "image/png".into(),
        }]);
        let registry = ToolRegistry::new();
        let req = build_request(&conv, &registry, &AgentLoopConfig::new("m"));
        assert!(req.messages.is_empty());
    }

    // ----- AgentMode + mode-aware build_request --------------------------

    struct DummyTool {
        name_: &'static str,
        category_: ToolCategory,
    }

    #[async_trait::async_trait]
    impl crate::registry::Tool for DummyTool {
        fn name(&self) -> &str {
            self.name_
        }
        fn description(&self) -> &str {
            "test tool"
        }
        fn schema(&self) -> Value {
            serde_json::json!({"type":"object","properties":{}})
        }
        fn category(&self) -> ToolCategory {
            self.category_
        }
        async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::Ok(Value::Null)
        }
    }

    fn registry_with_one_of_each() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register(std::sync::Arc::new(DummyTool {
            name_: "read_only_tool",
            category_: ToolCategory::ReadOnly,
        }));
        r.register(std::sync::Arc::new(DummyTool {
            name_: "mutating_tool",
            category_: ToolCategory::Mutating,
        }));
        r.register(std::sync::Arc::new(DummyTool {
            name_: "network_tool",
            category_: ToolCategory::Network,
        }));
        r
    }

    #[test]
    fn agent_mode_default_is_plan() {
        assert_eq!(AgentMode::default(), AgentMode::Plan);
    }

    #[test]
    fn agent_mode_flip_is_involutive() {
        assert_eq!(AgentMode::Plan.flip().flip(), AgentMode::Plan);
        assert_eq!(AgentMode::Implement.flip().flip(), AgentMode::Implement);
        assert_eq!(AgentMode::Plan.flip(), AgentMode::Implement);
        assert_eq!(AgentMode::Implement.flip(), AgentMode::Plan);
    }

    #[test]
    fn build_request_in_plan_mode_excludes_non_readonly_tools() {
        let conv = Conversation::new();
        let registry = registry_with_one_of_each();
        let mut config = AgentLoopConfig::new("m");
        config.mode = AgentMode::Plan;

        let req = build_request(&conv, &registry, &config);
        let names: Vec<&str> = req.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_only_tool"));
        assert!(
            !names.contains(&"mutating_tool"),
            "mutating_tool should be hidden in Plan mode; got {names:?}"
        );
        assert!(
            !names.contains(&"network_tool"),
            "network_tool should be hidden in Plan mode; got {names:?}"
        );
    }

    #[test]
    fn build_request_in_implement_mode_exposes_every_tool() {
        let conv = Conversation::new();
        let registry = registry_with_one_of_each();
        let mut config = AgentLoopConfig::new("m");
        config.mode = AgentMode::Implement;

        let req = build_request(&conv, &registry, &config);
        let names: Vec<&str> = req.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_only_tool"));
        assert!(names.contains(&"mutating_tool"));
        assert!(names.contains(&"network_tool"));
    }

    #[test]
    fn build_request_in_plan_mode_appends_plan_system_prompt() {
        let conv = Conversation::new();
        let registry = ToolRegistry::new();
        let mut config = AgentLoopConfig::new("m");
        config.mode = AgentMode::Plan;
        config.system_prompt = Some("Be terse.".to_string());

        let req = build_request(&conv, &registry, &config);
        let system = req
            .system
            .expect("plan mode should produce a system prompt");
        assert!(
            system.starts_with("Be terse."),
            "caller's prompt should lead: {system:?}"
        );
        assert!(
            system.contains("PLAN MODE"),
            "plan-mode marker should be present: {system:?}"
        );
    }

    #[test]
    fn build_request_in_plan_mode_uses_suffix_alone_when_caller_provided_no_prompt() {
        let conv = Conversation::new();
        let registry = ToolRegistry::new();
        let mut config = AgentLoopConfig::new("m");
        config.mode = AgentMode::Plan;
        config.system_prompt = None;

        let req = build_request(&conv, &registry, &config);
        let system = req
            .system
            .expect("plan mode should produce a system prompt");
        assert!(system.contains("PLAN MODE"));
        // Suffix's natural leading whitespace is trimmed when standalone.
        assert!(system.starts_with("You are currently in PLAN MODE"));
    }

    #[test]
    fn build_request_in_implement_mode_passes_system_prompt_through_unchanged() {
        let conv = Conversation::new();
        let registry = ToolRegistry::new();
        let mut config = AgentLoopConfig::new("m");
        config.mode = AgentMode::Implement;
        config.system_prompt = Some("Just be a normal assistant.".to_string());

        let req = build_request(&conv, &registry, &config);
        assert_eq!(req.system.as_deref(), Some("Just be a normal assistant."));
    }
}
