---
id: syn-add-derive
kind: tool
parent: components/rust-tools/syn-tools
order: 3
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/syn-tools
  - contracts/workspace-edit
code:
  - crates/oxidant-rust-tools/src/syn_query.rs
status: active
responsibility: |
  Add a #[derive(...)] entry to a struct or enum, merging with any existing derive attribute.
---

`category`: `ReadOnly` for preview; `Mutating` when applied.

## Schema

```json
{
  "type": "object",
  "required": ["file", "type_name", "derive"],
  "properties": {
    "file":      { "type": "string" },
    "type_name": { "type": "string", "description": "name of the struct or enum" },
    "derive":    { "oneOf": [
        { "type": "string" },
        { "type": "array", "items": { "type": "string" } }
    ] },
    "apply":     { "type": "boolean", "default": false }
  }
}
```

## Result

```json
{
  "workspace_edit": { "changes": { "src/foo.rs": [ ... ] } },
  "applied":        false,
  "merged_into_existing": true
}
```

## Semantics

- Locate the target type by name in the file (error if absent or duplicated).
- If a `#[derive(...)]` attribute already exists on the type, merge the new names into it (dedup, preserve original order, append new at end).
- Otherwise insert a fresh `#[derive(...)]` immediately above the type definition.
- The substrate's syn-parse check confirms validity.
