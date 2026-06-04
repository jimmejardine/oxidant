```yaml
---
id: edits-are-atomic
kind: invariant
order: 1
status: active
responsibility: |
  Every WorkspaceEdit is applied all-or-nothing across all files, with rollback on any post-edit validation failure.
depends_on:
  - components/tools/workspace-edit-substrate
---
```

For every `WorkspaceEdit` flowing through [[components/tools/workspace-edit-substrate]]:

1. Either every `TextEdit` in every file is applied, or none is. The filesystem never observes a partially-applied multi-file edit.
2. If any post-edit `syn::parse_file` rejects a `.rs` file, every change in the WorkspaceEdit — including changes to files that did parse — is reverted before the substrate returns.
3. If any `expected_text` check fails, no file is modified; the substrate returns the conflict report without touching disk.

Implemented via temp-file-plus-rename per file, with the rename batch reversed on partial failure. Cross-volume edits use copy+delete and accept that on extreme crash boundaries (kernel panic between two renames) the invariant is best-effort.

Consumers may rely on this when chaining edits in a single agent turn: a successful return from [[tools/edit/apply-edits]] or [[tools/edit/edit-string]] means the worktree is in the post-edit state, full stop.
