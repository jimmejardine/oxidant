```yaml
id: fs-write
kind: tool
parent: components/tools/fs
order: 2
implements:
  - contracts/tool
depends_on:
  - components/tools/fs
code:
  - crates/oxidant-tools/src/fs.rs
tests:
  - crates/oxidant-tools/src/fs.rs::fs_write_creates_then_overwrites
  - crates/oxidant-tools/src/fs.rs::fs_write_rejects_invalid_rust
status: active
responsibility: |
  Create a new file or fully overwrite an existing one; for in-place edits use edit-string or apply-edits instead.
```

`category`: `Mutating`.

## Schema

```json
{
  "type": "object",
  "required": ["file", "content"],
  "properties": {
    "file":    { "type": "string", "description": "path relative to workspace root" },
    "content": { "type": "string" }
  }
}
```

## Result

```json
{ "file": "Cargo.toml", "bytes_written": 451, "created": false }
```

## Semantics

- Path resolved + canonicalised against `ctx.workspace_root`.
- Atomic write: temp file + rename.
- `.rs` files get the [[invariants/rust-files-parse-after-edit]] check post-write — fails → file isn't written.
- For modifying existing source code, prefer [[tools/edit/edit-string]] or [[tools/edit/apply-edits]]: those produce diff-friendly history and pass the optimistic-concurrency check. `fs_write` is the "I'm creating a brand-new file" tool.

## When to use vs edit tools

| Situation | Tool |
|---|---|
| File does not exist yet | `fs_write` |
| Full rewrite of an existing file | `fs_write` |
| Localised change | [[tools/edit/edit-string]] or [[tools/edit/apply-edits]] |
