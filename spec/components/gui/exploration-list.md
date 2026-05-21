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

- **Open**: open or focus the exploration's OS window.
- **Spawn sub** (top of the list, prominent button): create a new sub-exploration off the current branch via [[flows/spawn-exploration]].
- **Merge** (sub only): merge back into parent branch via [[flows/merge-back]].
- **Discard** (sub only, confirm dialog): remove the worktree and archive the transcript.

## Sorting

1. Main first.
2. Then sub-explorations by last-activity (most recent first).

## Resource polling

Process memory via `sysinfo` crate, `target/` size via `walkdir` + summing file sizes. Polled every 5 seconds for visible rows only.
