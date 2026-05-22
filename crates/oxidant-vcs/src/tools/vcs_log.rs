// Realises spec/tools/vcs/vcs-log.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::git::{Git, LogOpts};

pub struct VcsLog;

#[derive(Deserialize, Default)]
struct Args {
    #[serde(default)]
    revspec: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    path: Option<String>,
}

#[async_trait]
impl Tool for VcsLog {
    fn name(&self) -> &str {
        "vcs_log"
    }
    fn description(&self) -> &str {
        "Return recent commits for the active workspace's branch (or a revspec) as structured records. For full history with co-change analysis, prefer spec_changes / code_changes."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "revspec": { "type": "string", "default": "HEAD" },
                "limit":   { "type": "integer", "default": 20, "maximum": 500 },
                "path":    { "type": "string" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: Args = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let git = Git::at(workspace);
        let opts = LogOpts {
            revspec: args.revspec,
            limit: Some(args.limit.unwrap_or(20).min(500)),
            path: args.path.map(PathBuf::from),
        };
        match git.log(opts).await {
            Ok(commits) => ToolResult::Ok(json!({
                "commits": commits,
                "count":   commits.len(),
            })),
            Err(e) => ToolResult::Err(e.to_string()),
        }
    }
}
