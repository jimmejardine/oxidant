// Realises spec/tools/vcs/vcs-diff.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::git::Git;

pub struct VcsDiff;

#[derive(Deserialize, Default)]
struct Args {
    #[serde(default)]
    revspec: Option<String>,
    #[serde(default)]
    name_only: Option<bool>,
    #[serde(default)]
    path_glob: Option<String>,
}

#[async_trait]
impl Tool for VcsDiff {
    fn name(&self) -> &str {
        "vcs_diff"
    }
    fn description(&self) -> &str {
        "Structured diff between the working tree and HEAD (or another revspec). With name_only=true returns paths + statuses only; otherwise additions/deletions + unified hunks."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "revspec":   { "type": "string", "description": "default: working tree vs HEAD" },
                "name_only": { "type": "boolean", "default": false },
                "path_glob": { "type": "string" }
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
        match git
            .diff(
                args.revspec.as_deref(),
                args.name_only.unwrap_or(false),
                args.path_glob.as_deref(),
            )
            .await
        {
            Ok(d) => match serde_json::to_value(&d) {
                Ok(v) => ToolResult::Ok(v),
                Err(e) => ToolResult::Err(format!("serialise diff: {e}")),
            },
            Err(e) => ToolResult::Err(e.to_string()),
        }
    }
}
