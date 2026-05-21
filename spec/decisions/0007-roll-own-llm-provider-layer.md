---
id: 0007-roll-own-llm-provider-layer
kind: decision
order: 7
status: active
date: 2026-05-21
responsibility: |
  Hand-roll the LLM provider abstraction rather than adopting rig, swiftide, or genai.
---

# 0007 — Hand-roll the provider layer

## Status

Active.

## Context

Three notable Rust LLM frameworks exist (rig, swiftide, genai) and all are pre-1.0 with frequent breaking releases. They overlap heavily with what oxidant needs from the LLM layer (chat + streaming + tool use across multiple backends) but each carries opinions we don't want: chains, agent objects, RAG plumbing, or framework-specific tool abstractions that don't compose with our [[contracts/tool]] design.

## Decision

`oxidant-providers` implements the [[contracts/provider]] trait directly against each backend's native API. Surface is small:

- Chat with streaming
- Tool use (request shape + response parsing)
- Capabilities probe (caching, thinking, vision)

Hand-rolled SSE handling per backend (no `reqwest-eventsource` — last release 2024-03, stagnating).

This is part of the same instinct as [[decisions/0006-shell-out-to-git-cli]]: when a job is small and well-defined, own it rather than depend on an opinionated framework.

## Consequences

Positive: no framework churn; minimal compile cost; the trait shape is exactly what oxidant needs.

Negative: backend-specific quirks (tool-call payload differences, streaming framing variations) live in our codebase. Mitigation: those quirks are localised behind `Provider` impls; the public surface stays clean.

## Related

- [[decisions/0001-multi-provider-llm]]
- [[contracts/provider]]
