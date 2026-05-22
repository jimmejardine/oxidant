---
id: ollama
kind: component
parent: overview
order: 3
implements:
  - contracts/provider
depends_on:
  - contracts/provider
  - components/providers/openai
code:
  - crates/oxidant-providers/src/ollama.rs
status: active
responsibility: |
  Talk to a local Ollama (or llama.cpp) server via its OpenAI-compatible endpoint.
---

A thin wrapper around [[components/providers/openai]] with conservative defaults for local servers.

## Configuration

- Default `base_url`: `http://localhost:11434/v1` (Ollama). llama.cpp's `server` listens on `http://localhost:8080/v1`, LM Studio on `http://localhost:1234/v1`, text-generation-webui (oobabooga) on `http://localhost:5000/v1` — same path, just configure.
- Auth: none by default; bearer token configurable for protected deployments.

## Capability defaults (vs OpenAI)

- `tool_use`: model-dependent. Detected at first use via a no-op probe; cached per model.
- `prompt_cache: false`
- `extended_thinking: false`
- `vision`: model-dependent (vision-capable Ollama models advertise via `/api/show`).
- `max_context_tokens`: queried from `/api/show`; default 8192 if unavailable.

## Why a separate provider rather than just configuring OpenAI

Most code paths overlap, but:
- Ollama-specific endpoints (`/api/show`, `/api/tags`, `/api/pull`) are useful for model management UX.
- Robust handling of missing fields (no `usage`, sparse `tool_calls`) differs from OpenAI.
- A distinct `name()` ("ollama") is helpful in logs and the GUI.

These differences are small enough that internally `OllamaProvider` holds an `OpenAIProvider` and overrides only the deltas.

## Same path for llama.cpp, LM Studio, and text-generation-webui

This component is the catch-all for local OpenAI-compatible servers. Point it at llama.cpp's `server` binary (`./server -m model.gguf --port 8080 --api`), LM Studio's local server (typically `http://localhost:1234/v1`), text-generation-webui (oobabooga) with the OpenAI extension enabled (`http://localhost:5000/v1`), or any other OpenAI-compatible local endpoint — no separate per-runner provider type needed. The type name `OllamaProvider` is historical; the responsibility is "local OpenAI-compatible server with conservative defaults".

## Server quirks to tolerate

OpenAI-compatible local servers vary in how strictly they follow the streaming spec. Known quirks the provider absorbs:

- **Split `finish_reason` and `usage`**: text-generation-webui emits `finish_reason` in one chunk and `usage` in a subsequent empty-choices chunk. The provider accumulates both and emits exactly one `Finish` ChatEvent — see [[contracts/provider]] for the contract that streams complete with exactly one `Finish`.
- **Omitted `usage`**: Ollama may not include usage at all. `Finish.usage` falls back to `Usage::default()`.
- **Sparse `tool_calls`**: some models emit malformed or no `tool_calls` even when the request includes tool schemas. The provider yields whatever the model produces; the agent loop is responsible for handling missing/invalid tool calls.
