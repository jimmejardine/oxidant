```yaml
id: explorations-are-isolated
kind: invariant
order: 3
status: active
depends_on:
  - components/core/exploration
  - components/vcs/worktree-mgmt
responsibility: |
  Operations in one exploration never modify another exploration's worktree, target/, LSP state, or conversation.
```

Two explorations may share `.git/objects` (the git worktree design point) but nothing else above that. Specifically:

- **Filesystem.** Tool calls resolve paths against `ToolContext::workspace_root` (the exploration's worktree) and reject any path that escapes after canonicalisation.
- **Build.** Each exploration's cargo runs with `CARGO_TARGET_DIR=<worktree>/target`. See [[invariants/target-dirs-are-never-shared]].
- **Language services.** Each exploration spawns its own `rust-analyzer` subprocess; queries route to that instance only.
- **State.** Conversation, transcript file, dock layout, model settings, allowlist all live under `<worktree>/.oxidant/`.
- **Agent loop.** Each exploration's tokio task tree is independent; cancellation tokens do not bleed across.

## Failure mode if violated

Two parallel agent edits to overlapping files would corrupt each other's syn-parse view; LSP indexes would race; transcript persistence would interleave lines. Outcome: undebuggable data loss.

## Enforcement

- `ToolContext::workspace_root` is required at construction and held by every tool dispatch.
- Path canonicalisation runs before every read/write and before LSP `file://` URI construction.
- Tests in `oxidant-core` assert that the path resolver rejects escapes (`../`, absolute paths outside root, junction traversal on Windows).
