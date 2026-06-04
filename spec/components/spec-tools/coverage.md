```yaml
id: coverage
kind: component
parent: overview
order: 8
implements: []
depends_on:
  - components/spec-tools/frontmatter
code:
  - crates/oxidant-spec-tools/src/coverage.rs
status: active
responsibility: |
  Compute spec coverage of code: which workspace Rust source files are transitively reachable from the files specs declare in their `code:` frontmatter, so code anchored to no spec can be found.
```

Specs name high-level files; those `use` utility files specs don't name directly. This component builds a **file-level import graph** and reports `crates/*/src/**/*.rs` files that nothing a spec declares transitively reaches.

## Method

1. **Crates**: each `crates/*/` with a `Cargo.toml` contributes a crate ident (dir name, `-`→`_`) and roots (`src/lib.rs`, `src/main.rs`).
2. **Module map**: follow `mod` declarations from each root (`mod foo;` → `foo.rs` / `foo/mod.rs`; inline `mod foo { … }` extends the path within the same file) → a `module-path → file` map.
3. **Edges** (`file → file`): from each file's `use` trees and `crate::` / `self::` / `super::` / `<workspace-crate>::`-rooted path expressions (collected via `syn`), resolved through the module map to the file the path lands in. External/std paths are skipped.
4. **Seed**: every existing `code:` file across all specs (gathered by walking `spec/**/*.md`, see [[components/spec-tools/frontmatter]]).
5. **Reachability**: BFS from the seeds over the edges.
6. **Report**: source files not reached = `uncovered`, grouped by crate; plus `missing_seeds` (declared `code:` files absent on disk).

## Limits

Reachability over real import edges — deterministic, but **heuristic**: macro-generated paths, `include!`, and edges that exist only through external re-exports can be missed. Binary entry points (`main.rs`, CLI-only modules) and pure re-export `mod.rs` hubs appear as uncovered unless a spec declares them — that's a genuine "not spec-anchored" signal, not a defect. This is a review aid, **not** a CI gate. Function-level granularity is a deferred follow-up (would need a call graph).
