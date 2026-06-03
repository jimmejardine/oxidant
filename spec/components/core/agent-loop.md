---
id: agent-loop
kind: component
parent: overview
order: 1
implements: []
depends_on:
  - contracts/provider
  - contracts/tool
  - components/core/tool-registry
  - components/core/conversation
  - components/core/agent-mode
code:
  - crates/oxidant-core/src/agent_loop.rs
tests:
  - crates/oxidant-core/src/agent_loop.rs
  - crates/oxidant-core/tests/agent_loop_mock.rs
status: active
responsibility: |
  Drive the per-exploration conversation: send to the Provider, stream events, dispatch tool calls through the registry, append results, repeat until stop.
---

One agent loop per exploration. Owns the streaming side of the conversation; the registry owns dispatch; the provider owns the network call. Surfaces every event as a `ChatEvent` to the GUI via `mpsc::UnboundedSender`.

## Loop shape

```
loop {
    let stream = provider.chat(build_request(&convo, &registry)).await?;
    while let Some(event) = stream.next().await {
        forward_to_gui(event);
        match event {
            TextDelta | ThinkingDelta => append_to_assistant_turn(),
            ToolUseStart{..} | ToolUseInputDelta{..} | ToolUseEnd{..} => accumulate_tool_call(),
            Finish{ stop_reason, .. } => {
                let pending = drain_pending_tool_calls();
                if pending.is_empty() { return Ok(()); }
                for call in pending {
                    let result = registry.invoke(&call.name, call.input, &ctx).await;
                    convo.push_tool_result(call.id, result);
                    forward_to_gui(ToolCallCompleted { call, result });
                }
                // continue outer loop: provider will see tool_result in next request
            }
            Error(e) => return Err(e),
        }
    }
}
```

## Tool-call accumulation

Tool-use deltas may arrive as fragmented JSON over many SSE events. The loop concatenates `ToolUseInputDelta.json_delta` per `id` and parses at `ToolUseEnd`. Parse failure → synthesise a `ToolResult::Err` instead of throwing, so the model gets a chance to retry.

## Tool dispatch concurrency

Tool dispatch is **eager and parallel** on both paths the loop knows about — native streaming events AND text-extracted envelopes from text-only models.

**Native path** (`ChatEvent::ToolUseEnd { id }`): the instant the event arrives — well before the model finishes streaming the rest of its turn — the loop parses the tool's input and `tokio::spawn`s `registry.invoke(name, input, ctx)` into a `JoinHandle`, keyed by call id, alongside its start `Instant`. The stream continues to accumulate any remaining text, thinking, and further tool_use events in parallel with the tool actually running.

**Text-extracted path** (Qwen / Hermes / smolagents `<tool_call>` envelopes): on every `ChatEvent::TextDelta`, after appending to `acc.text`, the loop runs an incremental scan via `text_tool_calls::find_next` over the unsearched suffix. The instant a complete envelope lands (open + matching close + parseable body), the loop synthesises a `PendingToolCall` with id `text_extracted_{n}`, pushes onto `acc.order`, and `tokio::spawn`s the same way the native path does — into the same `pending: HashMap<String, (Instant, JoinHandle)>`. After the envelope byte-ranges are stripped, the committed `Message::Assistant` doesn't carry the literal XML. See [[components/core/text-tool-call-extraction]]. Eager `tokio::spawn` on a complete envelope is structurally symmetric with the native `ToolUseEnd` path; since the cut (below) breaks the outer loop the moment `Complete { Some(_) }` fires, the spawn today races only the post-stream cleanup and the wall-clock benefit for text-extracted calls is microseconds — the shape is retained so a future relaxed cut rule (or a multi-envelope-per-turn model) needs no rewire.

Once a complete text-extracted envelope lands (parseable body), the loop sets `acc.stop_reason = Some(ToolUse)` and **breaks out of the stream-consumption phase for this turn**. Text-only models aren't trained to stop after `</tool_call>` the way native tool-use providers do; left to keep generating, they emit speculative content — most commonly hallucinated tool output, then further tool calls premised on that imagined output. Cutting the stream forces the real tool result to feed back as the next turn before the model can act on assumed values. Native `ToolUseEnd` events do **not** cut: providers that emit them (Anthropic, OpenAI) are explicit about parallel tool use and the loop preserves that intent. Parse failures (`Complete { parsed: None }`) also do not cut — the model retains its chance to either retry the call or end the turn cleanly.

After the stream's `Finish` event:

1. `conv.push_assistant(content, stop_reason, usage)` commits the assistant message first — `Message::Assistant` must precede any `Message::ToolResult` in the conversation so providers see a consistent prior-turn history on the next request.
2. The loop walks `acc.order` (the emit-order list of call ids), awaits each `JoinHandle`, computes `elapsed_ms = start.elapsed()` from the captured Instant, and calls `conv.push_tool_result(id, content, is_error, elapsed_ms)`. Awaiting in `acc.order` preserves emit order regardless of which tool actually completes first.
3. A `JoinError` (panic in the spawned task) becomes `ToolResult::Err("panic: …")` so the model gets a clean error instead of the loop dying.

Why this is safe even for `Mutating` tools running while the model still streams: the model's emission for the current turn is already encoded server-side by the time `ToolUseEnd` (or the incremental text-extracted envelope's close) arrives — the model can't observe and re-plan based on tools running concurrently. The tool runs no later than the previous "wait-for-Finish" design; only earlier. Wall-clock savings = `Finish_time − dispatch_time` per tool, which for wordy models or multi-tool turns is often several seconds.

Cancellation: spawned tasks receive a `ctx` clone that owns the same `CancellationToken`. A user cancel still short-circuits in-flight tool calls. If the agent-loop future itself is dropped, the spawned tasks are no longer polled and quiesce — tokio's default behaviour.

Conversation commits fire a second callback so the GUI sees the loop's progress in real time. `run()` takes an `on_commit: G where G: FnMut(&Conversation)` alongside the existing `on_event: F where F: FnMut(&ChatEvent)`. `on_commit` is invoked immediately after every `conv.push_*` call inside the loop:

- after `conv.push_assistant(...)` once per iteration,
- after every `conv.push_tool_result(...)` as each spawned tool's `JoinHandle` resolves,
- after the post-edit hook's synthetic `conv.push_user_text(...)`.

Without this callback the host (`drive_agent` in [[components/gui/chat-input-panel]]) only sees the final conversation when `run()` returns — even though tool results are pushed to `conv` as each handle resolves — because the host clones the conversation in, runs the loop on the clone, and only copies it back after `run()` returns. With the callback the host publishes each snapshot to `SharedState` mid-flight, so a long tool that completes well before the model finishes streaming shows up in the transcript the moment it lands rather than after the whole turn is over.

The transcript ([[components/gui/transcript-tab]]) shows `⟳ pending dispatch…` on a tool_use card whose result hasn't landed yet so the user can tell the tool is queued / running rather than the UI being frozen.

## Termination

The loop returns when:
- `Finish { stop_reason: EndTurn }` with no pending tool calls.
- `Finish { stop_reason: StopSequence | MaxTokens }`.
- The user cancels via the GUI (cancellation token in `ToolContext`).
- A provider error event arrives.

## Post-edit hook

After dispatching any turn that included a `Mutating`-category tool call, the loop may invoke a configured ReadOnly check tool (typically [[tools/spec/spec-diff]]). The check's result is appended to the conversation as a synthetic `User` text message — prefixed `[oxidant post-edit check]` — so the model sees it in its next request and can act on any flagged issues (drift, missing code paths, broken invariants) before declaring the work done.

Wired via `AgentLoopConfig::post_edit_check_tool: Option<String>`. Default `None`. When set, the named tool is invoked through the same registry as model-driven tool calls — same permission gating, same panic-catching dispatch. The hook does not fire if a turn used only `ReadOnly` tools (no mutation, nothing to drift).

This is the agent-loop side of [[decisions/0008-spec-is-canonical]]: drift is detected mechanically, surfaced immediately, and the model is given a chance to resolve before the next user turn.

## Cancellation

Each loop runs on a tokio task spawned by `oxidant-core`. Cancellation: drop the task handle. Any in-flight tool call sees `ctx.is_cancelled()` and short-circuits.

## Per-exploration isolation

The loop is parameterised by `Exploration`, which carries the working tree path, LSP handle, and conversation. See [[components/core/exploration]] and [[invariants/explorations-are-isolated]].
