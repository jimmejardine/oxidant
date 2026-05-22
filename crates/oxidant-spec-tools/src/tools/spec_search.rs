// Realises spec/tools/search/spec-search.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::index_db::{IndexDb, SpecFilter};

pub struct SpecSearch;

#[derive(Deserialize, Default)]
struct Args {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    implements: Option<String>,
    #[serde(default)]
    depends_on: Option<String>,
    #[serde(default)]
    depended_by: Option<String>,
    #[serde(default)]
    orphans: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for SpecSearch {
    fn name(&self) -> &str {
        "spec_search"
    }
    fn description(&self) -> &str {
        "Structured query over spec metadata (kind, status, parent, edges) without free-text matching. Backed by the SQLite spec index. For full-text search, use text_search."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "kind":        { "type": "string" },
                "status":      { "type": "string", "enum": ["draft", "active", "deprecated"] },
                "parent":      { "type": "string" },
                "implements":  { "type": "string" },
                "depends_on":  { "type": "string" },
                "depended_by": { "type": "string" },
                "orphans":     { "type": "boolean" },
                "limit":       { "type": "integer", "default": 50, "maximum": 500 }
            },
            "additionalProperties": false
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
        let db = match IndexDb::for_workspace(&workspace) {
            Ok(d) => d,
            Err(e) => return ToolResult::Err(e),
        };
        let filter = SpecFilter {
            kind: args.kind,
            status: args.status,
            parent: args.parent,
            implements: args.implements,
            depends_on: args.depends_on,
            depended_by: args.depended_by,
            orphans_only: args.orphans.unwrap_or(false),
            limit: args.limit.map(|n| n.min(500)),
        };
        let rows = match db.lock().unwrap().query(&filter) {
            Ok(r) => r,
            Err(e) => return ToolResult::Err(e),
        };
        ToolResult::Ok(json!({
            "rows": rows,
            "count": rows.len(),
        }))
    }
}
