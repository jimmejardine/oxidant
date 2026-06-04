```yaml
---
id: rust-rename
kind: tool
parent: components/rust-tools/lsp
order: 4
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/lsp
  - contracts/workspace-edit
code:
  - crates/oxidant-rust-tools/src/lsp_client.rs
tests:
  - crates/oxidant-rust-tools/tests/lsp_live.rs::rename_preview_returns_workspace_edit
  - crates/oxidant-rust-tools/tests/lsp_live.rs::rename_apply_routes_through_substrate
  - crates/oxidant-rust-tools/tests/lsp_live.rs::rename_rejects_invalid_identifier
status: active
responsibility: |
  Compute a cross-file rename WorkspaceEdit via rust-analyzer; the caller decides whether to apply.
---
```

`category`: `ReadOnly` for preview; the apply step is `Mutating` and goes through [[components/tools/workspace-edit-substrate]].

## Schema

```json
{
  "type": "object",
  "required": ["file", "line", "character", "new_name"],
  "properties": {
    "file":      { "type": "string" },
    "line":      { "type": "integer", "minimum": 0 },
    "character": { "type": "integer", "minimum": 0 },
    "new_name":  { "type": "string", "minLength": 1 },
    "apply":     { "type": "boolean", "default": false }
  }
}
```

## Result

```json
{
  "workspace_edit": { "changes": { "src/foo.rs": [ ... ] } },
  "applied":        false,
  "files_touched":  3,
  "edits_total":    17
}
```

If `apply: true`, the WorkspaceEdit is routed through the substrate; `applied: true` and the substrate's success/failure shape is included.

## Semantics

- rust-analyzer guarantees the rename respects scoping rules.
- `new_name` is validated as a syntactically-valid Rust identifier client-side (cheap guard against typos).
- For local-only renames where you already have the byte span, [[tools/syn/syn-rename-local]] is faster and doesn't need the LSP.
