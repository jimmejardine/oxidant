```yaml
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
```

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

Extraction is **incremental during the stream** — every `ChatEvent::TextDelta` triggers a scan over the suffix of `acc.text` that hasn't yet been searched. The instant a complete envelope is found, its tool call is dispatched eagerly via the same `tokio::spawn` path the native streaming events use ([[components/core/agent-loop]] "Tool dispatch concurrency"). The end-of-stream `absorb_text_tool_calls` becomes a safety-net fallback that fires only when no envelope was successfully extracted incrementally.

### Incremental scan

The core helper is `text_tool_calls::find_next(text, from) -> FindResult`:

```rust
pub enum FindResult {
    /// No opening token at or after `from`. Caller advances cursor to text.len() and stops.
    NoOpen,
    /// Open token at `open_at` but no matching close yet — wait for more deltas.
    /// Caller leaves cursor at `open_at` so the next scan picks up where this one paused.
    Incomplete { open_at: usize },
    /// Full envelope found.  `range` is the byte-range to strip; `parsed` is
    /// `Some` on success, `None` when the body failed to parse (advance past it).
    Complete { range: std::ops::Range<usize>, parsed: Option<ExtractedToolCall> },
}
```

After every `TextDelta`, the agent loop calls `find_next` in a tight loop until it returns `NoOpen` or `Incomplete`. For each `Complete { parsed: Some(call) }`:
1. Synthesise a `PendingToolCall` with id `text_extracted_{n}` (the per-turn counter), push it onto `acc.order` and into `acc.tool_calls`.
2. `tokio::spawn` `registry.invoke(name, args, ctx)`; drop the `JoinHandle` into `pending` keyed by the synthetic id.
3. Record `range` in `extracted_ranges` for the end-of-stream strip.

For each `Complete { parsed: None }` (malformed body), the loop logs at `tracing::warn` and advances past the range without dispatching — the bytes stay in `acc.text` for the user to see.

### End-of-stream

After the stream's `Finish`:
1. Sort `extracted_ranges` by `start` descending; `replace_range` each one with `"\n"` so earlier indices don't shift.
2. If `acc.order` is empty and `looks_like_text_tool_call(&acc.text)` is still true, run the legacy whole-text `extract()` via `absorb_text_tool_calls` as a fallback. This catches the vanishingly rare case where an envelope was `Incomplete` at every delta and its close only arrived in the final chunk after the last incremental scan.

### Why "lossy on the text side"

Extracted envelope bytes are removed from the assistant's `ContentBlock::Text` so the committed transcript shows clean prose with a tool_use card next to it, not the literal XML. Mid-stream the live-turn UI still shows the raw `<tool_call>` text in `LiveTurn.text` — synthesising `ChatEvent::ToolUseStart/End` so the live-turn renders a tool card is recorded as a follow-up.

## Out of scope

- Per-model dispatch tables. The detection is by-content, not by-model — a Hermes envelope coming from a model nobody told us about still works.
- Synthetic `ChatEvent::ToolUseStart/InputDelta/ToolUseEnd` for text-extracted dispatches so the live-turn UI renders a tool card during streaming instead of the raw envelope text. Worth doing as a follow-up; v1 ships the wall-clock concurrency win and accepts that the live turn still shows the XML mid-stream.
- Repairing malformed JSON in Format A. If the inner JSON is invalid, the block stays in the text and `parse_tool_input`'s normal fallback (empty object) doesn't fire because no `PendingToolCall` was created. The user sees the garbled call and can re-prompt.
