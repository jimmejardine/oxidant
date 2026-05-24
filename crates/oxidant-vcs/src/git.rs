// Realises spec/components/vcs/git-shellout.md.
//
// Low-level git CLI wrapper. Every call is a single
// `tokio::process::Command::new("git")` invocation with structured args
// and parsed output. LC_ALL=C + --no-pager set globally; no colour
// requests; branch/refspec args validated against a strict regex before
// invocation.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct Git {
    cwd: PathBuf,
}

impl Git {
    pub fn at(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.cwd)
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .arg("--no-pager")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    }

    async fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let mut cmd = self.cmd();
        for a in args {
            cmd.arg(a);
        }
        let output = cmd.output().await.map_err(|e| GitError::Spawn {
            args: args.iter().map(|s| s.to_string()).collect(),
            message: e.to_string(),
        })?;
        if !output.status.success() {
            return Err(GitError::Failed {
                args: args.iter().map(|s| s.to_string()).collect(),
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    // ----- status -----

    pub async fn status(&self) -> Result<StatusOutput, GitError> {
        let stdout = self
            .run(&[
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=all",
            ])
            .await?;
        Ok(parse_status_v2(&stdout))
    }

    // ----- diff -----

    pub async fn diff(
        &self,
        revspec: Option<&str>,
        name_only: bool,
        path_glob: Option<&str>,
    ) -> Result<DiffOutput, GitError> {
        if name_only {
            let mut args: Vec<String> = vec!["diff".into(), "--name-status".into()];
            if let Some(r) = revspec {
                validate_revspec(r)?;
                args.push(r.into());
            }
            if let Some(p) = path_glob {
                args.push("--".into());
                args.push(p.into());
            }
            let argrefs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let stdout = self.run(&argrefs).await?;
            return Ok(parse_diff_name_status(&stdout));
        }

        // numstat for additions/deletions, then a separate unified-diff call
        // for hunks. Cheap to do both for the typical small change set.
        let mut numstat_args: Vec<String> = vec!["diff".into(), "--numstat".into()];
        if let Some(r) = revspec {
            validate_revspec(r)?;
            numstat_args.push(r.into());
        }
        if let Some(p) = path_glob {
            numstat_args.push("--".into());
            numstat_args.push(p.into());
        }
        let numstat_refs: Vec<&str> = numstat_args.iter().map(|s| s.as_str()).collect();
        let numstat_out = self.run(&numstat_refs).await?;
        let mut files = parse_diff_numstat(&numstat_out);

        let mut unified_args: Vec<String> = vec!["diff".into()];
        if let Some(r) = revspec {
            unified_args.push(r.into());
        }
        if let Some(p) = path_glob {
            unified_args.push("--".into());
            unified_args.push(p.into());
        }
        let unified_refs: Vec<&str> = unified_args.iter().map(|s| s.as_str()).collect();
        let unified_out = self.run(&unified_refs).await?;
        attach_hunks_to_files(&mut files, &unified_out);
        Ok(DiffOutput { files })
    }

    // ----- log -----

    pub async fn log(&self, opts: LogOpts) -> Result<Vec<Commit>, GitError> {
        let mut args: Vec<String> = vec![
            "log".into(),
            "--pretty=format:%H%x09%aI%x09%an%x09%s".into(),
        ];
        if let Some(limit) = opts.limit {
            args.push(format!("-{limit}"));
        }
        if let Some(rev) = &opts.revspec {
            validate_revspec(rev)?;
            args.push(rev.clone());
        }
        if let Some(path) = &opts.path {
            args.push("--".into());
            args.push(path.to_string_lossy().to_string());
        }
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let stdout = self.run(&refs).await?;
        Ok(parse_log_tabbed(&stdout))
    }

    // ----- show -----

    /// `git show <sha>:<repo-relative-path>` — return the file's contents at
    /// the given revision. Returns `FileNotAtRevision` if the path doesn't
    /// exist at that commit so callers can render the absence cleanly rather
    /// than surfacing a generic command failure.
    pub async fn show_file(&self, sha: &str, path: &Path) -> Result<String, GitError> {
        validate_revspec(sha)?;
        let path_str = path.to_string_lossy().replace('\\', "/");
        let spec = format!("{sha}:{path_str}");
        match self.run(&["show", &spec]).await {
            Ok(s) => Ok(s),
            Err(GitError::Failed { stderr, .. })
                if stderr.contains("does not exist") || stderr.contains("exists on disk") =>
            {
                Err(GitError::FileNotAtRevision {
                    sha: sha.to_string(),
                    path: path.to_path_buf(),
                })
            }
            Err(e) => Err(e),
        }
    }

    // ----- commit -----

    pub async fn commit(
        &self,
        message: &str,
        paths: &[PathBuf],
        amend: bool,
    ) -> Result<CommitOutcome, GitError> {
        if message.trim().is_empty() {
            return Err(GitError::Invalid("commit message must not be empty".into()));
        }
        // Stage explicit paths, or stage all changes if none supplied.
        if paths.is_empty() {
            self.run(&["add", "-A"]).await?;
        } else {
            let mut args: Vec<String> = vec!["add".into(), "--".into()];
            for p in paths {
                args.push(p.to_string_lossy().to_string());
            }
            let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            self.run(&refs).await?;
        }

        // amend safety: refuse if the previous commit is on @{upstream}.
        if amend {
            // git rev-parse --abbrev-ref @{upstream} — fails if no upstream.
            if self
                .run(&["rev-parse", "--abbrev-ref", "@{upstream}"])
                .await
                .is_ok()
            {
                // Check that HEAD is ahead of upstream (at least one local-only commit).
                let count = self
                    .run(&["rev-list", "--count", "@{upstream}..HEAD"])
                    .await
                    .unwrap_or_default();
                let ahead: u32 = count.trim().parse().unwrap_or(0);
                if ahead == 0 {
                    return Err(GitError::Invalid(
                        "refusing to amend: HEAD is at @{upstream} (would rewrite published history)".into(),
                    ));
                }
            }
        }

        let mut commit_args: Vec<String> = vec!["commit".into(), "-m".into(), message.into()];
        if amend {
            commit_args.push("--amend".into());
            commit_args.push("--no-edit".into());
        }
        let refs: Vec<&str> = commit_args.iter().map(|s| s.as_str()).collect();
        self.run(&refs).await?;

        let sha = self.run(&["rev-parse", "HEAD"]).await?.trim().to_string();
        // Count files in this commit via diff-tree --name-only HEAD.
        let names_out = self
            .run(&["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"])
            .await
            .unwrap_or_default();
        let files_committed = names_out.lines().filter(|l| !l.trim().is_empty()).count();
        Ok(CommitOutcome {
            sha,
            files_committed,
        })
    }

    // ----- branches -----

    pub async fn current_branch(&self) -> Result<Option<String>, GitError> {
        let out = self
            .run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
            .await;
        match out {
            Ok(s) => {
                let s = s.trim().to_string();
                Ok(if s.is_empty() { None } else { Some(s) })
            }
            Err(_) => Ok(None), // detached HEAD
        }
    }

    pub async fn branch_create(
        &self,
        name: &str,
        base: Option<&str>,
    ) -> Result<BranchCreateOutcome, GitError> {
        validate_branch_name(name)?;
        let mut args: Vec<String> = vec!["branch".into(), name.into()];
        if let Some(b) = base {
            validate_revspec(b)?;
            args.push(b.into());
        }
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run(&refs).await?;
        let based_on = self
            .run(&["rev-parse", base.unwrap_or("HEAD")])
            .await?
            .trim()
            .to_string();
        Ok(BranchCreateOutcome {
            branch: name.to_string(),
            based_on,
        })
    }

    pub async fn checkout(&self, branch: &str, create: bool) -> Result<CheckoutOutcome, GitError> {
        validate_branch_name(branch)?;
        let previous = self.current_branch().await?.unwrap_or_default();
        let mut args: Vec<String> = vec!["checkout".into()];
        if create {
            args.push("-b".into());
        }
        args.push(branch.into());
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run(&refs).await?;
        Ok(CheckoutOutcome {
            branch: branch.to_string(),
            switched_from: previous,
        })
    }

    pub async fn merge(&self, branch: &str, opts: MergeOpts) -> Result<MergeOutcome, GitError> {
        validate_branch_name(branch)?;
        let mut args: Vec<String> = vec!["merge".into()];
        if opts.no_ff {
            args.push("--no-ff".into());
        }
        if let Some(m) = &opts.message {
            args.push("-m".into());
            args.push(m.clone());
        }
        args.push(branch.into());
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let result = self.run(&refs).await;
        match result {
            Ok(_) => {
                let sha = self.run(&["rev-parse", "HEAD"]).await?.trim().to_string();
                Ok(MergeOutcome {
                    commit_sha: Some(sha),
                    conflicts: Vec::new(),
                })
            }
            Err(GitError::Failed { stderr, .. }) if stderr.contains("CONFLICT") => {
                // Read the conflicted paths from status.
                let status_out = self
                    .run(&["diff", "--name-only", "--diff-filter=U"])
                    .await?;
                let conflicts: Vec<String> = status_out
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(String::from)
                    .collect();
                Ok(MergeOutcome {
                    commit_sha: None,
                    conflicts,
                })
            }
            Err(e) => Err(e),
        }
    }

    // ----- worktree -----

    pub async fn worktree_add(&self, path: &Path, branch: &str) -> Result<(), GitError> {
        validate_branch_name(branch)?;
        self.run(&[
            "worktree",
            "add",
            "-b",
            branch,
            path.to_string_lossy().as_ref(),
        ])
        .await?;
        Ok(())
    }

    pub async fn worktree_list(&self) -> Result<Vec<Worktree>, GitError> {
        let out = self.run(&["worktree", "list", "--porcelain"]).await?;
        Ok(parse_worktree_list(&out))
    }

    pub async fn worktree_remove(&self, path: &Path, force: bool) -> Result<(), GitError> {
        let mut args: Vec<String> = vec!["worktree".into(), "remove".into()];
        if force {
            args.push("--force".into());
        }
        args.push(path.to_string_lossy().to_string());
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run(&refs).await?;
        Ok(())
    }
}

// ---------------------------------------------------------------- errors

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("spawn `git {}` failed: {message}", .args.join(" "))]
    Spawn { args: Vec<String>, message: String },
    #[error("`git {}` exited {code:?}: {stderr}", .args.join(" "))]
    Failed {
        args: Vec<String>,
        code: Option<i32>,
        stderr: String,
    },
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("path {path:?} does not exist at revision {sha}")]
    FileNotAtRevision { sha: String, path: PathBuf },
}

// ---------------------------------------------------------------- validators

static BRANCH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9._\-/]+$").expect("known-good regex"));

static REVSPEC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9._\-/@\^~{}!:]+$").expect("known-good regex"));

fn validate_branch_name(name: &str) -> Result<(), GitError> {
    if name.is_empty() {
        return Err(GitError::Invalid("branch name must not be empty".into()));
    }
    if !BRANCH_RE.is_match(name) {
        return Err(GitError::Invalid(format!(
            "branch name {name:?} contains characters outside [a-zA-Z0-9._\\-/]"
        )));
    }
    Ok(())
}

fn validate_revspec(rev: &str) -> Result<(), GitError> {
    if rev.is_empty() {
        return Err(GitError::Invalid("revspec must not be empty".into()));
    }
    if !REVSPEC_RE.is_match(rev) {
        return Err(GitError::Invalid(format!(
            "revspec {rev:?} contains disallowed characters"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------- output types

#[derive(Debug, Clone, Default, Serialize)]
pub struct StatusOutput {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<StatusFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusFile {
    pub path: String,
    pub index: String,
    pub worktree: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffOutput {
    pub files: Vec<DiffFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffFile {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffHunk {
    pub old_range: (u32, u32),
    pub new_range: (u32, u32),
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct LogOpts {
    pub revspec: Option<String>,
    pub limit: Option<usize>,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Commit {
    pub sha: String,
    pub iso_date: String,
    pub author: String,
    pub subject: String,
}

#[derive(Debug, Clone, Default)]
pub struct MergeOpts {
    pub no_ff: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergeOutcome {
    pub commit_sha: Option<String>,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_main: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitOutcome {
    pub sha: String,
    pub files_committed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchCreateOutcome {
    pub branch: String,
    pub based_on: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckoutOutcome {
    pub branch: String,
    pub switched_from: String,
}

// ---------------------------------------------------------------- parsers

fn parse_status_v2(stdout: &str) -> StatusOutput {
    let mut out = StatusOutput::default();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            out.branch = if rest == "(detached)" {
                None
            } else {
                Some(rest.to_string())
            };
        } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            out.upstream = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // "+<ahead> -<behind>"
            let mut parts = rest.split_whitespace();
            if let Some(a) = parts.next() {
                out.ahead = a.trim_start_matches('+').parse().unwrap_or(0);
            }
            if let Some(b) = parts.next() {
                out.behind = b.trim_start_matches('-').parse().unwrap_or(0);
            }
        } else if let Some(rest) = line.strip_prefix("1 ") {
            // "<XY> <sub> <mh> <mi> <mw> <hH> <hI> <path>"
            let cols: Vec<&str> = rest.splitn(8, ' ').collect();
            if cols.len() == 8 {
                let xy = cols[0];
                let path = cols[7];
                let chars: Vec<char> = xy.chars().collect();
                let index = chars.first().copied().unwrap_or('?').to_string();
                let worktree = chars.get(1).copied().unwrap_or('?').to_string();
                out.files.push(StatusFile {
                    path: path.to_string(),
                    index,
                    worktree,
                });
            }
        } else if let Some(rest) = line.strip_prefix("2 ") {
            // rename: "<XY> <sub> ... <X><score> <new_path>\t<orig_path>"
            // We just record the new path with the status chars.
            let cols: Vec<&str> = rest.splitn(10, ' ').collect();
            if cols.len() >= 10 {
                let xy = cols[0];
                let last = cols[9];
                // path may be tab-separated; take the new path (first half).
                let path = last.split('\t').next().unwrap_or(last);
                let chars: Vec<char> = xy.chars().collect();
                let index = chars.first().copied().unwrap_or('?').to_string();
                let worktree = chars.get(1).copied().unwrap_or('?').to_string();
                out.files.push(StatusFile {
                    path: path.to_string(),
                    index,
                    worktree,
                });
            }
        } else if let Some(rest) = line.strip_prefix("? ") {
            out.files.push(StatusFile {
                path: rest.to_string(),
                index: "?".into(),
                worktree: "?".into(),
            });
        } else if let Some(rest) = line.strip_prefix("! ") {
            out.files.push(StatusFile {
                path: rest.to_string(),
                index: "!".into(),
                worktree: "!".into(),
            });
        }
    }
    out
}

fn parse_diff_name_status(stdout: &str) -> DiffOutput {
    let mut files = Vec::new();
    for line in stdout.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() >= 2 {
            let status = cols[0].to_string();
            let path = cols.last().unwrap().to_string();
            files.push(DiffFile {
                path,
                status,
                additions: 0,
                deletions: 0,
                hunks: Vec::new(),
            });
        }
    }
    DiffOutput { files }
}

fn parse_diff_numstat(stdout: &str) -> Vec<DiffFile> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() >= 3 {
            let additions: u32 = cols[0].parse().unwrap_or(0);
            let deletions: u32 = cols[1].parse().unwrap_or(0);
            let path = cols[2].to_string();
            out.push(DiffFile {
                path,
                status: "modified".into(),
                additions,
                deletions,
                hunks: Vec::new(),
            });
        }
    }
    out
}

fn attach_hunks_to_files(files: &mut [DiffFile], unified: &str) {
    static HUNK_HEADER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").expect("known-good regex")
    });
    let mut current_path: Option<String> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut file_map: std::collections::HashMap<String, Vec<DiffHunk>> =
        std::collections::HashMap::new();
    for line in unified.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            // Path is "a/<path> b/<path>"; take part after "b/".
            if let Some(b_part) = rest.split(" b/").nth(1) {
                if let Some(hunk) = current_hunk.take()
                    && let Some(p) = &current_path
                {
                    file_map.entry(p.clone()).or_default().push(hunk);
                }
                current_path = Some(b_part.to_string());
            }
        } else if let Some(caps) = HUNK_HEADER.captures(line) {
            if let Some(hunk) = current_hunk.take()
                && let Some(p) = &current_path
            {
                file_map.entry(p.clone()).or_default().push(hunk);
            }
            let old_start = caps
                .get(1)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            let old_count = caps
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let new_start = caps
                .get(3)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            let new_count = caps
                .get(4)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            current_hunk = Some(DiffHunk {
                old_range: (old_start, old_count),
                new_range: (new_start, new_count),
                text: format!("{line}\n"),
            });
        } else if let Some(h) = current_hunk.as_mut() {
            h.text.push_str(line);
            h.text.push('\n');
        }
    }
    if let Some(hunk) = current_hunk.take()
        && let Some(p) = current_path
    {
        file_map.entry(p).or_default().push(hunk);
    }

    for file in files.iter_mut() {
        if let Some(hunks) = file_map.remove(&file.path) {
            file.hunks = hunks;
        }
    }
}

fn parse_log_tabbed(stdout: &str) -> Vec<Commit> {
    stdout
        .lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.splitn(4, '\t').collect();
            if cols.len() == 4 {
                Some(Commit {
                    sha: cols[0].to_string(),
                    iso_date: cols[1].to_string(),
                    author: cols[2].to_string(),
                    subject: cols[3].to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

fn parse_worktree_list(stdout: &str) -> Vec<Worktree> {
    let mut out = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;
    let mut is_main = true;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            if let Some(path) = current_path.take() {
                out.push(Worktree {
                    path,
                    branch: current_branch.take(),
                    is_main,
                });
                is_main = false;
            }
            current_path = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("branch ") {
            // "refs/heads/<name>"
            let name = rest.strip_prefix("refs/heads/").unwrap_or(rest).to_string();
            current_branch = Some(name);
        }
    }
    if let Some(path) = current_path {
        out.push(Worktree {
            path,
            branch: current_branch,
            is_main,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_branch_names() {
        assert!(validate_branch_name("feat/x").is_ok());
        assert!(validate_branch_name("oxidant/explore/lsp-42").is_ok());
        assert!(validate_branch_name("with spaces").is_err());
        assert!(validate_branch_name("$danger").is_err());
        assert!(validate_branch_name("").is_err());
    }

    #[test]
    fn parses_porcelain_v2_status() {
        let text = "\
# branch.oid abc123
# branch.head feat/x
# branch.upstream origin/feat/x
# branch.ab +2 -0
1 .M N... 100644 100644 100644 abc def src/foo.rs
1 A. N... 000000 100644 100644 ... ... src/new.rs
? src/wip.txt
";
        let s = parse_status_v2(text);
        assert_eq!(s.branch.as_deref(), Some("feat/x"));
        assert_eq!(s.upstream.as_deref(), Some("origin/feat/x"));
        assert_eq!(s.ahead, 2);
        assert_eq!(s.behind, 0);
        assert_eq!(s.files.len(), 3);
        assert_eq!(s.files[2].path, "src/wip.txt");
        assert_eq!(s.files[2].index, "?");
    }

    #[test]
    fn parses_log_tabbed_output() {
        let text = "abc\t2026-05-21T14:00:00Z\tJane\tFix the thing\nbeef\t2026-05-20T10:00:00Z\tBob\tFirst commit\n";
        let commits = parse_log_tabbed(text);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "abc");
        assert_eq!(commits[1].subject, "First commit");
    }

    #[test]
    fn parses_worktree_list() {
        let text = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\nworktree /repo/.oxidant-worktrees/sub\nHEAD def\nbranch refs/heads/oxidant/explore/x\n";
        let wts = parse_worktree_list(text);
        assert_eq!(wts.len(), 2);
        assert!(wts[0].is_main);
        assert_eq!(wts[0].branch.as_deref(), Some("main"));
        assert!(!wts[1].is_main);
        assert_eq!(wts[1].branch.as_deref(), Some("oxidant/explore/x"));
    }
}
