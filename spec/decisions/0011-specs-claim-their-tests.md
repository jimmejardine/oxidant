---
id: 0011-specs-claim-their-tests
kind: decision
order: 11
status: active
date: 2026-05-22
responsibility: |
  Establish that spec frontmatter is the source of truth for which tests cover which specs; the validator surfaces orphan tests and unresolved test references as warnings.
---

# 0011 — Specs claim their tests

## Status

Active. Set following [[decisions/0008-spec-is-canonical]].

## Context

Specs already declare the code that realises them via the `code:` frontmatter field. Tests are part of that realisation but have so far been invisible to the spec layer: there is no mechanical way, given a spec, to enumerate the tests that exercise it; nor, given a test, to find the spec it serves. Two practical needs follow:

1. **Targeted re-runs.** When a spec is modified, an agent or human should be able to ask "which tests validate this spec?" and run exactly that subset, rather than the whole suite.
2. **Coverage hygiene.** A test that nothing claims is a candidate for deletion, rewrite, or — most often — a missing spec association. We want this surfaced, not hidden.

Two alternatives were considered and rejected:

- **Test-side annotations** (`// @covers tools/edit/apply-edits` in the test file, scanned by the validator). Keeps test authors honest at write time but inverts the source of truth: the spec no longer "owns" its tests, it merely receives them. Inconsistent with [[decisions/0008-spec-is-canonical]].
- **Derive coverage from `code:` paths.** Walk from each spec's `code:` files to tests that import them. Fragile (Rust integration tests rarely `use` the production module directly; many tests live in `tests/` and exercise via public surface) and produces noisy false positives.

## Decision

**Spec frontmatter owns test associations.** A new optional `tests:` field on every spec lists the tests that cover it. Two accepted forms:

```yaml
tests:
  - crates/oxidant-spec-tools/tests/validate_real_tree.rs::orphan_detection
  - crates/oxidant-spec-tools/tests/validate_real_tree.rs   # all #[test] in file
```

The `path::fn_name` form names a single test function; a bare path is shorthand for "every `#[test]` in this file is claimed by me". Identifiers are the validator's normalised form — `<repo-relative-path>::<fn>` — not cargo's module-path form, because the validator computes the inventory by scanning files for `#[test]` attributes, not by invoking cargo.

**Many-to-many.** One test may appear in multiple specs' `tests:` lists. The validator dedupes when computing the orphan set. Use this for tests that genuinely exercise more than one spec (an integration test crossing two components claims both).

**Any spec kind may carry `tests:`.** Tools, components, contracts, invariants, and decisions are all eligible. An invariant spec citing the test that enforces it is a high-value association.

**Orphans are warnings, never errors.** [[components/spec-tools/validate]] gains two new check kinds — `orphan_test` (a `#[test]` exists in code with no spec claiming it) and `unresolved_test` (a `tests:` entry refers to a test that doesn't exist). Both surface in the GUI spec panel and via [[tools/spec/spec-validate]]; neither blocks the agent. Consistent with the validator's existing posture: warn, don't gate.

**No reverse warning yet.** A symmetric `untested_spec` check is deliberately deferred. Contracts, invariants, and decisions are typically tested transitively through their implementers; flagging them as untested would swamp the signal. Revisit if the tooling around it improves.

## Consequences

Positive:
- A spec edit gives the agent an immediate, mechanically-derived list of tests to re-run.
- Orphan tests get a name and a place in the validator output, so they stop accumulating silently.
- The `tests:` list becomes a small but real cross-check that the spec actually has teeth.

Negative:
- A spec author who adds a test must remember to register it. Mitigation: `orphan_test` warnings surface the omission within the next validator run; this is the spec-first project's preferred failure mode (visible drift over silent drift).
- The `path::fn_name` identifier is not cargo's canonical form for inline `#[test]` functions (cargo uses module paths). Accepted: the validator's job is to map between the two, not to push cargo's grammar onto spec authors.

## Related

- [[components/spec-tools/frontmatter]] — where the `tests:` field is parsed
- [[components/spec-tools/validate]] — where `orphan_test` and `unresolved_test` are produced
- [[tools/spec/spec-validate]] — the model-facing surface
- [[decisions/0008-spec-is-canonical]] — the principle this extends
