```yaml
---
id: syn-find-items
kind: tool
parent: components/rust-tools/syn-tools
order: 1
implements:
  - contracts/tool
depends_on:
  - components/rust-tools/syn-tools
code:
  - crates/oxidant-rust-tools/src/syn_query.rs
tests:
  - crates/oxidant-rust-tools/src/syn_query.rs::find_items_returns_fns_and_structs
  - crates/oxidant-rust-tools/src/syn_query.rs::find_items_name_pattern_substring
status: active
responsibility: |
  Parse a Rust file via syn and return the items matching a kind + optional name pattern, with byte ranges.
---
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["file", "kind"],
  "properties": {
    "file":         { "type": "string" },
    "kind":         { "type": "string", "enum": ["fn", "struct", "enum", "trait", "impl", "mod", "const", "static", "use", "type"] },
    "name_pattern": { "type": "string", "description": "optional substring or /regex/" }
  }
}
```

## Result

```json
{
  "items": [
    { "name": "ToolRegistry", "kind": "struct",
      "range": { "start": {"line": 23, "character": 0}, "end": {"line": 45, "character": 1} },
      "visibility": "pub" }
  ]
}
```

## Use vs LSP

- `syn_find_items` — purely syntactic, single file, fast, no semantic resolution
- [[tools/lsp/rust-workspace-symbols]] — semantic resolution, cross-file, requires LSP query

Use `syn_find_items` when you've already opened a file and want a structured view of its contents; use `workspace_symbols` when you don't know which file to look in.
