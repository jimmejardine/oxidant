```yaml
id: vcs-branch-checkout
kind: tool
parent: components/vcs/git-shellout
order: 6
implements:
  - contracts/tool
depends_on:
  - components/vcs/git-shellout
code:
  - crates/oxidant-vcs/src/tools/vcs_branch_checkout.rs
status: active
responsibility: |
  Switch the active exploration's worktree to a different branch.
```

`category`: `Mutating`.

## Schema

```json
{
  "type": "object",
  "required": ["branch"],
  "properties": {
    "branch":      { "type": "string" },
    "create":      { "type": "boolean", "default": false, "description": "create the branch from HEAD if absent" }
  }
}
```

## Result

```json
{ "branch": "feat/x", "switched_from": "main" }
```

## Pre-flight

- If working tree is dirty, the tool refuses unless the user has set `permissions.allow_dirty_checkout = true`. Stash/commit first.
- If the target branch is checked out in another worktree (forbidden by git), the tool surfaces the conflict and refuses.

## After-effects

rust-analyzer gets a workspace change notification; cargo will need to rebuild against the new HEAD. The GUI surfaces this in the diagnostic panel.
