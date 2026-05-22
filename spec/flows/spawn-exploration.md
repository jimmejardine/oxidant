---
id: spawn-exploration
kind: flow
parent: overview
order: 2
status: active
responsibility: |
  Create a fresh sub-exploration from an existing one: compute target path and branch name, run `git worktree add`, initialise `.oxidant/` state, build an Exploration struct, persist it, open a viewport.
depends_on:
  - components/vcs/worktree-mgmt
  - components/vcs/session-persistence
  - components/core/exploration
  - components/gui/viewport
---

# Spawn a sub-exploration

A user-initiated flow to open a fresh, isolated workspace from any existing exploration.

## Trigger

- "Spawn sub-exploration" button in the exploration list, or
- "Explore this idea" context-menu action on any transcript line (the line's text becomes the seed prompt).

Note: agent tool calls **cannot** trigger this — see [[components/vcs/worktree-mgmt]] for the rationale. Spawning is always user-driven.

## Steps

1. **Collect seed.** Optional name + optional seed prompt. If none, the slug defaults to a short timestamp.
2. **Compute target path** per the [[components/vcs/worktree-mgmt]] convention: `<repo-parent>/.oxidant-worktrees/<repo-name>/<branch-slug>/`. Reject if the path exists.
3. **Branch.** Compute branch name `oxidant/explore/<slug>-<short-ts>`. Validate against git's ref rules.
4. **Create worktree.** `git worktree add <path> -b <branch>` from the parent exploration's worktree (so the new branch starts from the parent's HEAD).
5. **Initialise `.oxidant/`.** Create the per-worktree state directory; write a default `dock-layout.json`; append the worktree to git's `info/exclude`.
6. **Build `Exploration` struct** — id, kind: `Sub { parent_id }`, paths, fresh conversation. LSP not yet spawned.
7. **Persist** via [[components/vcs/session-persistence]].
8. **Open viewport** — `ctx.show_viewport_deferred(...)` from the spawning OS window. The new window opens with the default dock layout. Title bar shows `[sub: <branch-slug>]`.
9. **Seed conversation.** If a seed prompt was given, append it as the first `User` message and start the agent loop. Otherwise wait for user input.
10. **Lazy LSP.** Rust-analyzer remains unspawned until the first LSP-using tool call.

## Failure modes

- Worktree path already exists → reject with a friendly error and the existing path.
- Branch name conflict → suggest a `-2` suffix.
- Disk full → surface from git's stderr; suggest discarding stale explorations.

## Reverse flow

To undo: close the window and discard via [[components/gui/exploration-list]], which delegates to [[components/vcs/worktree-mgmt]]'s discard path.
