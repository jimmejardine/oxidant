// Realises spec/tools/vcs/vcs-discard.md.
//
// **GUI-only**. Destructive cross-exploration operations are
// user-initiated. See vcs_explore.rs for the policy.

use async_trait::async_trait;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

pub struct VcsDiscard;

#[async_trait]
impl Tool for VcsDiscard {
    fn name(&self) -> &str {
        "vcs_discard"
    }
    fn description(&self) -> &str {
        "Remove a sub-exploration's worktree and archive its transcript. GUI-only — destructive, user-initiated."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["exploration_id"],
            "properties": {
                "exploration_id": { "type": "string" },
                "force":          { "type": "boolean", "default": false },
                "archive":        { "type": "boolean", "default": true }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mutating
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::Err(
            "vcs_discard is GUI-only: discarding an exploration is destructive \
             and must be user-initiated."
                .into(),
        )
    }
}
