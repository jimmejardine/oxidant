---
id: merge-back
kind: flow
parent: overview
order: 3
status: active
depends_on:
  - components/vcs/worktree-mgmt
  - components/vcs/git-shellout
  - components/gui/diagnostic-panel
---

# Merge a sub-exploration back into its parent

Reincorporates an exploration's work into the parent branch (typically `main`). User-initiated; the agent assists with conflicts.

## Trigger

"Merge" button on a sub-exploration in [[components/gui/exploration-list]], or on the exploration's window menu.

## Pre-flight

1. **Sub-exploration clean?** `git -C <sub> status --porcelain` must be empty. Otherwise prompt: commit, stash, or cancel.
2. **Sub-exploration ahead?** Compute `git -C <sub> log <parent-branch>..HEAD --oneline`. Empty → "no changes to merge"; cancel.
3. **Sub on the right branch?** If not, switch first.

## Merge

In the **parent** worktree (not the sub):

```
git checkout <parent-branch>            # if not already there
git merge --no-ff <sub-branch>
```

- `--no-ff` so the exploration boundary is visible in history.
- Default merge commit message: `"Merge exploration <slug>: <seed-or-summary>"`.

## Conflict handling

If `git merge` exits non-zero with conflicts:
1. A dedicated **conflict pane** opens in the parent's window listing conflicted files (from `git status --porcelain=v2`).
2. For each conflict, the user can:
   - Open the file (centre tab) and edit manually.
   - Ask the agent to resolve via a structured prompt that includes both `<<<<<<<` regions plus surrounding context.
   - Accept "ours" or "theirs".
3. After resolution, `cargo_check` runs; the conflict pane updates with any new diagnostics.
4. When the conflict pane reports zero conflicts and `cargo_check` is clean: `git commit` finalises the merge.

## Post-merge

- The sub-exploration's worktree is **not** automatically removed. User can discard separately via [[components/gui/exploration-list]] (which calls [[components/vcs/worktree-mgmt]]).
- The merge commit SHA + outcome are appended to both the sub and the main exploration's transcripts as system messages.

## Failure modes

- Detached HEAD on parent → switch to a branch first; surface in the conflict pane.
- Merge that would lose work in the parent (force-required) → refuse; prompt the user to update the parent first.
- Pre-commit hook failure → surface the hook's stderr; do not bypass with `--no-verify`.
