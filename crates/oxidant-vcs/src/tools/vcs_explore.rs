// Realises spec/tools/vcs/vcs-explore.md.
//
// **GUI-only**. The agent loop is not allowed to spawn new explorations
// (would fork-bomb conversation context). When invoked from the agent's
// registry, this tool returns an error explaining the policy. The
// register_standard_tools function in lib.rs deliberately omits this
// tool from the default registry — when the GUI ships, it'll register
// it separately.

use async_trait::async_trait;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

pub struct VcsExplore;

#[async_trait]
impl Tool for VcsExplore {
    fn name(&self) -> &str {
        "vcs_explore"
    }
    fn description(&self) -> &str {
        "Spawn a new sub-exploration with its own worktree + branch + conversation. GUI-only — the agent cannot spawn explorations from a tool call."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "base":        { "type": "string", "default": "HEAD" },
                "name":        { "type": "string" },
                "seed_prompt": { "type": "string" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mutating
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::Err(
            "vcs_explore is GUI-only: the agent cannot spawn explorations. \
             Ask the user to spawn one via the GUI; the new exploration's \
             conversation will start fresh."
                .into(),
        )
    }
}
