---
id: edit
kind: component
parent: overview
order: 3
implements: []
depends_on:
  - components/tools/workspace-edit-substrate
  - contracts/workspace-edit
code:
  - crates/oxidant-tools/src/edit.rs
tests:
  - crates/oxidant-tools/src/edit.rs
status: active
responsibility: |
  Expose the two model-facing edit surfaces (string-replace and span-precise), both backed by the workspace-edit substrate.
---

The edit subsystem. Holds [[tools/edit/edit-string]] and [[tools/edit/apply-edits]], the only model-facing tools that mutate source code directly. Smart-tool refactors (rename, code actions, syn transforms) also produce `WorkspaceEdit`s and apply via the same substrate.

## Why two surfaces

See [[tools/edit/edit-string]] vs [[tools/edit/apply-edits]] for the per-surface rationale. Briefly: `edit_string` is the natural surface when the model has just read a file and wants to change `foo` to `bar`; `apply_edits` is the natural surface when a previous tool call (cargo diagnostic, LSP reference) already produced a span.

## What this component does not own

- The actual application logic — that's [[components/tools/workspace-edit-substrate]].
- LSP-driven refactors — those build WorkspaceEdits but live under [[components/rust-tools/lsp]].
- Syn-driven transforms — same pattern, under [[components/rust-tools/syn-tools]].

This component is the **thin model-facing layer**.
