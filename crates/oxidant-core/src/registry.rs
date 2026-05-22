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

pub struct ToolContext {
    pub workspace_root: Utf8PathBuf,
    pub exploration_id: String,
    pub cancellation: CancellationToken,
    // Permission state lands when spec/components/config/permissions.md is
    // realised; registry dispatch will gate on it then.
}

impl ToolContext {
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
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

    pub async fn invoke(&self, name: &str, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(tool) = self.tools.get(name).cloned() else {
            return ToolResult::Err(format!("unknown tool: {name}"));
        };

        // Steps 2 (jsonschema validation) and 3 (permission gating) from
        // spec/components/core/tool-registry.md land alongside the first
        // concrete Tool impls and the permission types.

        match AssertUnwindSafe(tool.invoke(args, ctx)).catch_unwind().await {
            Ok(result) => result,
            Err(_) => ToolResult::Err(format!("tool '{name}' panicked")),
        }
    }
}
