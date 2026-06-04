```yaml
---
id: git-shellout
kind: component
parent: overview
order: 1
implements: []
depends_on: []
code:
  - crates/oxidant-vcs/src/git.rs
status: active
responsibility: |
  Wrap subprocess calls to the git CLI behind a typed Rust API; never use git2/gix.
---
```

Low-level git wrapper. Every call is a single `tokio::process::Command::new("git")` invocation with structured arg construction and parsed output.

See [[decisions/0006-shell-out-to-git-cli]].

## Surface

```rust
pub struct Git { cwd: PathBuf }

impl Git {
    pub fn at(cwd: impl Into<PathBuf>) -> Self;

    pub async fn status(&self) -> Result<StatusOutput>;
    pub async fn diff(&self, revspec: Option<&str>, name_only: bool) -> Result<DiffOutput>;
    pub async fn log(&self, opts: LogOpts) -> Result<Vec<Commit>>;
    pub async fn show_file(&self, sha: &str, path: &Path) -> Result<String>;            // git show <sha>:<path>
    pub async fn commit(&self, message: &str, paths: &[PathBuf]) -> Result<String>;     // returns SHA
    pub async fn branch_create(&self, name: &str, base: Option<&str>) -> Result<()>;
    pub async fn checkout(&self, branch: &str) -> Result<()>;
    pub async fn merge(&self, branch: &str, opts: MergeOpts) -> Result<MergeOutcome>;

    pub async fn worktree_add(&self, path: &Path, branch: &str) -> Result<()>;
    pub async fn worktree_list(&self) -> Result<Vec<Worktree>>;
    pub async fn worktree_remove(&self, path: &Path, force: bool) -> Result<()>;
}
```

## Argument hygiene

- Paths are passed as separate args (never shell-interpolated).
- Branch names are validated against `^[a-zA-Z0-9._\-/]+$` before invocation.
- Refspecs are similarly screened — no opportunity for `;` or `$(...)` injection.

## Output parsing

Prefer porcelain formats with stable schemas:
- `git status --porcelain=v2 --branch` → typed.
- `git log --pretty=format:'%H%x09%aI%x09%an%x09%s' --name-status` → typed.
- `git worktree list --porcelain` → typed.

Avoid colour/locale-dependent output: always pass `LC_ALL=C` and `--no-pager` in the env, never request `--color=always`.

## Errors

Every command's exit code + stderr is surfaced. Non-zero with empty stderr is unusual and reported with the full command line.

`show_file` distinguishes "file not at this revision" from other failures: a non-zero exit with stderr matching `path '...' does not exist in '<sha>'` becomes `GitError::FileNotAtRevision { sha, path }`, so callers (e.g. [[components/gui/diff-history-panel]]) can render the absence cleanly rather than surfacing a generic command failure.

## Push / fetch / pull

Out of scope for MVP. The agent does not touch remotes; the user does that themselves. Adding remote ops is a future ADR.

## Model-facing tools backed by this component

- [[tools/vcs/vcs-status]] — branch, dirty files, ahead/behind
- [[tools/vcs/vcs-diff]] — structured diff against a revspec
- [[tools/vcs/vcs-log]] — recent commits as structured records
- [[tools/vcs/vcs-commit]] — stage + commit; never pushes
- [[tools/vcs/vcs-branch-create]] / [[tools/vcs/vcs-branch-checkout]] — branch ops within an exploration
