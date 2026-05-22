// Realises spec/tools/vcs/vcs-explorations-list.md.
//
// MVP scope: derives the explorations list from git worktree list + session
// metadata. lsp_running and target_size_mb are reported as best-effort
// (target_size_mb computed from `target/` dir size; lsp_running is always
// false here because LspClient lives in oxidant-rust-tools, which this
// crate doesn't depend on — a future GUI-layer assembly point joins those
// signals together).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::session_persist::{SessionSummary, list_sessions};
use crate::worktree;

pub struct VcsExplorationsList;

#[async_trait]
impl Tool for VcsExplorationsList {
    fn name(&self) -> &str {
        "vcs_explorations_list"
    }
    fn description(&self) -> &str {
        "List all explorations (main + sub) with their worktree paths, branches, and resource usage. Derived from `git worktree list` plus per-worktree .oxidant/sessions/."
    }
    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, _args: Value, ctx: &ToolContext) -> ToolResult {
        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let summaries = match worktree::list(&workspace).await {
            Ok(s) => s,
            Err(e) => return ToolResult::Err(e.to_string()),
        };
        let active_canonical = dunce::canonicalize(&workspace).ok();

        let explorations: Vec<Value> = summaries
            .into_iter()
            .map(|w| {
                let target_size_mb = dir_size_mb(&w.path.join("target"));
                let sessions = list_sessions(&w.path);
                let primary_session = sessions.first();
                let active = active_canonical
                    .as_ref()
                    .map(|p| {
                        dunce::canonicalize(&w.path)
                            .map(|c| &c == p)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                json!({
                    "id":             primary_session.map(|s| s.id.clone()).unwrap_or_default(),
                    "kind":           if w.is_main { "main" } else { "sub" },
                    "worktree":       w.path.to_string_lossy().replace('\\', "/"),
                    "branch":         w.branch.unwrap_or_default(),
                    "active":         active,
                    "lsp_running":    false, // joined elsewhere; see module comment
                    "target_size_mb": target_size_mb,
                    "sessions":       sessions
                        .iter()
                        .map(session_to_json)
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        ToolResult::Ok(json!({
            "explorations": explorations,
            "count":        explorations.len(),
        }))
    }
}

fn session_to_json(s: &SessionSummary) -> Value {
    json!({
        "id":            s.id,
        "branch":        s.branch,
        "last_seen":     s.last_seen,
        "message_count": s.message_count,
    })
}

fn dir_size_mb(dir: &Path) -> u64 {
    if !dir.exists() {
        return 0;
    }
    let mut total: u64 = 0;
    for entry in walkdir::WalkDir::new(dir).follow_links(false) {
        let Ok(e) = entry else { continue };
        if e.file_type().is_file() {
            if let Ok(md) = e.metadata() {
                total += md.len();
            }
        }
    }
    total / 1_048_576
}
