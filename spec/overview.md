```yaml
id: overview
kind: overview
order: 0
status: active
responsibility: |
  Root orientation for the oxidant spec tree; entry points to every other spec by reader interest.
```

# oxidant

A Rust-native desktop code agent for working on Rust projects. Three differentiators distinguish it from general-purpose agents (Claude Code, Codex, Cursor):

1. **First-class Rust tooling** — rust-analyzer, cargo, syn, clippy, miri are exposed as structured tools, not text-scraped from shell output. Spans flow end-to-end. See [[components/rust-tools/lsp]], [[components/rust-tools/cargo-runner]], [[components/rust-tools/syn-tools]].

2. **Spec-driven design** — this very tree (`spec/`) is the source of truth for oxidant's design. Code is a realisation of spec, not the other way round. See [[decisions/0008-spec-is-canonical]] and [[components/spec-tools/validate]].

3. **Multi-exploration via git worktrees** — each side conversation is its own branch + worktree + rust-analyzer + `target/`, isolated from the main checkout. See [[components/vcs/worktree-mgmt]] and [[decisions/0004-git-worktree-per-exploration]].

## Reading this spec

- Files are organised by **kind** under `spec/<kind>/...`: `components/` describe what exists, `contracts/` describe interfaces, `tools/` describe model-facing agent tools, `flows/` describe multi-tool narratives, `invariants/` describe cross-cutting truths, `decisions/` are immutable ADRs.
- Cross-spec references use `[[path/under/spec]]` (no `.md`), e.g. `[[contracts/tool]]`.
- Spec-to-code references use plain relative markdown links plus the `code:` frontmatter field.
- See [[glossary]] for shared vocabulary.

## Entry points by interest

- "How does the agent edit code?" → [[components/tools/workspace-edit-substrate]], [[tools/edit/apply-edits]], [[invariants/edits-are-atomic]], [[flows/mutating-edit]]
- "How does one turn of the agent loop run end-to-end?" → [[flows/conversation-turn]], [[components/core/agent-loop]]
- "How are tool calls gated?" → [[flows/tool-permission-check]], [[contracts/tool]]
- "How does the GUI work?" → [[components/gui/dock-layout]], [[components/gui/viewport]], [[decisions/0003-egui-gui-over-tui]]
- "How are settings edited at runtime?" → [[flows/edit-settings]], [[components/config/settings]]
- "How does rust-analyzer come up the first time?" → [[flows/lsp-cold-start]], [[components/rust-tools/lsp]]
- "How do worktrees work?" → [[flows/spawn-exploration]], [[flows/merge-back]]
- "How do I extend oxidant with a new tool?" → [[flows/add-tool]]
- "How does spec-first editing actually go?" → [[flows/spec-first-edit]], [[decisions/0008-spec-is-canonical]]
- "What does the spec CI gate check?" → [[flows/spec-ci-gate]], [[components/spec-tools/validate]]
- "Why this and not that?" → all of `decisions/`

## Status

Pre-MVP. Phase 0 (this spec tree) is the first commit; Phase 1 (Cargo scaffold) follows.
