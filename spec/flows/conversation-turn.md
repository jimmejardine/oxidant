```yaml
id: conversation-turn
kind: flow
parent: overview
order: 5
status: active
responsibility: |
  Central narrative: user submits a prompt, the agent loop drives a turn (provider stream → tool dispatch → repeat), and the result lands back in the GUI transcript. Every other flow is a refinement of one step in this one.
depends_on:
  - contracts/provider
  - contracts/tool
  - components/core/agent-loop
  - components/core/conversation
  - components/core/tool-registry
  - components/gui/chat-input-panel
  - components/gui/transcript-tab
  - tools/spec/spec-diff
```

# Run a conversation turn

The agent's heartbeat. One full turn = one user message in, zero-or-more provider iterations, one assistant response committed to history. Every product surface (GUI, CLI, eventually MCP) routes through this loop.

## Trigger

User types into the [[components/gui/chat-input-panel]] and hits Ctrl+Enter (or clicks Send). Programmatic triggers (CLI `oxidant chat`, scheduled jobs) are equivalent — the entry point is `agent_loop::run` either way.

## Steps

1. **Stage the prompt.** ChatInputPanel pushes the user text onto [[components/core/conversation]], snapshots the conversation + registry, and creates a fresh per-turn `CancellationToken`. `live_turn` is set so the spinner shows.

2. **Spawn the agent task.** A tokio task calls [[components/core/agent-loop]]::run with `(provider, registry, ctx, conv, config)`. The GUI thread returns to rendering; events flow back via an `mpsc::UnboundedSender<AgentEvent>`.

3. **Iteration boundary.** Inside the loop, build a `ChatRequest` from the current conversation + the registry's tool specs + the configured system prompt. Send via `provider.chat(request)` and receive a `BoxStream<ChatEvent>`.

4. **Stream the response.** Pull events:
   - `TextDelta` / `ThinkingDelta` → append to `LiveTurn` (the transcript panel re-renders).
   - `ToolUseStart` / `ToolUseInputDelta` / `ToolUseEnd` → accumulate input JSON per call id.
   - `Finish { stop_reason, usage }` → close the iteration.
   - `Error(_)` → abort with `provider stream error: …`.

5. **Commit the assistant message.** Whatever accumulated (text + thinking + tool_use blocks) is pushed onto `conv` as an Assistant message with `stop_reason` and `usage`.

6. **Dispatch tool calls (if any).** For each pending tool call:
   - Parse JSON arguments (malformed → empty object, treated as a real call).
   - `registry.invoke(name, input, ctx).await` — schema validation happens inside the registry, [[flows/tool-permission-check]] gates execution.
   - Push the `ToolResult` back as a User message with `ToolResultContent`.
   - Track whether any call was `Mutating` (drives the post-edit hook).

7. **Post-edit hook.** If any Mutating tool fired and `config.post_edit_check_tool` is set (default: `spec_diff`), [[flows/mutating-edit]] runs that hook now. Its output is appended as a synthetic User message so the model sees it on the next iteration.

8. **Loop or finish.** If the iteration produced no tool calls, return — the turn is done. Otherwise go to step 3 with the updated conversation, up to `max_iterations` (default 30; exhausting it returns `Err`). The chat input panel detects this specific error and surfaces a **Continue iterating** button — see [[components/gui/chat-input-panel]] — that re-invokes the loop on the same conversation with a higher cap.

9. **Surface the outcome.** The task sends `AgentEvent::Completed(outcome)` to the GUI; `SharedState.live_turn` is cleared, `last_outcome` is recorded, and the transcript shows the committed history.

## Cancellation

The per-turn `CancellationToken` is held on `SharedState.cancellation`. Esc in the chat panel (or the Cancel button) cancels it; the agent task observes via `ctx.is_cancelled()` between iterations, drops the in-flight stream, and emits `Completed` with whatever was committed so far. The conversation state is consistent even mid-turn.

## Invariants preserved

- [[invariants/explorations-are-isolated]] — `ToolContext.workspace_root` scopes every tool call to this exploration's worktree; nothing in the loop reads or writes outside it.
- The conversation pushes are append-only across one turn — partial assistant content is still committed on cancellation/error so the next turn sees a coherent history.

## Common failure modes

- **`max_iterations` exhausted.** The model kept calling tools without finishing. Typically a runaway plan; the outcome's `error` field surfaces this so the GUI shows it, and `TurnOutcome.hit_max_iterations` is set so the chat input can offer a one-click resume with a higher cap.
- **Provider stream `Error(_)`.** Network drop, auth failure, rate limit. Returned as `Err`; the partial assistant message (whatever streamed first) is still committed.
- **Malformed tool input JSON.** The loop substitutes `{}` and dispatches anyway — schema validation in the registry produces the actual error message, which is the most useful signal to the model.

## See also

- [[components/core/agent-loop]] — the loop implementation
- [[flows/mutating-edit]] — what the post-edit hook does
- [[flows/tool-permission-check]] — how the registry gates tool calls
