```yaml
id: 0008-spec-is-canonical
kind: decision
order: 8
status: active
date: 2026-05-21
responsibility: |
  Establish spec/ as the authoritative design source for oxidant; code is a realisation of spec, not the other way round.
```

# 0008 — `spec/` is the source of truth for design

## Status

Active. Set at project inception; rescinding requires a new ADR.

## Context

Code-only projects develop drift between "what the system does" and "what the team thinks it does". Comments and READMEs lag. Architecture documents become shelfware. For oxidant — which has [[tools/spec/spec-diff]] and ambitions to dogfood spec-driven work — letting the spec be retrofit documentation would undermine the whole proposition.

Two alternatives were considered and rejected:

- **Spec generated from code.** Doc-gen tools (rustdoc, mdBook from source) produce a faithful record of what *is* but not what *should be*. Responsibility, invariants, and design intent disappear. `spec_diff` becomes pointless because there's no independent thing to diff against.
- **Spec as advisory commentary.** Keep specs but treat code as authoritative; specs are nice-to-have. This is the default failure mode of every architecture-docs effort. Specs go stale, nobody trusts them, they're deleted in three months.

## Decision

`spec/` is the source of truth for oxidant's design. The rule is:

- **Order is spec → code.** State intent in the relevant spec file(s) first; then implement against it.
- **No code change without a matching spec change** when the change crosses a contract, responsibility, or invariant. Bug fixes inside a function don't qualify; renaming a trait method does.
- **`spec_diff` runs after every successful agent edit.** Surfaces drift in the GUI's spec panel as a badge; the agent is prompted to resolve before the task is declared done.
- **Length budgets enforced by warnings.** Hitting the cap is the lever that produces decomposition; see [[components/spec-tools/validate]].
- **References across the spec graph are mechanical** (`[[ref]]` syntax), so links can't silently rot.

## Consequences

Positive:
- Drift is mechanically detectable, not socially.
- The agent has a hierarchical, navigable, machine-checkable design artifact independent of source code.
- Reviewers reading a PR see spec + code diff side-by-side and can verify they agree.

Negative:
- Friction on small changes that "shouldn't need a spec edit". Mitigation: bug fixes and internal refactors don't require spec edits — only changes that cross declared contracts/responsibilities do.
- A second artifact to maintain. Mitigation: the agent maintains it; that's the whole proposition.
- Length budgets occasionally produce decomposition busywork. Accepted as the cost of mechanically-enforceable modularity.

## Related

- [[components/spec-tools/diff]] — the drift detector that makes this enforceable
- [[components/spec-tools/validate]] — enforces frontmatter completeness and link integrity
- [[decisions/0007-roll-own-llm-provider-layer]] — same "thin, owned, principled" instinct
