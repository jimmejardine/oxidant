---
id: 0009-no-ra-ap-crates-lsp-suffices
kind: decision
order: 9
status: active
date: 2026-05-21
responsibility: |
  Use rust-analyzer as an LSP subprocess; do not embed ra_ap_* internal crates for semantic queries.
---

# 0009 — No `ra_ap_*` internal crates; LSP is enough

## Status

Active.

## Context

rust-analyzer publishes its internal crates (`ra_ap_syntax`, `ra_ap_hir`, `ra_ap_ide`) as `0.0.x` releases on crates.io. They expose richer semantic info than LSP — direct access to the HIR, type inference, name resolution — but they have:

- No API stability guarantee. Weekly breaking releases.
- A heavy compile cost. Adds significantly to oxidant's build.
- Operational complexity: you're running rust-analyzer's analysis engine in-process; you own its memory, its panics, its semantics.

LSP already exposes everything oxidant needs: hover (type + docs), goto-definition, find-references, rename, code actions, workspace symbols, diagnostics. rust-analyzer serves all of these as an LSP subprocess.

## Decision

oxidant runs `rust-analyzer` as a subprocess per active exploration and talks to it over LSP via `async-lsp` + `lsp-types`. We do **not** depend on `ra_ap_*` crates.

For syntactic queries (and write-capable AST transforms), we use `syn` 2.x — same parser rust-analyzer uses under the hood, but standalone and stable.

## Consequences

Positive: stable dep tree; compile cost remains tractable; rust-analyzer's analyzer is treated as a service, not a library; we ride RA's own improvements via process upgrades.

Negative: a small set of queries that aren't surfaced over LSP (e.g., raw HIR introspection, call-graph queries beyond `find_references`) are out of reach. None of them are MVP requirements. Reassess only if a concrete need emerges.

## Related

- [[components/rust-tools/lsp]] — LSP client
- [[components/rust-tools/syn-tools]] — syntactic queries and transforms
- [[decisions/0006-shell-out-to-git-cli]] — same "use the mature external tool" pattern
