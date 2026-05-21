---
id: exploration
kind: component
parent: overview
order: 3
implements: []
depends_on:
  - components/core/conversation
  - components/vcs/worktree-mgmt
  - components/rust-tools/lsp
code:
  - crates/oxidant-core/src/exploration.rs
status: active
responsibility: |
  Bundle the runtime state of one self-contained workspace: conversation, worktree, branch, LSP handle, target/, cancellation token.
---

## Struct

```rust
pub struct Exploration {
    pub id: ExplorationId,                  // ULID
    pub kind: ExplorationKind,              // Main | Sub { parent_id }
    pub worktree_path: PathBuf,
    pub branch: String,
    pub conversation: Conversation,
    pub lsp_handle: Option<LspHandle>,      // spawned lazily on first LSP query
    pub target_dir: PathBuf,                // <worktree>/target
    pub cancellation: CancellationToken,
    pub created_at: DateTime<Utc>,
}
```

## Lifecycle

| Event | What changes |
|---|---|
| Created (main) | At app launch from the repo's existing checkout. |
| Created (sub) | Via [[flows/spawn-exploration]] — git worktree add, new branch, new conversation. |
| Opened in GUI | Window created; LSP not yet spawned. |
| First LSP query | `lsp_handle` populated (one-shot). |
| Cancelled | `cancellation.cancel()` — agent loop short-circuits at the next yield point. |
| Discarded | Worktree removed via `git worktree remove`; transcript archived. See [[flows/merge-back]] for the merge-then-discard flow. |
| Restored (app restart) | Reconstruct from on-disk `git worktree list` plus `.oxidant/sessions/`. LSP not spawned until first query. |

## Isolation

Two explorations never share filesystem state above `.git/objects` (shared by git worktree design). Specifically: no shared `target/` ([[invariants/target-dirs-are-never-shared]]), no shared rust-analyzer process, no cross-exploration tool dispatch.

The `ToolContext` an exploration's agent loop builds is scoped to `worktree_path` — tools must respect it and not escape. See [[contracts/tool]] invariants.

## ID format

`ExplorationId` is a ULID. Used in transcript filenames, GUI window keys, and as the directory suffix in cases where multiple slug-collisions occur on the same branch name.
