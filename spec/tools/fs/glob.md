---
id: glob
kind: tool
parent: components/tools/fs
order: 3
implements:
  - contracts/tool
depends_on:
  - components/tools/fs
code:
  - crates/oxidant-tools/src/fs.rs
tests:
  - crates/oxidant-tools/src/fs.rs::glob_finds_files
status: active
responsibility: |
  Return paths matching a glob pattern under the worktree root.
---

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["pattern"],
  "properties": {
    "pattern":   { "type": "string", "description": "e.g. crates/**/*.rs" },
    "limit":     { "type": "integer", "default": 200, "maximum": 5000 },
    "case_insensitive": { "type": "boolean", "default": false }
  }
}
```

## Result

```json
{ "paths": ["crates/oxidant-core/src/lib.rs", "..."], "truncated": false, "count": 47 }
```

## Semantics

- Matching via `globset`.
- Honours `.gitignore` by default (configurable in future).
- Paths returned relative to workspace root, sorted lexically.
- For ranked or full-text search, prefer [[tools/search/text-search]].
