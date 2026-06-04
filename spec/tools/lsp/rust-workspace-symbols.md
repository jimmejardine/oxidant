```yaml
id: rust-workspace-symbols
kind: tool
parent: components/rust-tools/lsp
order: 6
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/lsp
code:
  - crates/oxidant-rust-tools/src/lsp_client.rs
tests:
  - crates/oxidant-rust-tools/tests/lsp_live.rs::workspace_symbols_finds_add
status: active
responsibility: |
  Search across the workspace for symbols (functions, types, traits, modules) by name.
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["query"],
  "properties": {
    "query": { "type": "string", "minLength": 1 },
    "kind":  { "type": "string", "description": "fn | struct | enum | trait | impl | mod | const | static" },
    "limit": { "type": "integer", "default": 50, "maximum": 500 }
  }
}
```

## Result

```json
{
  "symbols": [
    { "name": "ToolRegistry", "kind": "struct",
      "file": "src/registry.rs",
      "range": { "start": {...}, "end": {...} } }
  ]
}
```

## When to use vs text-search

- `text_search` — concept lookups, fuzzy matches across comments and docs
- `rust_workspace_symbols` — exact symbol lookup, scoped to actual definitions, fastest path to "where is `Foo` defined"
- `grep` — regex matching across raw source text
