```yaml
---
id: vcs-diff
kind: tool
parent: components/vcs/git-shellout
order: 2
implements:
  - contracts/tool
depends_on:
  - components/vcs/git-shellout
code:
  - crates/oxidant-vcs/src/tools/vcs_diff.rs
status: active
responsibility: |
  Return a structured diff between the worktree and a revspec (or between two revspecs).
---
```

`category`: `ReadOnly`.

## Schema

```json
{
  "type": "object",
  "properties": {
    "revspec":   { "type": "string", "description": "default: working tree vs HEAD" },
    "name_only": { "type": "boolean", "default": false },
    "path_glob": { "type": "string", "description": "limit to matching paths" }
  }
}
```

## Result

```json
{
  "files": [
    {
      "path": "src/foo.rs",
      "status": "modified",
      "additions": 12,
      "deletions": 4,
      "hunks": [
        { "old_range": [42, 4], "new_range": [42, 12], "text": "@@ ..." }
      ]
    }
  ]
}
```

With `name_only: true`, `hunks` is omitted and only paths/statuses are returned (much smaller payload).
