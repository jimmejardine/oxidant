```yaml
---
id: apply-edits
kind: tool
parent: components/tools/edit
order: 2
implements:
  - contracts/tool
depends_on:
  - components/tools/workspace-edit-substrate
  - contracts/workspace-edit
code:
  - crates/oxidant-tools/src/edit.rs
tests:
  - crates/oxidant-tools/src/edit.rs::apply_edits_span_precise
status: active
responsibility: |
  Apply one or more span-precise edits across one or more files, atomically.
---
```

The span-native edit surface. Used when the model already has spans from a previous tool call (cargo diagnostic, LSP reference, syn query) and wants to make a precise change without copying source text.

`category`: `Mutating`.

## Why this tool exists

Every smart Rust tool in oxidant returns spans. If the only edit surface were [[tools/edit/edit-string]], the model would have to `fs_read` the file, copy the exact source text at the span, and submit it as `old_string`. That's a wasted roundtrip and a likely failure mode (whitespace mismatch). `apply_edits` lets the model pipe spans straight from the tool that produced them into a mutation.

## Schema

```json
{
  "type": "object",
  "required": ["edits"],
  "properties": {
    "edits": {
      "type": "array", "minItems": 1,
      "items": {
        "type": "object",
        "required": ["file", "range", "new_text"],
        "properties": {
          "file":          { "type": "string" },
          "range":         { "$ref": "#/$defs/Range" },
          "new_text":      { "type": "string" },
          "expected_text": { "type": "string", "description": "optimistic-concurrency check" }
        }
      }
    }
  }
}
```

Range follows LSP convention (0-indexed line, 0-indexed UTF-16 character) so spans from `rust_hover`, `rust_find_references`, `cargo_check` etc. paste in directly. See [[contracts/workspace-edit]] for the canonical Range type.

## Semantics

1. Build a `WorkspaceEdit` from the edits, grouping by file.
2. Hand off to [[components/tools/workspace-edit-substrate]] — all atomicity, expected-text checks, syn-parse validation, and rollback live there.
3. On success, return per-edit post-application byte ranges and per-file one-line summary.
4. On failure, return the substrate's structured error (overlap / expected-text mismatch / syn parse failure with file:line).

## Example invocation

```json
{
  "edits": [
    {
      "file": "crates/oxidant-tools/src/edit.rs",
      "range": { "start": {"line": 42, "character": 4}, "end": {"line": 42, "character": 14} },
      "new_text": "apply_edits",
      "expected_text": "applyEdits"
    }
  ]
}
```

## See also

- [[tools/edit/edit-string]] — string-replace surface for when you don't have a span yet
- [[invariants/edits-are-atomic]]
- [[flows/fix-diagnostic]] — primary use case
