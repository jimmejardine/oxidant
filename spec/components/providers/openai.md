---
id: openai
kind: component
parent: overview
order: 2
implements:
  - contracts/provider
depends_on:
  - contracts/provider
code:
  - crates/oxidant-providers/src/openai.rs
status: active
responsibility: |
  Talk to OpenAI Chat Completions (default) or the Responses API (feature-gated); base path also serves OpenAI-compatible servers.
---

## API specifics

- Default endpoint: `POST https://api.openai.com/v1/chat/completions` with `stream: true`.
- Configurable `base_url` — same code path serves Ollama (see [[components/providers/ollama]]), llama.cpp's server, Azure OpenAI deployments, and other OpenAI-compatible APIs.
- Auth: `Authorization: Bearer ...` from `OPENAI_API_KEY` or settings.

## Capabilities

- `tool_use: true` (function calling)
- `prompt_cache: false` (OpenAI's caching is automatic, not advertised in the request)
- `extended_thinking: false` (reasoning models surface via separate fields; treated as text in MVP)
- `vision: true` for vision-capable models

## Tool use translation

- Request: `tools: [{ type: "function", function: { name, description, parameters } }]`.
- Response: `tool_calls: [{ id, type: "function", function: { name, arguments: "<json string>" } }]`.
- Translation to oxidant's `ChatEvent`: emit `ToolUseStart` on first delta of a tool_call, `ToolUseInputDelta` for each argument chunk, `ToolUseEnd` on completion.

## Responses API (feature flag)

`openai-responses` feature compiles in the alternative `/v1/responses` path. Off by default — the Chat Completions path is more stable for tool use today. Reassess in v2.

## Notes for OpenAI-compatible servers

Some servers (Ollama, llama.cpp) don't implement every field. The provider tolerates missing `usage`, missing `logprobs`, and partial tool-call support. `capabilities()` for those uses the `OllamaProvider` wrapper, which sets sane defaults.
