// Realises spec/tools/vcs/vcs-branch-create.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::git::Git;

pub struct VcsBranchCreate;

#[derive(Deserialize)]
struct Args {
    name: String,
    #[serde(default)]
    base: Option<String>,
}

#[async_trait]
impl Tool for VcsBranchCreate {
    fn name(&self) -> &str {
        "vcs_branch_create"
    }
    fn description(&self) -> &str {
        "Create a new branch in the active workspace (does not switch). Branch name validated against `[a-zA-Z0-9._\\-/]+`. To spawn a new exploration with its own worktree, the user uses the GUI flow."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string", "minLength": 1 },
                "base": { "type": "string", "default": "HEAD" }
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
        match git.branch_create(&args.name, args.base.as_deref()).await {
            Ok(o) => ToolResult::Ok(json!({
                "branch":   o.branch,
                "based_on": o.based_on,
            })),
            Err(e) => ToolResult::Err(e.to_string()),
        }
    }
}
