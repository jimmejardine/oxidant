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
code:
  - crates/oxidant-core/src/agent_loop.rs
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

## Termination

The loop returns when:
- `Finish { stop_reason: EndTurn }` with no pending tool calls.
- `Finish { stop_reason: StopSequence | MaxTokens }`.
- The user cancels via the GUI (cancellation token in `ToolContext`).
- A provider error event arrives.

## Cancellation

Each loop runs on a tokio task spawned by `oxidant-core`. Cancellation: drop the task handle. Any in-flight tool call sees `ctx.is_cancelled()` and short-circuits.

## Per-exploration isolation

The loop is parameterised by `Exploration`, which carries the working tree path, LSP handle, and conversation. See [[components/core/exploration]] and [[invariants/explorations-are-isolated]].
