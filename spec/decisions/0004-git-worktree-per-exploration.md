```yaml
id: 0004-git-worktree-per-exploration
kind: decision
order: 4
status: active
date: 2026-05-21
responsibility: |
  Each side exploration gets its own git worktree + branch + rust-analyzer + target/, providing filesystem-level isolation between concurrent ideas.
```

# 0004 — One git worktree per exploration

## Status

Active.

## Context

A code agent doing useful work often wants to try multiple ideas in parallel: "what if the cache evicts on LRU vs LFU", "compare this refactor with that one". Without isolation, parallel chats stomp the same files and confuse each other (and the LSP).

Options considered:
- **Shared filesystem, branch-switching as needed** — checkpoints get fragile, the LSP re-indexes constantly, mistakes overwrite real work.
- **Full clones per exploration** — works, but multi-GB per branch and duplicated object database.
- **Git worktrees** — sibling working trees sharing `.git/objects`. Cheap to create, true filesystem isolation, each tree can be on its own branch.

## Decision

Each side exploration is a git worktree under `<repo-parent>/.oxidant-worktrees/<repo-name>/<branch-slug>/`. New worktrees are created via `git worktree add <path> -b <branch>` (shelling out — see [[decisions/0006-shell-out-to-git-cli]]). The main exploration uses the original checkout.

Each worktree has its own:
- Branch (auto-named `oxidant/explore/<slug>-<ts>`, renameable)
- Rust-analyzer process
- `target/` directory (never shared — see [[decisions/0005-no-shared-target-dir-use-sccache]])
- Conversation transcript in `<worktree>/.oxidant/sessions/<exploration_id>.jsonl`

## Consequences

Positive: filesystem-level isolation; user's mental model "one branch = one exploration" maps directly; merge-back is a real git merge.

Negative: per-worktree rust-analyzer is RAM-expensive (500MB–2GB each). Multiple stale worktrees accumulate disk space. Mitigation: GUI shows per-exploration resource badges; explicit "discard" UX; on-launch lazy LSP spawn.

## Related

- [[components/vcs/worktree-mgmt]] — implementation
- [[flows/spawn-exploration]] / [[flows/merge-back]] — lifecycle
- [[invariants/explorations-are-isolated]]
