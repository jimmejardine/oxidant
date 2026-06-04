```yaml
id: merge-back
kind: flow
parent: overview
order: 3
status: active
responsibility: |
  Reincorporate a sub-exploration's work into its parent branch via `git merge --no-ff`, with the agent assisting conflict resolution in the parent's window.
depends_on:
  - components/vcs/worktree-mgmt
  - components/vcs/git-shellout
  - components/gui/diagnostic-panel
```

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
git merge --no-ff <sub-branch>          # default
   OR
git merge --squash <sub-branch>         # squash variant
git commit -m "<message>"               # squash needs an explicit commit
```

- `--no-ff` preserves the exploration boundary in history as an explicit merge commit referencing both parents.
- `--squash` collapses the sub's commits into one staged change on the parent; the caller finalises with `git commit`. Use when the exploration's incremental commit history adds no value to the parent — e.g. a noisy back-and-forth where only the final state matters.
- The two modes are mutually exclusive. Picked at call time via `MergeBackOpts { squash, message }`; see [[components/vcs/worktree-mgmt]].
- Default merge commit message: `"Merge exploration <slug>: <seed-or-summary>"`.

## Conflict handling

If `git merge` exits non-zero with conflicts:

1. `SharedState.merge_conflicts` populates with `{ sub_id, parent_id, target_branch, message, files: Vec<String>, resolved: HashSet<String> }`. The exploration-list panel pushes a `DockTab::MergeConflicts` into the centre area so the resolution panel opens automatically; see [[components/gui/merge-conflicts]].

2. For each file the user picks one of two paths:
   - **Open in editor** (default): the file opens in a centre tab via `pending_centre_tabs`. The file on disk already carries standard `<<<<<<<` / `=======` / `>>>>>>>` markers from git; the user edits them out in oxidant's editor and saves. "Mark resolved" runs `git add <file>`.
   - **Open in mergetool**: spawns `git mergetool --no-prompt -- <file>` as a subprocess. The user's configured external tool launches (kdiff3, meld, VS Code, vimdiff — whatever `git config merge.tool` points at). When the tool exits cleanly, `git add` is implicit; the user clicks "Mark resolved" to confirm.

3. Once every file in `files` is in `resolved`, the **Finalize merge commit** button enables. For squash merges this runs `git commit -m <message>`. For `--no-ff` merges that hit a conflict it runs `git commit` to finish the in-progress merge using the merge message git already prepared.

4. **Abort merge** at any time runs `git merge --abort` for `--no-ff` paths, or `git reset --hard HEAD` for `--squash` paths (which doesn't set `MERGE_HEAD`). Either resets the parent's index to a clean state; the sub-exploration is left intact.

**Why shell out to `git mergetool` instead of a Rust merge crate.** Git has already done the 3-way merge and inserted standard conflict markers — the work left is purely UX. `git mergetool` already brokers every mature external diff/merge tool via the user's existing `git config merge.tool`. Reimplementing that brokerage in Rust (gitoxide's `gix-merge`, the `merge3` crate, etc.) duplicates a battle-tested git feature and reads the user's tooling preference twice. Aligns with [[decisions/0006-shell-out-to-git-cli]].

## Post-merge

- After a clean finalise (no conflicts, or conflicts resolved and committed): the exploration-list panel removes the sub from `SharedState.explorations`, runs `worktree::discard` on its path (+ deletes the branch via `Git::branch_delete`), and switches `active_id` back to the parent. Cleanup is automatic; the user doesn't need to discard separately.
- The merge commit SHA + outcome are appended to both the sub and the main exploration's transcripts as system messages.

## Failure modes

- Detached HEAD on parent → switch to a branch first; surface in the conflict pane.
- Merge that would lose work in the parent (force-required) → refuse; prompt the user to update the parent first.
- Pre-commit hook failure → surface the hook's stderr; do not bypass with `--no-verify`.
