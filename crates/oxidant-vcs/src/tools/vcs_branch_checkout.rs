// Realises spec/tools/vcs/vcs-branch-checkout.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::git::Git;

pub struct VcsBranchCheckout;

#[derive(Deserialize)]
struct Args {
    branch: String,
    #[serde(default)]
    create: Option<bool>,
}

#[async_trait]
impl Tool for VcsBranchCheckout {
    fn name(&self) -> &str {
        "vcs_branch_checkout"
    }
    fn description(&self) -> &str {
        "Switch the active workspace to a different branch. Refuses if the working tree is dirty (commit or stash first). With create=true, creates the branch from HEAD if absent."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["branch"],
            "properties": {
                "branch": { "type": "string" },
                "create": { "type": "boolean", "default": false }
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

        // Pre-flight: refuse on dirty worktree (settings-driven override
        // deferred until oxidant-config lands).
        let status = match git.status().await {
            Ok(s) => s,
            Err(e) => return ToolResult::Err(e.to_string()),
        };
        if !status.files.is_empty() {
            return ToolResult::Err(format!(
                "working tree is dirty ({} files); commit or stash before checkout",
                status.files.len()
            ));
        }

        match git.checkout(&args.branch, args.create.unwrap_or(false)).await {
            Ok(o) => ToolResult::Ok(json!({
                "branch":        o.branch,
                "switched_from": o.switched_from,
            })),
            Err(e) => ToolResult::Err(e.to_string()),
        }
    }
}
