```yaml
---
id: syn-tools
kind: component
parent: overview
order: 3
implements: []
depends_on:
  - contracts/workspace-edit
code:
  - crates/oxidant-rust-tools/src/syn_query.rs
tests:
  - crates/oxidant-rust-tools/src/syn_query.rs
status: active
responsibility: |
  Parse and transform Rust source via syn 2.x; backs the syntactic agent tools and produces WorkspaceEdits for the substrate.
---
```

The fast, syntactic, write-capable tier. Complements the semantic LSP layer ([[components/rust-tools/lsp]]) — see [[decisions/0009-no-ra-ap-crates-lsp-suffices]].

## What it does

- Parse a `.rs` file into a `syn::File`.
- Query: find items by kind + name pattern. See [[tools/syn/syn-find-items]].
- Transform: add `use` paths, add derives, perform scoped local renames. Each transform produces a `WorkspaceEdit` per [[contracts/workspace-edit]] — no direct filesystem writes.
- Round-trip via `prettyplease` for clean printing of generated nodes when needed; otherwise edits are byte-precise on the original source.

## Tools backed

- [[tools/syn/syn-find-items]]
- [[tools/syn/syn-add-use]]
- [[tools/syn/syn-add-derive]]
- [[tools/syn/syn-rename-local]]

## Coordinate system

`syn` operates on byte offsets. The component converts to/from LSP-style line/UTF-16-character ranges at the WorkspaceEdit boundary, identical to [[components/tools/workspace-edit-substrate]]'s convention.

## What lives elsewhere

- Cross-file rename (needs semantic resolution): [[tools/lsp/rust-rename]].
- Type-of / inferred-type queries: [[tools/lsp/rust-hover]].
- Find-references: [[tools/lsp/rust-find-references]].
