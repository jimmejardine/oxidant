```yaml
id: spec-read
kind: tool
parent: components/spec-tools/frontmatter
order: 1
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/frontmatter
code:
  - crates/oxidant-spec-tools/src/tools/spec_read.rs
tests:
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_read_canonical_ref
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_read_short_ref
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_read_unknown_ref_errors
status: active
responsibility: |
  Fetch one spec file by canonical or short-form ref, returning parsed frontmatter and raw body.
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "required": ["ref"],
  "properties": {
    "ref": { "type": "string", "description": "canonical (tools/edit/apply-edits) or short (apply-edits)" }
  }
}
```

## Result

```json
{
  "id":            "apply-edits",
  "kind":          "tool",
  "path":          "spec/tools/edit/apply-edits.md",
  "frontmatter":   { ... },
  "body":          "...",
  "outbound_refs": ["components/tools/workspace-edit-substrate", "invariants/edits-are-atomic"]
}
```

## Resolution

Canonical refs resolve directly to the file path. Short refs query the SQLite index by `id`; ambiguous short refs return an error listing the candidates.
