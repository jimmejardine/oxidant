// Live VCS integration tests — they spawn real `git` against a tempdir
// repo. Reasonable to run by default (~100ms per test); no #[ignore].
// `git` must be on PATH (it is wherever this workspace builds at all).

use std::path::Path;
use std::process::Command as SyncCommand;

use camino::Utf8PathBuf;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use oxidant_core::{Tool, ToolContext, ToolResult};
use oxidant_vcs::{
    Git, GitError, LogOpts, VcsBranchCreate, VcsCommit, VcsDiff, VcsExplore, VcsLog, VcsStatus,
    worktree,
};

fn run_git(repo: &Path, args: &[&str]) {
    let status = SyncCommand::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("spawn git");
    assert!(
        status.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

fn ctx_for(dir: &Path) -> ToolContext {
    ToolContext {
        workspace_root: Utf8PathBuf::from_path_buf(dunce::canonicalize(dir).unwrap()).unwrap(),
        exploration_id: "vcs-test".into(),
        cancellation: CancellationToken::new(),
    }
}

fn make_repo(commits: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "main"]);
    run_git(dir.path(), &["config", "user.email", "t@example.com"]);
    run_git(dir.path(), &["config", "user.name", "test"]);
    run_git(dir.path(), &["config", "commit.gpgsign", "false"]);
    for (path, content) in commits {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
        run_git(dir.path(), &["add", path]);
        run_git(
            dir.path(),
            &["commit", "-m", &format!("add {path}"), "--quiet"],
        );
    }
    dir
}

#[tokio::test]
async fn vcs_status_clean_repo_reports_no_files() {
    let dir = make_repo(&[("a.txt", "hello\n")]);
    let v = match VcsStatus.invoke(json!({}), &ctx_for(dir.path())).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["branch"], "main");
    assert_eq!(v["files"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn vcs_status_detects_dirty_files() {
    let dir = make_repo(&[("a.txt", "hello\n")]);
    std::fs::write(dir.path().join("a.txt"), "hello world\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "fresh\n").unwrap();
    let v = match VcsStatus.invoke(json!({}), &ctx_for(dir.path())).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .any(|f| f["path"] == "new.txt" && f["worktree"] == "?")
    );
}

#[tokio::test]
async fn vcs_log_lists_commits() {
    let dir = make_repo(&[("a.txt", "1"), ("b.txt", "2")]);
    let v = match VcsLog.invoke(json!({}), &ctx_for(dir.path())).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    let commits = v["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 2);
    assert!(
        commits[0]["subject"]
            .as_str()
            .unwrap()
            .contains("add b.txt")
    );
    assert!(
        commits[1]["subject"]
            .as_str()
            .unwrap()
            .contains("add a.txt")
    );
}

#[tokio::test]
async fn vcs_diff_with_modifications() {
    let dir = make_repo(&[("a.txt", "line 1\nline 2\n")]);
    std::fs::write(
        dir.path().join("a.txt"),
        "line 1\nline 2 MODIFIED\nline 3\n",
    )
    .unwrap();
    let v = match VcsDiff
        .invoke(json!({ "name_only": true }), &ctx_for(dir.path()))
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "a.txt");
}

#[tokio::test]
async fn vcs_commit_stages_and_commits() {
    let dir = make_repo(&[("a.txt", "1\n")]);
    std::fs::write(dir.path().join("a.txt"), "1\n2\n").unwrap();
    let v = match VcsCommit
        .invoke(
            json!({ "message": "Update a.txt with second line" }),
            &ctx_for(dir.path()),
        )
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["branch"], "main");
    assert!(v["sha"].as_str().unwrap().len() >= 7);
    // Repo is clean again.
    let status = match VcsStatus.invoke(json!({}), &ctx_for(dir.path())).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(status["files"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn vcs_branch_create_does_not_switch() {
    let dir = make_repo(&[("a.txt", "1\n")]);
    let v = match VcsBranchCreate
        .invoke(json!({ "name": "feat/new" }), &ctx_for(dir.path()))
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["branch"], "feat/new");
    let status = match VcsStatus.invoke(json!({}), &ctx_for(dir.path())).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(status["branch"], "main"); // branch_create didn't switch
}

#[tokio::test]
async fn vcs_explore_is_gui_only_and_refuses() {
    let dir = make_repo(&[("a.txt", "1\n")]);
    let result = VcsExplore
        .invoke(json!({ "name": "test" }), &ctx_for(dir.path()))
        .await;
    let err = match result {
        ToolResult::Err(e) => e,
        _ => panic!("expected error"),
    };
    assert!(err.to_lowercase().contains("gui-only"));
}

#[tokio::test]
async fn worktree_spawn_creates_dir_and_branch() {
    let dir = make_repo(&[("a.txt", "1\n")]);
    let handle = worktree::spawn(
        dir.path(),
        worktree::SpawnOpts {
            name: Some("test exploration".into()),
            base: None,
            branch_prefix: None,
        },
    )
    .await
    .unwrap();
    assert!(
        handle
            .branch
            .starts_with("oxidant/explore/test-exploration-")
    );
    assert!(handle.path.exists());
    assert!(handle.path.join(".oxidant").exists());
}

#[tokio::test]
async fn worktree_list_includes_main_and_sub() {
    let dir = make_repo(&[("a.txt", "1\n")]);
    let _sub = worktree::spawn(dir.path(), worktree::SpawnOpts::default())
        .await
        .unwrap();
    let entries = worktree::list(dir.path()).await.unwrap();
    assert!(entries.len() >= 2);
    assert!(entries.iter().any(|w| w.is_main));
    assert!(entries.iter().any(|w| !w.is_main));
}

#[tokio::test]
async fn show_file_returns_contents_at_revision() {
    let dir = make_repo(&[("a.txt", "first\n"), ("a.txt", "first\nsecond\n")]);
    let git = Git::at(dunce::canonicalize(dir.path()).unwrap());
    let commits = git
        .log(LogOpts {
            limit: Some(10),
            path: Some(Path::new("a.txt").to_path_buf()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(commits.len(), 2, "two commits should touch a.txt");
    // Commits are newest-first; the older commit holds "first\n".
    let older = git
        .show_file(&commits[1].sha, Path::new("a.txt"))
        .await
        .unwrap();
    assert_eq!(older, "first\n");
    let newer = git
        .show_file(&commits[0].sha, Path::new("a.txt"))
        .await
        .unwrap();
    assert_eq!(newer, "first\nsecond\n");
}

#[tokio::test]
async fn show_file_reports_file_not_at_revision() {
    let dir = make_repo(&[("a.txt", "only a\n"), ("b.txt", "only b\n")]);
    let git = Git::at(dunce::canonicalize(dir.path()).unwrap());
    let commits = git
        .log(LogOpts {
            limit: Some(10),
            ..Default::default()
        })
        .await
        .unwrap();
    // commits[1] is the older commit (only a.txt existed then). b.txt didn't.
    let err = git
        .show_file(&commits[1].sha, Path::new("b.txt"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, GitError::FileNotAtRevision { ref path, .. } if path == Path::new("b.txt")),
        "expected FileNotAtRevision, got {err:?}"
    );
}
