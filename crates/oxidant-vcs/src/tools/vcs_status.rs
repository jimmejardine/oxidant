// Realises spec/tools/vcs/vcs-status.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::git::Git;

pub struct VcsStatus;

#[async_trait]
impl Tool for VcsStatus {
    fn name(&self) -> &str {
        "vcs_status"
    }
    fn description(&self) -> &str {
        "Return current branch, dirty files (with porcelain v2 status codes), and ahead/behind counts for the active workspace."
    }
    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, _args: Value, ctx: &ToolContext) -> ToolResult {
        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let git = Git::at(workspace);
        match git.status().await {
            Ok(s) => match serde_json::to_value(&s) {
                Ok(v) => ToolResult::Ok(v),
                Err(e) => ToolResult::Err(format!("serialise status: {e}")),
            },
            Err(e) => ToolResult::Err(e.to_string()),
        }
    }
}
