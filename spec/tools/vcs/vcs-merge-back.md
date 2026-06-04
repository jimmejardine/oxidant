```yaml
id: vcs-merge-back
kind: tool
parent: components/vcs/worktree-mgmt
order: 9
implements:
  - contracts/tool
depends_on:
  - components/vcs/worktree-mgmt
  - components/vcs/git-shellout
code:
  - crates/oxidant-vcs/src/tools/vcs_merge_back.rs
status: active
responsibility: |
  Merge a sub-exploration's branch back into its parent branch. GUI-only — destructive cross-exploration operations are user-initiated.
```

`category`: `Mutating`. **GUI-only**.

## Schema

```json
{
  "type": "object",
  "required": ["sub_exploration_id"],
  "properties": {
    "sub_exploration_id": { "type": "string" },
    "target_branch":      { "type": "string", "default": "main" }
  }
}
```

## Result

```json
{
  "merge_commit": "deadbee",
  "conflicts":    [],
  "outcome":      "merged"
}
```

When conflicts occur, `outcome` is `"conflicted"` and the GUI's conflict pane opens. The agent can be invoked to help resolve once the user accepts that flow.

## See also

- [[flows/merge-back]] — the full UX flow including conflict resolution
- [[tools/vcs/vcs-discard]] — typical follow-up after a successful merge
