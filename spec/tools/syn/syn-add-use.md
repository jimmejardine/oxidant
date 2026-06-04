```yaml
id: syn-add-use
kind: tool
parent: components/rust-tools/syn-tools
order: 2
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/syn-tools
  - contracts/workspace-edit
code:
  - crates/oxidant-rust-tools/src/syn_query.rs
tests:
  - crates/oxidant-rust-tools/src/syn_query.rs::add_use_inserts_after_existing_uses
  - crates/oxidant-rust-tools/src/syn_query.rs::add_use_skips_when_already_imported
status: active
responsibility: |
  Add a use path to a Rust file at the correct location (after existing use clauses, respecting grouping); produce a WorkspaceEdit.
```

`category`: `ReadOnly` for preview; `Mutating` when applied via the substrate.

## Schema

```json
{
  "type": "object",
  "required": ["file", "path"],
  "properties": {
    "file":  { "type": "string" },
    "path":  { "type": "string", "description": "e.g. crate::foo::Bar or serde::Serialize" },
    "apply": { "type": "boolean", "default": false }
  }
}
```

## Result

```json
{
  "workspace_edit": { "changes": { "src/foo.rs": [ ... ] } },
  "applied":        false,
  "skipped_reason": null
}
```

`skipped_reason` is set to `"already_imported"` (and no edit produced) when the path is already in scope.

## Semantics

- Insert after the last existing `use` if any, else after the module's `//!` doc comments and `#![...]` inner attributes.
- Sort within the existing block alphabetically? — not in v1; oxidant respects whatever convention the file uses (preserve grouping by `std`/external/`crate` if detectable).
- Always produces a `WorkspaceEdit`; the substrate's syn-parse check catches malformed paths.

## See also

- [[tools/lsp/rust-code-actions]] — `source.organizeImports` for cleanup after several adds
