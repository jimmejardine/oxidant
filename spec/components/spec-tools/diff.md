---
id: diff
kind: component
parent: overview
order: 4
implements: []
depends_on:
  - components/spec-tools/frontmatter
  - components/spec-tools/graph
code:
  - crates/oxidant-spec-tools/src/diff.rs
tests:
  - crates/oxidant-spec-tools/src/diff.rs
  - crates/oxidant-spec-tools/tests/diff_real_tree.rs
status: active
responsibility: |
  Detect spec↔code drift: trait-method drift for contract specs and code-path existence for component specs.
---

This component uses `syn` directly for the trait parse — [[components/rust-tools/syn-tools]] is the model-facing AST surface, separate concern.

The mechanism that makes [[decisions/0008-spec-is-canonical]] enforceable. Without `spec_diff`, specs become shelfware; with it, divergence is a flagged warning the agent is prompted to fix.

## MVP scope

Two checks:

### 1. Contract trait drift

For each spec with `kind: contract`:
- Extract every ```rust fenced block from the body, parse each with `syn::parse_file`, collect the `ItemTrait` entries.
- For each spec trait, locate the same-named trait in any of the `code:` paths (each parsed as a `syn::File`).
- Compare methods by name and signature. Signatures are normalised before comparison: the token stream of each `syn::Signature` is rendered via `quote!{}`, then every path prefix is stripped to its last segment (so `serde_json::Value` matches `Value`, `crate::foo::Bar` matches `Bar`). This catches genuine drift while tolerating `use` statements rewriting paths.
- Emit `MethodAdded` (in code, not in spec), `MethodRemoved` (in spec, not in code), or `MethodSignatureChanged` (in both, signatures differ after normalisation).

### 2. Component code-path existence

For each spec with `kind: component`:
- For every entry in `code:`, check the path exists relative to the worktree root.
- Emit `MissingCodePath` per missing entry.

## Output shape

```rust
pub enum Drift {
    MethodAdded { contract_id: String, method: String },
    MethodRemoved { contract_id: String, method: String },
    MethodSignatureChanged { contract_id: String, method: String, spec: String, code: String },
    MissingCodePath { spec_id: String, path: PathBuf },
}

pub fn diff_spec(spec_id: &str) -> Vec<Drift>;
pub fn diff_all() -> Vec<Drift>;
```

## Deferred to v2

- Responsibility-vs-impl semantic drift (compare prose `responsibility:` to inferred behaviour — needs the LLM in the loop).
- Flow-vs-call-graph drift (does the named sequence of tool calls match what the code actually does?).
- Invariant verification via miri/proptest.

These are all genuinely useful but each is a multi-week project. v1 ships the two cheap, mechanical checks that already create most of the value.

## See also

- [[tools/spec/spec-diff]] — the agent-facing tool
- [[decisions/0008-spec-is-canonical]]
