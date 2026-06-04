```yaml
id: workspace-edit-substrate
kind: component
parent: overview
order: 1
implements:
  - contracts/workspace-edit
depends_on: []
code:
  - crates/oxidant-tools/src/workspace_edit.rs
tests:
  - crates/oxidant-tools/src/workspace_edit.rs
  - crates/oxidant-tools/src/edit.rs::byte_offset_to_position_works_across_lines
  - crates/oxidant-rust-tools/tests/lsp_live.rs::rename_apply_routes_through_substrate
status: active
responsibility: |
  Apply atomic, span-precise multi-file edits with optimistic-concurrency checks and post-edit syntactic validation for .rs files.
```

The internal apply path for every code change in oxidant. Private to `oxidant-tools`; not exposed directly to the model. Both [[tools/edit/edit-string]] and [[tools/edit/apply-edits]] route through it, and so do smart tools like [[tools/lsp/rust-rename]], [[tools/syn/syn-add-use]], and clippy-fix flows.

## Inputs

A `WorkspaceEdit` per [[contracts/workspace-edit]]: a map of file path → list of `TextEdit { range, new_text, expected_text? }`.

## Behaviour

1. **Range normalisation.** Convert all ranges to byte offsets per file (LSP uses UTF-16 code units, rustc/syn use bytes). Done once per file at the boundary.
2. **Order edits.** Within each file, sort by descending start offset so applying one never invalidates the next's offsets.
3. **Conflict check.** Reject overlapping ranges within a single file.
4. **Optimistic-concurrency check.** If `expected_text` is supplied on any edit, verify the current bytes at that range match. Mismatch → abort the entire WorkspaceEdit and report which edit failed and what was actually there.
5. **In-memory application.** Apply all edits to in-memory file contents.
6. **Syntactic validation.** For every `.rs` file touched, run `syn::parse_file` on the post-edit content. Any parse failure → roll back everything; surface the syn error with file/line context. See [[invariants/rust-files-parse-after-edit]].
7. **Atomic write.** For each file, write to a temp file in the same directory and `rename` over the original. All-or-nothing across the WorkspaceEdit; if any rename fails, the in-flight ones are reverted.
8. **Result.** Return a summary including post-edit byte ranges of each replacement, useful for chained edits in the same agent turn.

## What lives here vs elsewhere

- Range types, conversion helpers, and the `TextEdit` struct → [[contracts/workspace-edit]]
- String-replace UX → [[tools/edit/edit-string]]
- Span-precise UX → [[tools/edit/apply-edits]]
- LSP/syn refactors that produce WorkspaceEdits → those tool specs, not here

## Non-goals

- Does not pretty-print or reformat. Edits are byte-for-byte; the caller chooses `new_text` exactly.
- Does not run `cargo fmt`. Formatting is a separate explicit step the agent invokes when desired.
- Does not validate non-`.rs` files beyond write success.
