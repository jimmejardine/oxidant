---
id: fs
kind: component
parent: overview
order: 1
implements: []
depends_on: []
code:
  - crates/oxidant-tools/src/fs.rs
tests:
  - crates/oxidant-tools/src/fs.rs
status: active
responsibility: |
  Filesystem read/write/list capabilities scoped to the exploration's workspace root.
---

Generic filesystem operations. Backs [[tools/fs/fs-read]], [[tools/fs/fs-write]], [[tools/fs/glob]], [[tools/fs/grep]].

## Scope enforcement

Every operation resolves paths against `ToolContext::workspace_root` and rejects paths that escape (after canonicalisation). On Windows, paths go through `dunce::canonicalize` to handle UNC and case quirks.

## Read

Returns content as a UTF-8 string when possible. Binary files return `{ binary: true, size }` without content. Optional `offset` + `limit` (in lines) for large files.

## Write

`fs_write` creates new files or fully overwrites. For in-place modification use [[tools/edit/edit-string]] or [[tools/edit/apply-edits]] — write is the "this file doesn't exist yet, or I'm replacing it wholesale" tool.

## Glob

Pattern matching using `globset`. Returns relative paths from workspace root, sorted.

## Grep

Backed by `grep-searcher` (the engine underneath ripgrep). Streaming, ranking-free. For ranked search, prefer [[tools/search/text-search]].

## Permission categories

- `fs_read`, `glob`, `grep`: `ReadOnly` (auto-approved)
- `fs_write`: `Mutating` (prompted unless allowlisted)
