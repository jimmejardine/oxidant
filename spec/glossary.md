---
id: glossary
kind: glossary
order: 1
status: active
responsibility: |
  Shared vocabulary referenced across the rest of the spec tree.
---

# Glossary

Shared vocabulary used throughout the spec.

| Term | Meaning |
|---|---|
| **agent loop** | The `send → stream → parse tool calls → dispatch → append results → repeat` cycle in [[components/core/agent-loop]]. |
| **conversation** | An ordered sequence of messages between user, assistant, and tool results. One per exploration. See [[components/core/conversation]]. |
| **exploration** | A self-contained workspace: one conversation, one git branch, one worktree, one rust-analyzer process, one `target/`. See [[components/core/exploration]]. The main exploration is the original checkout; sub-explorations live in `.oxidant-worktrees/`. |
| **provider** | An LLM backend (Anthropic, OpenAI, Ollama, llama.cpp). See [[contracts/provider]]. |
| **tool** | A capability exposed to the model, with a JSON schema and an invoke method. See [[contracts/tool]]. |
| **tool context** | The scope a tool runs in — workspace root path, permission state, exploration id. Tools are pure modulo this context. |
| **WorkspaceEdit** | An atomic, multi-file edit payload. See [[contracts/workspace-edit]]. Internal substrate for every code change. |
| **span** | A `(file, range)` pair. Spans flow between LSP, cargo diagnostics, syn queries, and the edit substrate without round-tripping through text. |
| **spec** | A single markdown file under `spec/` with frontmatter declaring its `kind` and graph edges. |
| **kind** | The discipline a spec file falls under: `overview`, `glossary`, `component`, `contract`, `tool`, `flow`, `invariant`, `decision`. Determines required frontmatter, length budget, and folder. |
| **drift** | Divergence between a spec and the code it describes. Detected by [[components/spec-tools/diff]]. |
| **dock panel** | A region of an exploration's window (LEFT, RIGHT, BOTTOM, CENTRE) managed by `egui_dock`. See [[components/gui/dock-layout]]. |
| **sub-exploration** | An exploration spawned from another exploration (recursive). Always gets its own OS window. |
| **sccache** | External binary required at runtime; provides cross-worktree rustc cache. See [[decisions/0005-no-shared-target-dir-use-sccache]]. |
