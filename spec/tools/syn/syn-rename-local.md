```yaml
id: syn-rename-local
kind: tool
parent: components/rust-tools/syn-tools
order: 4
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/syn-tools
  - contracts/workspace-edit
code:
  - crates/oxidant-rust-tools/src/syn_query.rs
tests:
  - crates/oxidant-rust-tools/src/syn_query.rs::rename_local_renames_parameter_inside_fn
  - crates/oxidant-rust-tools/src/syn_query.rs::rename_local_rejects_invalid_ident
status: active
responsibility: |
  Rename a local binding (variable, parameter, or local function) within a single file based on a syntactic span; cross-file renames go through rust-rename.
```

`category`: `ReadOnly` for preview; `Mutating` when applied.

## Schema

```json
{
  "type": "object",
  "required": ["file", "span", "new_name"],
  "properties": {
    "file":     { "type": "string" },
    "span":     { "type": "object", "description": "byte range of the binding's name token" },
    "new_name": { "type": "string", "minLength": 1 },
    "apply":    { "type": "boolean", "default": false }
  }
}
```

## Result

Same WorkspaceEdit shape as the other syn tools.

## Limitations

- Syntactic only. If the local name is shadowed by an outer binding of the same identifier, the rename may rename more than intended. Use [[tools/lsp/rust-rename]] for scope-aware cross-file renames.
- The span must point at the binding declaration site, not a reference. Tool returns an error if span points at a reference (and suggests using `rust_rename` instead).

## When this tool earns its keep

When you've just produced a fresh local with `syn` (e.g. extracted from a refactor) and need to rename it before any LSP sync — faster and self-contained.
