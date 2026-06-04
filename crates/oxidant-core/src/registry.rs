// Realises spec/contracts/tool.md and spec/components/core/tool-registry.md.
//
// The Tool trait, its supporting types, and the dispatching ToolRegistry all
// live here; the contract spec defines the trait surface and the registry
// component spec defines the dispatch flow.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use async_trait::async_trait;
use camino::Utf8PathBuf;
use futures::FutureExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str {
        ""
    }
    fn schema(&self) -> Value;
    fn category(&self) -> ToolCategory;
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCategory {
    ReadOnly,
    Mutating,
    Network,
}

#[derive(Debug)]
pub enum ToolResult {
    Ok(Value),
    Err(String),
}

/// Cheap to clone: workspace_root and exploration_id are owned strings;
/// CancellationToken is Arc-backed. The agent_loop clones this once per
/// spawned tool task so each tokio task has its own `'static` copy.
#[derive(Clone)]
pub struct ToolContext {
    pub workspace_root: Utf8PathBuf,
    pub exploration_id: String,
    pub cancellation: CancellationToken,
    /// Hook for tools that need to defer to the user — e.g. `ask_user`
    /// posing a multiple-choice question. `None` in headless / CLI
    /// contexts; `Some` when an interactive host (the GUI) is driving
    /// the loop. Tools requiring user input fail with
    /// `ToolResult::Err` when this is `None`. See
    /// spec/contracts/tool.md "UiBridge".
    pub ui: Option<Arc<dyn UiBridge>>,
    // Permission state lands when spec/components/config/permissions.md is
    // realised; registry dispatch will gate on it then.
}

impl ToolContext {
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

/// Interactive UI hook for tools that genuinely need the user to make a
/// choice (only `ask_user` today). Implementations live in the host —
/// the GUI provides `GuiBridge`; CLI / test contexts pass `None`. See
/// spec/contracts/tool.md "UiBridge" and spec/tools/ask-user.md.
#[async_trait]
pub trait UiBridge: Send + Sync {
    /// Pose a multiple-choice question to the user and return their
    /// answer. `allow_freeform = true` adds a free-form text field as
    /// an implicit N+1th option. The returned string is either one of
    /// `options` verbatim or the user's typed text. Errors when the
    /// user cancels the question or the bridge is torn down before a
    /// response arrives.
    async fn ask_user(
        &self,
        question: String,
        options: Vec<String>,
        allow_freeform: bool,
    ) -> anyhow::Result<String>;
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn schemas(&self) -> Vec<(String, Value)> {
        self.tools
            .iter()
            .map(|(name, tool)| (name.clone(), tool.schema()))
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Tool>> + '_ {
        self.tools.values()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub async fn invoke(&self, name: &str, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(tool) = self.tools.get(name).cloned() else {
            return ToolResult::Err(format!("unknown tool: {name}"));
        };

        // Steps 2 (jsonschema validation) and 3 (permission gating) from
        // spec/components/core/tool-registry.md land alongside the first
        // concrete Tool impls and the permission types.

        match AssertUnwindSafe(tool.invoke(args, ctx))
            .catch_unwind()
            .await
        {
            Ok(result) => result,
            Err(_) => ToolResult::Err(format!("tool '{name}' panicked")),
        }
    }
}
