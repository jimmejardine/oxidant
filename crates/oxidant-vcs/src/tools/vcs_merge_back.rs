// Realises spec/tools/vcs/vcs-merge-back.md.
//
// **GUI-only**. Cross-exploration merge is user-initiated; the GUI
// owns the conflict-resolution UX.

use async_trait::async_trait;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

pub struct VcsMergeBack;

#[async_trait]
impl Tool for VcsMergeBack {
    fn name(&self) -> &str {
        "vcs_merge_back"
    }
    fn description(&self) -> &str {
        "Merge a sub-exploration's branch back into its parent branch. GUI-only — conflict resolution is user-initiated."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["sub_exploration_id"],
            "properties": {
                "sub_exploration_id": { "type": "string" },
                "target_branch":      { "type": "string", "default": "main" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mutating
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::Err(
            "vcs_merge_back is GUI-only: cross-exploration merges and the \
             conflict-resolution UX are user-initiated."
                .into(),
        )
    }
}
