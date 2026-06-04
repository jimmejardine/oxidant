```yaml
---
id: rust-goto-definition
kind: tool
parent: components/rust-tools/lsp
order: 2
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/lsp
code:
  - crates/oxidant-rust-tools/src/lsp_client.rs
status: active
responsibility: |
  Return the definition site(s) of the symbol at a position.
---
```

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
{
  "locations": [
    { "file": "src/registry.rs",
      "range": { "start": {"line": 23, "character": 4}, "end": {"line": 23, "character": 12} } }
  ]
}
```

Multiple locations possible (e.g. trait methods with multiple impls); empty → not resolvable.

## See also

- [[tools/lsp/rust-find-references]] — inverse direction
- [[tools/lsp/rust-workspace-symbols]] — find by name rather than by position
