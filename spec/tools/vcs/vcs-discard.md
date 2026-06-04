```yaml
---
id: vcs-discard
kind: tool
parent: components/vcs/worktree-mgmt
order: 10
implements:
  - contracts/tool
depends_on:
  - components/vcs/worktree-mgmt
code:
  - crates/oxidant-vcs/src/tools/vcs_discard.rs
status: active
responsibility: |
  Remove a sub-exploration's worktree and archive its transcript. GUI-only — destructive, user-initiated.
---
```

`category`: `Mutating`. **GUI-only**.

## Schema

```json
{
  "type": "object",
  "required": ["exploration_id"],
  "properties": {
    "exploration_id": { "type": "string" },
    "force":          { "type": "boolean", "default": false, "description": "remove even if the worktree is dirty" },
    "archive":        { "type": "boolean", "default": true, "description": "save transcript to ~/.local/share/oxidant/archive/" }
  }
}
```

## Result

```json
{
  "discarded":      true,
  "archived_to":    "~/.local/share/oxidant/archive/01J0....jsonl",
  "branch_left":    "oxidant/explore/lsp-cache-3f2"
}
```

## Behaviour

- Refuses to discard the **main** exploration.
- Without `force`, refuses if `git -C <worktree> status --porcelain` is non-empty.
- Runs `git worktree remove [--force] <path>`; the branch itself is left in place (user can `git branch -d` separately if they wish).
- Archives the `.jsonl` transcript to the user-level archive directory.
- Frees the rust-analyzer process and the agent loop task.

## See also

- [[flows/merge-back]] — standard precursor when keeping the work
