// Realises spec/tools/vcs/vcs-commit.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::git::Git;

pub struct VcsCommit;

#[derive(Deserialize)]
struct Args {
    message: String,
    #[serde(default)]
    paths: Option<Vec<String>>,
    #[serde(default)]
    amend: Option<bool>,
}

#[async_trait]
impl Tool for VcsCommit {
    fn name(&self) -> &str {
        "vcs_commit"
    }
    fn description(&self) -> &str {
        "Stage paths (default: all changes) and create a commit. Runs pre-commit hooks; never passes --no-verify. amend=true is allowed only if the previous commit is not yet on @{upstream}. Never pushes."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["message"],
            "properties": {
                "message": { "type": "string", "minLength": 1 },
                "paths":   { "type": "array", "items": { "type": "string" } },
                "amend":   { "type": "boolean", "default": false }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mutating
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: Args = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let git = Git::at(workspace);
        let paths: Vec<PathBuf> = args
            .paths
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let outcome = match git
            .commit(&args.message, &paths, args.amend.unwrap_or(false))
            .await
        {
            Ok(o) => o,
            Err(e) => return ToolResult::Err(e.to_string()),
        };
        let branch = git.current_branch().await.ok().flatten().unwrap_or_default();
        ToolResult::Ok(json!({
            "sha":             outcome.sha,
            "branch":          branch,
            "files_committed": outcome.files_committed,
        }))
    }
}
