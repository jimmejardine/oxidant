---
id: merge-conflicts
kind: component
parent: overview
order: 9
implements: []
depends_on:
  - components/gui/exploration-list
  - components/vcs/worktree-mgmt
code:
  - crates/oxidant-gui/src/panels/merge_conflicts.rs
status: active
responsibility: |
  Centre-tab panel that opens when a merge-back returns conflicts. Lists conflicted files, lets the user resolve each via in-editor or external mergetool, then finalises the merge with a single commit + worktree cleanup.
---

## Trigger

Opened automatically by the exploration-list panel when `worktree::merge_back` returns `MergeOutcome` with `conflicts.len() > 0`. The exploration-list closure populates `SharedState.merge_conflicts` and pushes a `DockTab::MergeConflicts` into `pending_centre_tabs`. The panel reads everything it needs from `SharedState.merge_conflicts`.

## SharedState shape

```rust
pub struct MergeConflictsState {
    pub sub_id: ExplorationId,
    pub parent_id: ExplorationId,
    pub target_branch: String,
    /// Worktree where the merge was attempted — the parent's worktree.
    pub parent_worktree: PathBuf,
    /// Sub-exploration's branch (for cleanup post-finalise).
    pub sub_branch: String,
    /// Sub-exploration's worktree path (for cleanup post-finalise).
    pub sub_worktree: PathBuf,
    /// Whether the merge was started with `--squash`. Drives the
    /// finalise call (`git commit -m <message>` for squash; just
    /// `git commit` for `--no-ff` which already has a prepared msg).
    pub squash: bool,
    pub message: String,
    pub files: Vec<String>,
    pub resolved: HashSet<String>,
}
```

## UI

Header line: `Merge from <sub-branch> into <target-branch> — N conflicts, M resolved`.

Per file row:

```
[ ] crates/foo/src/bar.rs   [Open in editor] [Open in mergetool] [Mark resolved]
[✓] spec/components/x.md
```

- **Open in editor**: pushes the absolute file path into `SharedState.pending_centre_tabs` as a `DockTab::File { path }`. The file already contains conflict markers from git; the user edits and saves through the existing FileTabPanel.
- **Open in mergetool**: spawns `git mergetool --no-prompt -- <file>` as a tokio subprocess from `parent_worktree`. Fire-and-forget; on tool exit the user comes back to oxidant and clicks "Mark resolved".
- **Mark resolved**: runs `git add <file>` in `parent_worktree`, then adds the path to `resolved`. The row's checkbox flips to `✓`.

Footer:

- **Finalize merge commit** (enabled when `resolved == files.len()`): runs `git commit -m <message>` (squash) or `git commit` (no-ff). On success, removes the sub from `SharedState.explorations`, runs `worktree::discard` on `sub_worktree`, deletes `sub_branch` via `Git::branch_delete`, sets `active_id = parent_id`, clears `merge_conflicts`.
- **Abort merge**: runs `git merge --abort` for `--no-ff`, or `git reset --hard HEAD` for `--squash`. Clears `merge_conflicts`. Leaves the sub-exploration intact for the user to retry or rework.

## Why this shape

Two resolution paths cover the spectrum:

- The in-editor path handles the easy cases where conflict markers are short and obvious — no external tool needed.
- The mergetool path defers to whatever the user already configured (`git config merge.tool`). That's the existing standard for 3-way merge UIs; oxidant doesn't reinvent it.

No Rust merge-conflict crate is involved — git already produced the markers; the question is purely UX. See the rationale in [[flows/merge-back]] "Conflict handling".
