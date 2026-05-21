---
id: validate
kind: component
parent: overview
order: 3
implements: []
depends_on:
  - components/spec-tools/frontmatter
  - components/spec-tools/graph
  - components/spec-tools/index-db
code:
  - crates/oxidant-spec-tools/src/validate.rs
status: active
responsibility: |
  Produce structured warnings about frontmatter completeness, link integrity, length budgets, orphans, and code-path existence.
---

The drift detector for spec hygiene. Warnings, never errors — even severe issues surface in the GUI's spec panel rather than blocking the agent. See [[decisions/0008-spec-is-canonical]].

## Checks

| Check | What it catches |
|---|---|
| frontmatter_missing_required | A field required for the spec's kind is absent. |
| frontmatter_invalid_value | `status` not in {draft, active, deprecated}, `kind` unknown, etc. |
| duplicate_id | Two specs share the same canonical `id`. |
| unresolved_ref | A `[[ref]]` (frontmatter or body) doesn't resolve to an existing spec. |
| short_form_ambiguous | A short-form `[[name]]` matches multiple canonical refs. |
| orphan | A non-`overview` spec has zero inbound edges. |
| cycle | The graph contains a cycle in `parent` or `depends_on`. |
| length_budget_exceeded | Spec body exceeds the per-kind budget. |
| missing_code_path | A `code:` entry refers to a file that doesn't exist. |
| reachability | `overview` cannot reach this spec via the link graph in ≤ 4 hops. |

## Output

```rust
pub struct Warning {
    pub spec_id: Option<String>,    // None for tree-wide issues
    pub kind: WarningKind,
    pub message: String,
    pub location: Option<(PathBuf, usize, usize)>,
}

pub fn validate(repo: &Path) -> Vec<Warning>;
```

The GUI buckets warnings by `kind` in the spec panel; the agent receives them via [[tools/spec/spec-validate]].

## Performance

A full validate scan over hundreds of specs completes in <100ms via the in-memory graph built from the SQLite index. Incremental validation on a single file: <10ms.
