```yaml
id: worktree-mgmt
kind: component
parent: overview
order: 2
implements: []
depends_on:
  - components/vcs/git-shellout
code:
  - crates/oxidant-vcs/src/worktree.rs
status: active
responsibility: |
  Create, list, and remove git worktrees for explorations; resolve canonical paths; enforce one worktree per branch. Returns lean WorktreeHandle values that higher layers wrap into Exploration aggregates.
```

The worktree-lifecycle layer on top of [[components/vcs/git-shellout]]. Owns the choice of worktree paths, branch naming, and resource bookkeeping. Deliberately does **not** depend on [[components/core/exploration]] — the dependency goes the other way (an Exploration carries a worktree path produced here). The spawn-exploration flow ([[flows/spawn-exploration]]) is the assembly point that takes a `WorktreeHandle` and constructs the full `Exploration` aggregate.

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
pub struct WorktreeHandle {
    pub path: PathBuf,
    pub branch: String,
    pub created_at: DateTime<Utc>,
}

pub struct WorktreeSummary {
    pub path: PathBuf,
    pub branch: String,
    pub is_main: bool,
}

pub async fn spawn(repo: &Path, opts: SpawnOpts) -> Result<WorktreeHandle>;
pub async fn list(repo: &Path) -> Result<Vec<WorktreeSummary>>;
pub async fn discard(repo: &Path, path: &Path, force: bool) -> Result<()>;
pub async fn merge_back(
    repo: &Path,
    sub: &WorktreeHandle,
    target_branch: &str,
    opts: MergeBackOpts,
) -> Result<MergeOutcome>;

pub struct MergeBackOpts {
    /// When true, run `git merge --squash`: stage the combined diff
    /// as a single change without committing, then immediately commit
    /// with `message`. When false, run `git merge --no-ff`: preserve
    /// an explicit merge commit referencing both parent and sub history.
    /// Mutually exclusive — `--squash` and `--no-ff` cannot combine.
    pub squash: bool,
    /// Merge commit message. Falls back to a default of the form
    /// "Merge exploration <dir-name>: branch <branch>" when None.
    pub message: Option<String>,
}
```

The API talks in terms of paths and branches, not Explorations. Wrapping a `WorktreeHandle` into a full `Exploration` (with its `Conversation`, `LspHandle`, cancellation token, etc.) happens in [[flows/spawn-exploration]] — that's where the layering crosses from VCS plumbing into runtime aggregates.

## Spawn flow (mechanical)

1. Resolve target path; ensure it doesn't already exist.
2. `git worktree add <path> -b <branch>` from the repo's main worktree (or from the parent exploration's worktree for sub-of-sub).
3. Create `<path>/.oxidant/` directory.
4. Return the `WorktreeHandle`. The caller (spawn-exploration flow) builds the `Exploration` and persists the session.

## Discard flow

1. Confirm the worktree is clean (`git status --porcelain` empty) OR `force: true`.
2. Optionally archive the transcript file to `~/.local/share/oxidant/archive/<id>.jsonl` — driven by the caller, not by this component.
3. `git worktree remove [--force] <path>`.

## Model-facing tools backed by this component

- [[tools/vcs/vcs-explorations-list]] — read-only; agent + GUI
- [[tools/vcs/vcs-explore]] — spawn (GUI-only)
- [[tools/vcs/vcs-merge-back]] — merge back (GUI-only)
- [[tools/vcs/vcs-discard]] — remove worktree + archive transcript (GUI-only)

## See also

- [[flows/spawn-exploration]]
- [[flows/merge-back]]
- [[invariants/explorations-are-isolated]]
