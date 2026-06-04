```yaml
---
id: spec-validate
kind: tool
parent: components/spec-tools/validate
order: 5
implements:
  - contracts/tool
depends_on:
  - components/spec-tools/validate
code:
  - crates/oxidant-spec-tools/src/tools/spec_validate.rs
tests:
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_validate_tree_wide_returns_warnings
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_validate_kind_filter_works
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_validate_unknown_kind_filter_yields_empty
status: active
responsibility: |
  Run the full validator over spec/ and return structured warnings (frontmatter completeness, link integrity, length budgets, orphans, code path existence).
---
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "ref":      { "type": "string", "description": "limit to this spec; omit for tree-wide" },
    "kinds":    { "type": "array", "items": { "type": "string" }, "description": "filter to these warning kinds" }
  }
}
```

## Result

```json
{
  "warnings": [
    { "spec_id": "tools/edit/apply-edits",
      "kind":    "unresolved_ref",
      "message": "[[components/tools/edit]] does not resolve",
      "location": ["spec/tools/edit/apply-edits.md", 24, 18] }
  ],
  "counts": { "unresolved_ref": 3, "orphan": 1, "length_budget_exceeded": 0 }
}
```

Warnings never fail the tool — even severe issues return `ok: true` with the warnings list. Surface drives the GUI badge in [[components/gui/spec-tree-panel]].

## See also

- [[components/spec-tools/validate]] — implementation, full list of warning kinds
- [[tools/spec/spec-diff]] — for spec↔code drift, not internal spec hygiene
