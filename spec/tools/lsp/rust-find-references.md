```yaml
id: rust-find-references
kind: tool
parent: components/rust-tools/lsp
order: 3
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/lsp
code:
  - crates/oxidant-rust-tools/src/lsp_client.rs
tests:
  - crates/oxidant-rust-tools/tests/lsp_live.rs::find_references_returns_call_and_definition
status: active
responsibility: |
  Return all references to the symbol at a position across the workspace.
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["file", "line", "character"],
  "properties": {
    "file":               { "type": "string" },
    "line":               { "type": "integer", "minimum": 0 },
    "character":          { "type": "integer", "minimum": 0 },
    "include_declaration":{ "type": "boolean", "default": true }
  }
}
```

## Result

```json
{
  "references": [
    { "file": "src/foo.rs", "range": {...}, "kind": "definition" },
    { "file": "src/bar.rs", "range": {...}, "kind": "read" },
    { "file": "src/baz.rs", "range": {...}, "kind": "write" }
  ]
}
```

`kind` is best-effort from rust-analyzer; many references are reported as `unspecified` and tools should not rely on it for correctness.

## Why this beats grep

Disambiguates by binding: a search for `bar` won't false-positive on `unrelated::bar` or `Other::bar`. Use this whenever the rust-analyzer-indexed answer is available.

## See also

- [[tools/lsp/rust-rename]] — cross-file rename driven by the same resolution
