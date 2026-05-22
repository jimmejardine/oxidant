---
id: index-db
kind: component
parent: overview
order: 1
implements: []
depends_on:
  - components/spec-tools/frontmatter
code:
  - crates/oxidant-spec-tools/src/index_db.rs
tests:
  - crates/oxidant-spec-tools/src/index_db.rs
status: active
responsibility: |
  Maintain a SQLite metadata index of spec files supporting graph queries, status filters, and git-derived timestamps.
---

The structural backbone for [[tools/spec/spec-tree]], [[tools/spec/spec-resolve-links]], [[tools/spec/spec-for-file]], and the GUI's spec-tree panel ([[components/gui/spec-tree-panel]]). A single SQLite file at `<worktree>/.oxidant/spec-index.db`. Rebuilt incrementally by a file watcher in the running app, fully via `oxidant spec rebuild-index` on demand.

## Schema (sketch)

```sql
CREATE TABLE specs (
  id              TEXT PRIMARY KEY,         -- canonical ref, e.g. "tools/edit/apply-edits"
  kind            TEXT NOT NULL,            -- overview|component|contract|tool|flow|invariant|decision|glossary
  parent          TEXT,                     -- nullable for overview/glossary
  path            TEXT NOT NULL,            -- absolute path under worktree
  status          TEXT NOT NULL,            -- draft|active|deprecated
  order_idx       INTEGER,                  -- frontmatter `order:`, nullable
  line_count      INTEGER NOT NULL,
  last_modified   TEXT NOT NULL,            -- ISO-8601 from git log -1
  last_commit     TEXT NOT NULL             -- SHA from git log -1
);

CREATE TABLE spec_edges (
  src_id          TEXT NOT NULL,            -- referrer spec id
  dst_ref         TEXT NOT NULL,            -- referent (canonical ref text)
  edge_kind       TEXT NOT NULL,            -- implements | depends_on | body_ref
  PRIMARY KEY (src_id, dst_ref, edge_kind)
);

CREATE TABLE spec_code_paths (
  spec_id         TEXT NOT NULL,
  code_path       TEXT NOT NULL,            -- relative to worktree
  PRIMARY KEY (spec_id, code_path)
);

CREATE INDEX idx_specs_kind   ON specs(kind);
CREATE INDEX idx_specs_parent ON specs(parent);
CREATE INDEX idx_edges_dst    ON spec_edges(dst_ref);
CREATE INDEX idx_code_path    ON spec_code_paths(code_path);
```

## Build pipeline

1. Walk `spec/**/*.md`. Parse frontmatter via [[components/spec-tools/frontmatter]]; extract body `[[ref]]`s via the same crate.
2. For each file, run `git log -1 --format=%H%n%aI <path>` to get `last_commit` + `last_modified`. Batch — single `git log` call covering all paths via `--all` and `--name-only` is the optimisation for large trees.
3. Upsert into `specs`, replace `spec_edges` and `spec_code_paths` rows for the file.
4. Wrap the whole build in a single transaction.

## Watcher

- Use `notify` crate for cross-platform filesystem events.
- Debounce: 200 ms after the last event in a burst.
- On event: re-parse only the affected file(s); re-derive git metadata only for files known to git (untracked → skip).
- Watcher restart: if the index file is missing or its `schema_version` PRAGMA doesn't match the current build, drop and rebuild.

## Query surface used by other components

```rust
fn specs_by_kind(kind: &str) -> Vec<Row>;
fn dependents_of(spec_id: &str) -> Vec<Row>;         // edges where dst_ref = spec_id
fn dependencies_of(spec_id: &str) -> Vec<Row>;       // edges where src_id = spec_id
fn specs_for_code_path(code: &Path) -> Vec<Row>;     // reverse from spec_code_paths
fn recent(limit: usize) -> Vec<Row>;                 // ORDER BY last_modified DESC
fn orphans() -> Vec<Row>;                            // specs with no inbound edges (other than overview)
fn status_counts() -> HashMap<String, usize>;
```

## Non-goals

- The index is not the source of truth. Spec files are. The index is rebuildable from disk + git.
- The index does not store body text — that's [[components/spec-tools/search-index]]'s job.
- The index is local-only; not synced across machines.
