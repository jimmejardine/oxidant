---
id: rust-hover
kind: tool
parent: components/rust-tools/lsp
order: 1
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/lsp
code:
  - crates/oxidant-rust-tools/src/lsp_client.rs
status: active
responsibility: |
  Return the rust-analyzer hover info at a position: type signature plus markdown docs.
---

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["file", "line", "character"],
  "properties": {
    "file":      { "type": "string" },
    "line":      { "type": "integer", "minimum": 0 },
    "character": { "type": "integer", "minimum": 0 }
  }
}
```

## Result

```json
{ "type_signature": "fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult", "doc_md": "..." }
```

Empty hover → `{ "type_signature": null, "doc_md": null }`.

## Coordinate system

LSP-style (0-indexed line, 0-indexed UTF-16 character). The `character` arrives from cargo diagnostics, prior LSP responses, or syn-derived spans converted at the substrate boundary.
