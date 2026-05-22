// Realises spec/components/vcs/worktree-mgmt.md.
//
// Worktree-lifecycle layer on top of git-shellout. Owns the
// `<repo-parent>/.oxidant-worktrees/<repo-name>/<branch-slug>/` path
// convention, the `oxidant/explore/<slug>-<short-ts>` branch naming
// scheme, and the spawn/list/discard/merge-back surface. Does NOT
// know about Exploration — the spawn-exploration flow is where
// WorktreeHandle gets wrapped into the full runtime aggregate.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Serialize;

use crate::git::{Git, GitError, MergeOpts, MergeOutcome, Worktree};

#[derive(Debug, Clone, Serialize)]
pub struct WorktreeHandle {
    pub path: PathBuf,
    pub branch: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorktreeSummary {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_main: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SpawnOpts {
    pub name: Option<String>,
    pub base: Option<String>,
    pub branch_prefix: Option<String>, // default "oxidant/explore"
}

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("io error on {path}: {message}", path = path.display())]
    Io { path: PathBuf, message: String },
    #[error("worktree path already exists: {0}")]
    PathExists(PathBuf),
    #[error("invalid input: {0}")]
    Invalid(String),
}

pub async fn spawn(repo: &Path, opts: SpawnOpts) -> Result<WorktreeHandle, WorktreeError> {
    let canonical_repo = dunce::canonicalize(repo).map_err(|e| WorktreeError::Io {
        path: repo.to_path_buf(),
        message: e.to_string(),
    })?;
    let parent = canonical_repo.parent().ok_or_else(|| {
        WorktreeError::Invalid("repo has no parent dir; expected workspace under a containing directory".into())
    })?;
    let repo_name = canonical_repo
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| WorktreeError::Invalid("repo path has no file name".into()))?;

    let slug = opts
        .name
        .as_deref()
        .map(slugify)
        .unwrap_or_else(|| "unnamed".to_string());
    let ts = short_timestamp();
    let prefix = opts.branch_prefix.as_deref().unwrap_or("oxidant/explore");
    let branch = format!("{prefix}/{slug}-{ts}");
    let dir_slug = branch.replace('/', "_");

    let worktree_dir = parent
        .join(".oxidant-worktrees")
        .join(&repo_name)
        .join(&dir_slug);
    if worktree_dir.exists() {
        return Err(WorktreeError::PathExists(worktree_dir));
    }

    let git = Git::at(&canonical_repo);
    git.worktree_add(&worktree_dir, &branch).await?;
    let canonical_path = dunce::canonicalize(&worktree_dir).map_err(|e| WorktreeError::Io {
        path: worktree_dir.clone(),
        message: e.to_string(),
    })?;

    // .oxidant/ state directory inside the worktree; gitignored via the
    // worktree's local exclude file so it never lands in commits.
    let oxidant_dir = canonical_path.join(".oxidant");
    std::fs::create_dir_all(&oxidant_dir).map_err(|e| WorktreeError::Io {
        path: oxidant_dir.clone(),
        message: e.to_string(),
    })?;
    let _ = std::fs::create_dir_all(oxidant_dir.join("sessions"));

    // Add .oxidant/ to info/exclude (worktree-local exclude) so we never
    // accidentally stage it. For a worktree the file lives in
    // <main-repo>/.git/worktrees/<id>/info/exclude — locating it precisely
    // is harder than just touching the global per-worktree exclude file via
    // `git config core.excludesFile`. For MVP we write to .git/info/exclude
    // in the main repo (which is shared); the user can override later.
    let _ = append_to_exclude(&canonical_repo, ".oxidant/");

    Ok(WorktreeHandle {
        path: canonical_path,
        branch,
        created_at: Utc::now(),
    })
}

pub async fn list(repo: &Path) -> Result<Vec<WorktreeSummary>, WorktreeError> {
    let git = Git::at(repo);
    let entries: Vec<Worktree> = git.worktree_list().await?;
    Ok(entries
        .into_iter()
        .map(|w| WorktreeSummary {
            path: w.path,
            branch: w.branch,
            is_main: w.is_main,
        })
        .collect())
}

pub async fn discard(repo: &Path, path: &Path, force: bool) -> Result<(), WorktreeError> {
    let git = Git::at(repo);
    if !force {
        // Pre-flight: refuse if the target worktree has uncommitted changes.
        let target_git = Git::at(path);
        let status = target_git.status().await?;
        if !status.files.is_empty() {
            return Err(WorktreeError::Invalid(format!(
                "worktree {} is dirty ({} files); use force=true to discard anyway",
                path.display(),
                status.files.len()
            )));
        }
    }
    git.worktree_remove(path, force).await?;
    Ok(())
}

pub async fn merge_back(
    repo: &Path,
    sub: &WorktreeHandle,
    target_branch: &str,
) -> Result<MergeOutcome, WorktreeError> {
    let parent_git = Git::at(repo);
    // The merge happens in the *parent* worktree, not the sub.
    parent_git
        .checkout(target_branch, false)
        .await?;
    let outcome = parent_git
        .merge(
            &sub.branch,
            MergeOpts {
                no_ff: true,
                message: Some(format!(
                    "Merge exploration {}: branch {}",
                    sub.path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    sub.branch
                )),
            },
        )
        .await?;
    Ok(outcome)
}

// ---------------------------------------------------------------- helpers

static SLUG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[^a-z0-9]+").expect("known-good regex")
});

fn slugify(input: &str) -> String {
    let lower = input.to_lowercase();
    let collapsed = SLUG_RE.replace_all(&lower, "-");
    let trimmed = collapsed.trim_matches('-');
    if trimmed.is_empty() {
        "unnamed".to_string()
    } else {
        trimmed.chars().take(40).collect()
    }
}

fn short_timestamp() -> String {
    // Mod against 16^4 → 4 hex chars, enough to disambiguate concurrent spawns.
    let secs = Utc::now().timestamp() as u64;
    format!("{:04x}", secs & 0xffff)
}

fn append_to_exclude(repo: &Path, line: &str) -> std::io::Result<()> {
    let exclude_path = repo.join(".git").join("info").join("exclude");
    if !exclude_path.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(&exclude_path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == line.trim()) {
        return Ok(());
    }
    let mut new_content = existing;
    if !new_content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    }
    new_content.push_str(line);
    new_content.push('\n');
    std::fs::write(&exclude_path, new_content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_collapses_non_alnum() {
        assert_eq!(slugify("Fix the LSP cache!"), "fix-the-lsp-cache");
        assert_eq!(slugify("  spaces   only  "), "spaces-only");
        assert_eq!(slugify("--leading"), "leading");
        assert_eq!(slugify("!!!"), "unnamed");
    }

    #[test]
    fn slugify_truncates_long_input() {
        let long = "a".repeat(200);
        assert!(slugify(&long).len() <= 40);
    }
}
