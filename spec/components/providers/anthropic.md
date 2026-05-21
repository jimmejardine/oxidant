---
id: anthropic
kind: component
parent: overview
order: 1
implements:
  - contracts/provider
depends_on:
  - contracts/provider
code:
  - crates/oxidant-providers/src/anthropic.rs
status: active
responsibility: |
  Talk to the Anthropic Messages API: streaming, tool use, prompt caching, extended thinking.
---

The primary provider in MVP. Uses the native Messages API directly (not OpenAI-compatible).

## API specifics

- Endpoint: `POST https://api.anthropic.com/v1/messages` with `stream: true`.
- Auth: `x-api-key` header from `ANTHROPIC_API_KEY` env or [[components/config/settings]].
- Versioning: `anthropic-version: 2023-06-01` (current stable). Capability flags via `anthropic-beta` when needed.

## Prompt caching

`cache_control: { type: "ephemeral" }` markers added to:
- The system prompt (large, stable across turns).
- The `tools` array (also large, stable).
- The most recent user message when it's expected to repeat (rare; default off).

`capabilities().prompt_cache = true`. The agent loop uses this to decide whether to mark cacheable blocks; non-caching providers see those markers stripped.

## Extended thinking

`thinking: { type: "enabled", budget_tokens: <N> }` enabled by config (off by default). Streams `ThinkingDelta` events.

`capabilities().extended_thinking = true`.

## Tool use translation

Anthropic's wire format:
- Request: `tools: [{ name, description, input_schema }]`.
- Response: `content` includes `{ type: "tool_use", id, name, input }` blocks.

Translated to/from oxidant's `Tool` and `ChatEvent` shapes one-for-one.

## SSE handling

Hand-rolled, per [[decisions/0007-roll-own-llm-provider-layer]]. Parser is ~150 lines; the API is stable enough that we don't need `reqwest-eventsource`.
