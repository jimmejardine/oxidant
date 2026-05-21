---
id: workspace-edit
kind: contract
parent: overview
order: 3
status: active
depends_on: []
code:
  - crates/oxidant-tools/src/workspace_edit.rs
responsibility: |
  The atomic multi-file edit data structure used by every code-changing path in oxidant.
---

`WorkspaceEdit` is the lingua franca for code changes. LSP refactors produce it; clippy-fix flows produce it; `syn` transforms produce it; the two model-facing edit tools both build it. The [[components/tools/workspace-edit-substrate]] consumes it.

## Types

```rust
pub struct WorkspaceEdit {
    pub changes: HashMap<PathBuf, Vec<TextEdit>>,
}

pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
    pub expected_text: Option<String>,
}

pub struct Range {
    pub start: Position,
    pub end: Position,
}

pub struct Position {
    pub line: u32,      // 0-indexed
    pub character: u32, // 0-indexed UTF-16 code units (LSP convention)
}
```

## Semantic contract

| Property | Required behaviour |
|---|---|
| Atomicity | The whole WorkspaceEdit applies or none of it. See [[invariants/edits-are-atomic]]. |
| Range coordinates | LSP-style: 0-indexed line, 0-indexed UTF-16 character. Substrate converts to bytes once at apply time. |
| Overlap | Edits within one file must not overlap. Producers ensure this; the substrate rejects violations. |
| Order | Producers may emit edits in any order. The substrate sorts by descending start within each file before applying. |
| `expected_text` | Optional optimistic-concurrency check. If supplied, the current bytes at `range` must match; mismatch aborts the WorkspaceEdit. |
| Syntactic validity | After applying, every `.rs` file touched must parse with `syn`. The substrate validates and rolls back on failure. See [[invariants/rust-files-parse-after-edit]]. |

## Construction patterns

- **LSP-driven**: `WorkspaceEdit` arrives directly from rust-analyzer (`rename`, `code_actions`). Type conversion only.
- **Cargo-driven**: A diagnostic with `suggestion.replacement` becomes a single-edit `WorkspaceEdit` at the diagnostic span.
- **Syn-driven**: A transform on a parsed file produces a `WorkspaceEdit` from the byte ranges of the modified nodes.
- **Model-driven (string)**: [[tools/edit/edit-string]] locates a unique substring and builds a single-edit `WorkspaceEdit` with `expected_text` set to the matched string.
- **Model-driven (span)**: [[tools/edit/apply-edits]] is a thin shell around manual construction.

## Implementors / consumers

- Producer: [[tools/edit/edit-string]], [[tools/edit/apply-edits]], [[tools/lsp/rust-rename]], [[tools/lsp/rust-code-actions]], [[tools/syn/syn-add-use]], [[tools/syn/syn-add-derive]], [[tools/syn/syn-rename-local]]
- Consumer: [[components/tools/workspace-edit-substrate]]
