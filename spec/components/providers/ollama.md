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

- Default `base_url`: `http://localhost:11434/v1` (Ollama). llama.cpp's server typically listens on `http://localhost:8080/v1` — same path, just configure.
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

## llama.cpp via the same path

Point this provider at llama.cpp's `server` binary (`./server -m model.gguf --port 8080 --api`) and it works. No separate `LlamaCppProvider` needed.
