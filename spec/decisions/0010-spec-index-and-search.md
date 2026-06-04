```yaml
id: 0010-spec-index-and-search
kind: decision
order: 10
status: active
date: 2026-05-21
responsibility: |
  Adopt SQLite for spec metadata, Tantivy for full-text search over spec+code, and git for timeline; do not store timestamps in spec frontmatter.
```

# 0010 — Spec metadata index, full-text search, and git-backed timeline

## Status

Active. Set at project inception following [[decisions/0008-spec-is-canonical]].

## Context

The spec tree will grow to dozens, then hundreds, then thousands of small files. Hand-walking the tree for every navigation, dependency lookup, or "what changed recently" query won't scale. We need:

1. A **metadata index** for graph queries (`what depends on X`, `which components implement contract Y`, `find specs with status: draft`).
2. **Full-text search** that spans both `spec/**/*.md` and `crates/**/*.rs` (so the agent and the user can locate concepts that may be expressed in either).
3. A **timeline** of changes — when was this spec last edited, who edited what last week, which specs are co-changing with which code.

Naive approach considered and rejected: store `created` and `updated` ISO timestamps in spec frontmatter. This creates a self-modifying-history loop — every edit changes the frontmatter, changing the file's own commit footprint and making git history harder to reason about. It also duplicates information git already holds authoritatively.

## Decision

**SQLite for the metadata index.** Single file at `.oxidant/spec-index.db`. One row per spec file with columns: `id` (PK), `kind`, `parent`, `path`, `line_count`, `status`, `last_modified_iso` (derived from git), `last_modified_commit`. Edge tables for `implements`, `depends_on`, `code_paths`, and `[[ref]]` mentions in body text. Rebuilt incrementally by a file watcher in the running app; offline rebuild via `oxidant spec rebuild-index`.

**Tantivy for full-text search.** Single index at `.oxidant/search-index/` with fields `source` (`spec | code`), `path`, `content`, `kind` (specs only), `lang` (code only). BM25 ranking. Updated incrementally on the same watcher. Powers the model-facing [[tools/search/text-search]] and a search box in the GUI.

**Git is the timeline.** No timestamps in spec frontmatter. `spec_timeline` and `code_timeline` tools shell out to `git log` (consistent with [[decisions/0006-shell-out-to-git-cli]]) and cache results in the SQLite index keyed by commit SHA. The GUI's "recently changed" badges read from SQLite; clicking through opens the diff via `git`.

**Visual ordering: `order:` frontmatter field.** Optional integer per spec controlling within-folder display order. Absent → sorts alphabetically after all explicit orders. This is independent of dependency-topological order, which is computed on demand by walking `depends_on` edges in the index.

## Consequences

Positive:
- Graph queries become subqueries, not tree walks. `spec_for_file` and `spec_resolve_links` become O(1)–O(log n).
- The agent can grep the whole codebase + spec tree with a single tool call.
- Timeline answers are authoritative because they're from git.
- No frontmatter churn from auto-updating timestamps.

Negative:
- Two more crate dependencies in `oxidant-spec-tools` (`rusqlite` and `tantivy`). Both are mature; `tantivy` is heavier (~10MB) but pulls in much less than embedding a search engine via FFI.
- The index can go stale if the file watcher misses events (rare, but offline rebuild is the recovery).
- Tantivy is a fast-moving Rust crate; pin a version and document the upgrade plan.

## Related

- [[components/spec-tools/index-db]] — the SQLite index design
- [[components/spec-tools/search-index]] — the Tantivy index design
- [[components/spec-tools/timeline]] — git-backed timeline component
- [[tools/search/text-search]], [[tools/search/spec-search]] — model-facing search tools
- [[tools/timeline/spec-changes]], [[tools/timeline/code-changes]] — model-facing timeline tools
