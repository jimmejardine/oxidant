```yaml
id: vcs-branch-create
kind: tool
parent: components/vcs/git-shellout
order: 5
implements:
  - contracts/tool
depends_on:
  - components/vcs/git-shellout
code:
  - crates/oxidant-vcs/src/tools/vcs_branch_create.rs
status: active
responsibility: |
  Create a new branch in the active exploration's worktree (does not switch to it).
```

`category`: `Mutating`.

## Schema

```json
{
  "type": "object",
  "required": ["name"],
  "properties": {
    "name": { "type": "string", "minLength": 1 },
    "base": { "type": "string", "default": "HEAD", "description": "revspec to branch from" }
  }
}
```

## Result

```json
{ "branch": "feat/x", "based_on": "deadbee" }
```

## Validation

Branch name is screened against `^[a-zA-Z0-9._\-/]+$` and against git's own ref rules. Rejected → friendly error with the offending character.

## Scope

Only creates within the current exploration's branch namespace. To create a new exploration with its own worktree, the user uses [[flows/spawn-exploration]] from the GUI; this tool only creates a plain branch in the current worktree.
