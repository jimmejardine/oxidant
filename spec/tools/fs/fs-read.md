```yaml
---
id: fs-read
kind: tool
parent: components/tools/fs
order: 1
implements:
  - contracts/tool
depends_on:
  - components/tools/fs
code:
  - crates/oxidant-tools/src/fs.rs
tests:
  - crates/oxidant-tools/src/fs.rs::fs_read_whole_file
  - crates/oxidant-tools/src/fs.rs::fs_read_with_offset_and_limit
  - crates/oxidant-tools/src/fs.rs::fs_read_binary_returns_marker
  - crates/oxidant-tools/src/fs.rs::fs_read_rejects_escape
status: active
responsibility: |
  Read a file from the worktree, optionally a line range, returning UTF-8 text or a binary marker.
---
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["file"],
  "properties": {
    "file":   { "type": "string", "description": "path relative to workspace root" },
    "offset": { "type": "integer", "minimum": 0, "description": "1-indexed start line" },
    "limit":  { "type": "integer", "minimum": 1, "description": "number of lines to read" }
  }
}
```

## Result

```json
{ "content": "...", "lines": 215, "binary": false }
```

For binary files: `{ "binary": true, "size": 1048576 }` (no `content`).

## Semantics

- Path resolved against `ctx.workspace_root`; canonicalised; rejected if it escapes.
- UTF-8 strict — invalid bytes → binary marker.
- Without `offset/limit`, reads whole file (subject to a default 1MB cap; exceeding → error suggesting `offset/limit`).

## See also

- [[tools/fs/glob]] for filename matching
- [[tools/fs/grep]] for content matching
