```yaml
id: vcs-commit
kind: tool
parent: components/vcs/git-shellout
order: 3
implements:
  - contracts/tool
depends_on:
  - components/vcs/git-shellout
code:
  - crates/oxidant-vcs/src/tools/vcs_commit.rs
status: active
responsibility: |
  Stage paths and create a commit in the active exploration's worktree; never pushes to a remote.
```

`category`: `Mutating`.

## Schema

```json
{
  "type": "object",
  "required": ["message"],
  "properties": {
    "message": { "type": "string", "minLength": 1 },
    "paths":   { "type": "array", "items": { "type": "string" }, "description": "default: all changes in the worktree" },
    "amend":   { "type": "boolean", "default": false }
  }
}
```

## Result

```json
{ "sha": "deadbee", "branch": "oxidant/explore/foo", "files_committed": 7 }
```

## Behaviour

- Stages `paths` (or all changes if absent).
- Commits with `message`. Pre-commit hooks run as configured by the repo — failures surface verbatim; oxidant never passes `--no-verify`.
- `amend: true` amends the previous commit but never rewrites already-pushed history; the tool checks `git log` against `@{upstream}` first and refuses if the previous commit is on a remote tracking branch.
- Never pushes. Push is out of scope per [[components/vcs/git-shellout]].

## Co-authoring

The commit trailer is appended automatically when the agent makes the commit:
```
Co-Authored-By: oxidant <oxidant@local>
```
Disabled via [[components/config/settings]].
