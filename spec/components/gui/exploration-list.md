---
id: exploration-list
kind: component
parent: overview
order: 8
implements: []
depends_on:
  - components/core/exploration
  - components/vcs/session-persistence
code:
  - crates/oxidant-gui/src/panels/exploration_list.rs
status: active
responsibility: |
  Left-panel tab listing all explorations for the current repo with status, branch, resource badges, and spawn/discard actions.
---

## Row content

```
[main]    main           ●   2 GB RAM   45 MB disk    [open]
[sub]     explore-lsp    ●   1.5 GB     12 MB         [open] [merge] [discard]
[sub]     refactor-x         (cold)     5 MB          [open]
```

- `[main]` / `[sub]` badge
- Branch name (clickable to copy)
- `●` = active (LSP spawned, agent loop running)
- RAM/disk badges from a periodic poll of the LSP process and `du`-equivalent on `target/`
- `(cold)` = exploration exists on disk but LSP not spawned (saves resources)

## Actions

- **Spawn sub** (top of the list, prominent button): create a new sub-exploration off the active branch via [[flows/spawn-exploration]].
- **Switch active** (row click on a non-active row): MVP — flip `SharedState.active_id` to the clicked exploration so the rest of the GUI (transcript, chat input, file tree, health check) operates on that exploration's conversation and worktree. The previously-active exploration's runtime stays in memory; the user can switch back.
- **Merge (squash)** (sub only): merge back into parent branch via [[flows/merge-back]] with `MergeBackOpts { squash: true, … }`. On clean merge, cleans up the sub-worktree and switches the active back to the parent. On conflict, surfaces [[components/gui/merge-conflicts]] (Phase 3) for resolution.
- **Discard** (sub only, two-stage warning): remove the worktree and archive the transcript. First click runs a pre-check against the parent — `Git::commits_ahead(parent_branch, sub_branch)`. If the sub has 0 unmerged commits, the discard fires immediately (matches the historical behaviour). If it has any unmerged commits, the button flips to a red `Confirm discard ({N} unmerged)` and the status line shows `{N} commit(s) not merged into <parent_branch>; click Discard again to confirm.` — a second click within 5 seconds proceeds; otherwise the arm expires. The `worktree::discard` call still refuses on dirty worktree files independently.

## Sorting

1. Main first.
2. Then sub-explorations by last-activity (most recent first).

## SharedState shape

`SharedState` carries an ordered map of explorations plus a pointer to the active one:

```rust
pub struct SharedState {
    pub explorations: indexmap::IndexMap<ExplorationId, Exploration>,
    pub active_id: ExplorationId,
    // …
}

impl SharedState {
    pub fn active(&self) -> &Exploration { … }
    pub fn active_mut(&mut self) -> &mut Exploration { … }
}
```

Every panel that previously read `state.exploration.…` now reads `state.active().…`. The transcript, chat input, file tree, health check, and live-turn streaming all operate against the active exploration. `pending_centre_tabs`, `pending_chat_prompt`, `pending_continue`, `live_turn`, `health`, and the editor buffers are window-scoped (not per-exploration) — they belong to the host process, not to any single exploration's runtime state. **Multi-viewport per exploration is a follow-up**; under MVP one window holds all explorations and switches between them.

## Resource polling

Process memory via `sysinfo` crate, `target/` size via `walkdir` + summing file sizes. Polled every 5 seconds for visible rows only.
