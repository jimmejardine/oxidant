```yaml
---
id: edit-string
kind: tool
parent: components/tools/edit
order: 1
implements:
  - contracts/tool
depends_on:
  - components/tools/workspace-edit-substrate
  - contracts/workspace-edit
code:
  - crates/oxidant-tools/src/edit.rs
tests:
  - crates/oxidant-tools/src/edit.rs::edit_string_unique_replacement
  - crates/oxidant-tools/src/edit.rs::edit_string_rejects_ambiguous_match
  - crates/oxidant-tools/src/edit.rs::edit_string_replace_all
  - crates/oxidant-tools/src/edit.rs::edit_string_missing_returns_clear_error
status: active
responsibility: |
  Replace a unique occurrence of `old_string` with `new_string` in a single file; the natural surface when no upstream tool produced a span.
---
```

The string-replace edit surface. Use after `fs_read`-ing a file when the model wants to change `foo` to `bar` in one spot. For span-precise edits (e.g. driven by `cargo_check` or `rust_hover`), prefer [[tools/edit/apply-edits]].

`category`: `Mutating`.

## Schema

```json
{
  "type": "object",
  "required": ["file", "old_string", "new_string"],
  "properties": {
    "file":        { "type": "string" },
    "old_string":  { "type": "string", "minLength": 1 },
    "new_string":  { "type": "string" },
    "replace_all": { "type": "boolean", "default": false }
  }
}
```

## Semantics

1. Read the file.
2. Locate occurrences of `old_string`.
3. If `replace_all = false` and there isn't exactly one occurrence → error with the count.
4. Build a `TextEdit` per occurrence with `expected_text = old_string` (the optimistic-concurrency check guarantees the file didn't change between locate and apply).
5. Route through [[components/tools/workspace-edit-substrate]] — gets atomicity, syn-parse validation for `.rs`, rollback.

## Result

```json
{
  "file": "crates/oxidant-tools/src/edit.rs",
  "replacements": 1,
  "post_edit_ranges": [
    { "start": {"line": 42, "character": 4}, "end": {"line": 42, "character": 14} }
  ]
}
```

## Failure modes

- `old_string` not found.
- `old_string` matches multiple times and `replace_all = false`.
- Post-edit syn parse fails (`.rs` only) → substrate rolls back; tool returns the syn error.

## See also

- [[tools/edit/apply-edits]] — for span-driven edits
