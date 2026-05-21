---
id: worktree-mgmt
kind: component
parent: overview
order: 2
implements: []
depends_on:
  - components/vcs/git-shellout
  - components/core/exploration
code:
  - crates/oxidant-vcs/src/worktree.rs
status: active
responsibility: |
  Create, list, and remove git worktrees for explorations; resolve canonical paths; enforce one worktree per branch.
---

The exploration-lifecycle layer on top of [[components/vcs/git-shellout]]. Owns the choice of worktree paths, branch naming, and resource bookkeeping.

## Path convention

`<repo-parent>/.oxidant-worktrees/<repo-name>/<branch-slug>/`

- `repo-parent` is the directory containing the main worktree.
- `repo-name` is the main worktree's directory name.
- `branch-slug` is the branch name with `/` replaced by `_` and other non-safe chars stripped.

All paths resolved through `dunce::canonicalize` on Windows.

## Branch naming

Auto-generated branches: `oxidant/explore/<slug>-<short-ts>`. User can rename via `vcs_branch_*` tools. Slugs derived from optional seed prompt; default `unnamed`.

## API

```rust
pub async fn spawn(repo: &Path, opts: SpawnOpts) -> Result<Exploration>;
pub async fn list(repo: &Path) -> Result<Vec<ExplorationSummary>>;
pub async fn discard(repo: &Path, expl: &Exploration) -> Result<()>;
pub async fn merge_back(repo: &Path, expl: &Exploration, target_branch: &str) -> Result<MergeOutcome>;
```

## Spawn flow (mechanical)

1. Resolve target path; ensure it doesn't already exist.
2. `git worktree add <path> -b <branch>` from the repo's main worktree (or from the parent exploration's worktree for sub-of-sub).
3. Create `<path>/.oxidant/` directory.
4. Build the `Exploration` struct (no LSP yet; lazy spawn).
5. Persist the exploration entry via [[components/vcs/session-persistence]].

## Discard flow

1. Confirm the worktree is clean (`git status --porcelain` empty) OR `force: true`.
2. Optionally archive the transcript file to `~/.local/share/oxidant/archive/<id>.jsonl`.
3. `git worktree remove [--force] <path>`.
4. Delete the persistence entry.

## Model-facing tools backed by this component

- [[tools/vcs/vcs-explorations-list]] — read-only; agent + GUI
- [[tools/vcs/vcs-explore]] — spawn (GUI-only)
- [[tools/vcs/vcs-merge-back]] — merge back (GUI-only)
- [[tools/vcs/vcs-discard]] — remove worktree + archive transcript (GUI-only)

## See also

- [[flows/spawn-exploration]]
- [[flows/merge-back]]
- [[invariants/explorations-are-isolated]]
