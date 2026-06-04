```yaml
---
id: vcs-explorations-list
kind: tool
parent: components/vcs/worktree-mgmt
order: 7
implements:
  - contracts/tool
depends_on:
  - components/vcs/worktree-mgmt
  - components/vcs/session-persistence
code:
  - crates/oxidant-vcs/src/tools/vcs_explorations_list.rs
status: active
responsibility: |
  List all known explorations (main + sub) with their worktree paths, branches, status, and basic resource usage.
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
  "explorations": [
    { "id": "01HZ...", "kind": "main",
      "worktree": "/repo", "branch": "main",
      "active":   true,
      "lsp_running": true,
      "target_size_mb": 1843 },
    { "id": "01J0...", "kind": "sub",
      "parent_id": "01HZ...",
      "worktree": "/repo/../.oxidant-worktrees/repo/explore-lsp-cache",
      "branch": "oxidant/explore/lsp-cache-3f2",
      "active":   false,
      "lsp_running": false,
      "target_size_mb": 0 }
  ]
}
```

## Used by

- The agent for situational awareness (cross-exploration context, when relevant).
- The GUI [[components/gui/exploration-list]] for the left-panel list.
