```yaml
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
tests:
  - crates/oxidant-spec-tools/tests/validate_real_tree.rs
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_validate_tree_wide_returns_warnings
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_validate_kind_filter_works
  - crates/oxidant-spec-tools/tests/spec_tools_real_tree.rs::spec_validate_unknown_kind_filter_yields_empty
status: active
responsibility: |
  Produce structured warnings about frontmatter completeness, link integrity, length budgets, orphans, and code-path existence.
---
```

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
| cycle | The graph contains a cycle in `parent` or `depends_on`. The message lists the participating specs as a path (e.g. `components/a → components/b → components/c → components/a`) so the agent can immediately fix the offending edge without grepping. One warning is emitted per disjoint cycle. |
| length_budget_exceeded | Spec body exceeds the per-kind budget. |
| missing_code_path | A `code:` entry refers to a file that doesn't exist. |
| orphan_test | A `#[test]` exists in code but no spec's `tests:` claims it (directly or via whole-file shorthand). See [[decisions/0011-specs-claim-their-tests]]. |
| unresolved_test | A `tests:` entry refers to a path or function that doesn't exist. |
| reachability | `overview` cannot reach this spec via the link graph at all (orphan). Depth is unbounded — deep abstraction layers are fine; the only problem is total disconnection. |

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

## Test inventory

To produce `orphan_test` and `unresolved_test`, the validator builds the universe of test ids by walking `crates/**/*.rs` and emitting one `<repo-relative-path>::<fn_name>` per `#[test]` attribute it finds. Both integration tests (`crates/x/tests/y.rs`) and inline `#[cfg(test)] mod` tests in `src/` are included. The id form is the validator's normalised shape, not cargo's module-path form; the spec author writes the path they see on disk.

A spec's `tests:` entries are expanded: a bare path claims every test in that file, a `path::fn` entry claims exactly that one. The union across all specs is the *claimed* set; the inventory minus claimed is the orphan set; claimed minus inventory is the unresolved set.

## Performance

A full validate scan over hundreds of specs completes in <100ms via the in-memory graph built from the SQLite index. Incremental validation on a single file: <10ms.
