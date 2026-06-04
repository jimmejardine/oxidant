```yaml
---
id: vcs-status
kind: tool
parent: components/vcs/git-shellout
order: 1
implements:
  - contracts/tool
depends_on:
  - components/vcs/git-shellout
code:
  - crates/oxidant-vcs/src/tools/vcs_status.rs
status: active
responsibility: |
  Return current branch, dirty files (with statuses), and ahead/behind counts for the active exploration's worktree.
---
```

`category`: `ReadOnly`.

## Schema

```json
{ "type": "object", "properties": {} }
```

## Result

```json
{
  "branch":   "oxidant/explore/lsp-cache-eviction-7f3a",
  "upstream": "origin/main",
  "ahead":    2,
  "behind":   0,
  "files": [
    { "path": "src/foo.rs", "index": "M", "worktree": "." },
    { "path": "src/new.rs", "index": "?", "worktree": "?" }
  ]
}
```

Status codes follow `git status --porcelain=v2`.
