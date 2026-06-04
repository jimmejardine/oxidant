```yaml
---
id: 0001-multi-provider-llm
kind: decision
order: 1
status: active
date: 2026-05-21
responsibility: |
  Support multiple LLM providers (Anthropic, OpenAI, Ollama, llama.cpp) behind a hand-rolled Provider trait.
---
```

# 0001 — Support multiple LLM providers via a hand-rolled abstraction

## Status

Active.

## Context

Tying oxidant to a single provider would constrain users with strong preferences (privacy → local models; cost → cheaper providers; quality → frontier models) and would make oxidant fragile to a single API breaking change. A multi-provider abstraction is the obvious answer.

Two paths: adopt an existing Rust LLM framework (`rig`, `swiftide`, `genai`), or roll our own trait. The existing crates are all pre-1.0 with frequent breaking releases and carry opinionated abstractions (chains, agents-as-objects, RAG pipelines) we don't need.

## Decision

A hand-rolled `Provider` trait in `oxidant-providers`. The surface is small — chat with streaming, tool use, capabilities probe — and we own it.

Backends in MVP:
- Anthropic Claude (native API, prompt caching, extended thinking)
- OpenAI (Chat Completions; Responses behind a feature flag)
- Ollama (OpenAI-compatible local endpoint)
- llama.cpp server (also OpenAI-compatible, slots into the same backend)

See [[contracts/provider]] and [[decisions/0007-roll-own-llm-provider-layer]] (which records the same instinct applied repeatedly across the project).

## Consequences

Positive: independent of framework release cycles; minimal compile cost; easy to add backends.

Negative: per-provider quirks (streaming framing, tool-call payload shapes) live in our code, not theirs. Mitigation: normalisation layer in `oxidant-providers` keeps the public surface clean.
