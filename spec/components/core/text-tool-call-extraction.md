---
id: text-tool-call-extraction
kind: component
parent: overview
order: 6
implements: []
depends_on:
  - components/core/agent-loop
code:
  - crates/oxidant-core/src/text_tool_calls.rs
status: active
responsibility: |
  Recover tool calls that the model emitted as literal text rather than via the provider's native tool-use mechanism. Runs once per turn at end-of-stream, scanning the accumulated text for known XML/JSON tool-call envelopes, and inserts each match into the agent-loop's pending-tool-call set so the existing dispatch path picks it up unchanged.
---

## Motivation

Hosted providers (Anthropic, OpenAI, the official Anthropic-API-style frontends) emit tool calls as **structured stream events** — `ChatEvent::ToolUseStart` / `ToolUseInputDelta` / `ToolUseEnd`. Local providers running smaller open-source models (Qwen3, Llama-3, smolagents-trained, Hermes-trained) frequently do **not**. They emit the call as literal text inside the assistant's reply, e.g.:

```
Let me tackle this. <tool_call>
<function=fs_write>
<parameter=file>foo.md</parameter>
<parameter=content>...</parameter>
</function>
</tool_call>
```

Without extraction, oxidant treats the whole thing as plain text — the tool never fires, the transcript is full of XML noise, and the agent loop terminates with `StopReason::EndOfTurn` even though the model clearly intended to act.

This component bridges the gap by recognising the two most common text envelopes and converting them into the same `PendingToolCall` entries the streaming path produces.

## Recognised formats

### Format A — Hermes / Qwen3 JSON-in-tag

```
<tool_call>
{"name": "fs_write", "arguments": {"file": "foo.md", "content": "…"}}
</tool_call>
```

The body is a JSON object. The parser accepts both `arguments` and `parameters` as the args key (different model families use different names).

### Format B — smolagents / Qwen3-Coder function-and-parameters

```
<tool_call>
<function=fs_write>
<parameter=file>
foo.md
</parameter>
<parameter=content>
…
</parameter>
</function>
</tool_call>
```

`<function=NAME>` carries the tool name; each `<parameter=KEY>VALUE</parameter>` is a single argument. Whitespace around `VALUE` is trimmed.

### Outer wrapper variants

Some models omit `<tool_call>…</tool_call>` and emit the inner `<function>` block directly; some wrap it in `` ```xml `` / `` ``` ``. The parser strips those code fences if present and still parses successfully.

## Algorithm

1. Run only when the stream produced zero native tool-use events AND the text contains at least one of the recognised opening tokens (`<tool_call>` or `<function=`).
2. Walk the text linearly, finding each candidate block. Each block is removed from the text and replaced with a single newline so the surrounding prose stays intact.
3. For each block, try Format A first (JSON); if that fails to parse, try Format B (XML).
4. On a successful match, synthesise a `PendingToolCall` with a stable id (`text_extracted_{index}` — uniqueness is per-turn, the agent loop's HashMap doesn't need cross-turn uniqueness) and push it onto the accumulator's `order` vector.
5. On a parse failure, leave the block in the text and log at `tracing::warn` — better to surface the garbled call than to silently drop it.

The parser is intentionally **lossy on the text side**: extracted XML is removed from the assistant's `ContentBlock::Text` so the transcript shows clean prose. The original raw text is logged at `tracing::debug` for diagnostics.

## Out of scope

- Per-model dispatch tables. The detection is by-content, not by-model — a Hermes envelope coming from a model nobody told us about still works.
- Streaming extraction. The parser runs once at end-of-turn; tool cards appear when the message commits, not while the XML is still streaming. The window of visible XML is brief and acceptable for v1.
- Repairing malformed JSON in Format A. If the inner JSON is invalid, the block stays in the text and `parse_tool_input`'s normal fallback (empty object) doesn't fire because no `PendingToolCall` was created. The user sees the garbled call and can re-prompt.
