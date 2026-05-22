// Realises spec/tools/spec/spec-diff.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::diff::{diff_all, diff_spec};

pub struct SpecDiff;

#[derive(Deserialize)]
struct Args {
    #[serde(default, rename = "ref")]
    ref_id: Option<String>,
}

#[async_trait]
impl Tool for SpecDiff {
    fn name(&self) -> &str {
        "spec_diff"
    }
    fn description(&self) -> &str {
        "Detect drift between specs and code. MVP checks: (1) trait-method drift for `kind: contract` specs and (2) missing `code:` paths for components/tools. Optional `ref` field scopes to a single spec id like `contracts/tool`."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "limit to this spec id (e.g. contracts/tool); omit for tree-wide"
                }
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
        let repo = PathBuf::from(ctx.workspace_root.as_std_path());
        let drifts = match args.ref_id {
            Some(id) => diff_spec(&repo, &id),
            None => diff_all(&repo),
        };
        ToolResult::Ok(json!({
            "count": drifts.len(),
            "drifts": drifts,
        }))
    }
}
