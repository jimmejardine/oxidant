// Realises spec/tools/timeline/spec-changes.md.

use std::path::PathBuf;
use std::time::Instant;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::index_db::{IndexDb, SpecFilter};
use crate::timeline::{Timeline, TimelineFilter};
use crate::walker::walk_specs;

pub struct SpecChanges;

#[derive(Deserialize, Default)]
struct Args {
    #[serde(default, rename = "ref")]
    ref_id: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    until: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for SpecChanges {
    fn name(&self) -> &str {
        "spec_changes"
    }
    fn description(&self) -> &str {
        "Return chronological change history for spec files. Backed by `git log` over spec/**/*.md, optionally narrowed to a single spec (`ref`) or filtered by kind/status (via the spec index) plus the usual since/until/author."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ref":    { "type": "string", "description": "canonical spec ref; omit for tree-wide" },
                "kind":   { "type": "string" },
                "status": { "type": "string", "enum": ["draft", "active", "deprecated"] },
                "since":  { "type": "string", "description": "ISO-8601 or relative ('7 days ago')" },
                "until":  { "type": "string" },
                "author": { "type": "string" },
                "limit":  { "type": "integer", "default": 50, "maximum": 500 }
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

        // Resolve pathspec: explicit ref → that file; kind/status filter →
        // matching specs' paths via the index; otherwise the whole spec/ tree.
        let pathspecs: Vec<String> = if let Some(r) = &args.ref_id {
            // Walk to find the file path for the ref (so this works without
            // requiring index population to have happened).
            let records = walk_specs(&workspace);
            let matched = records.iter().find(|rec| rec.canonical_id == *r);
            match matched {
                Some(rec) => vec![rec
                    .path
                    .strip_prefix(&workspace)
                    .unwrap_or(&rec.path)
                    .to_string_lossy()
                    .replace('\\', "/")],
                None => return ToolResult::Err(format!("no spec found for ref {r:?}")),
            }
        } else if args.kind.is_some() || args.status.is_some() {
            let db = match IndexDb::for_workspace(&workspace) {
                Ok(d) => d,
                Err(e) => return ToolResult::Err(e),
            };
            let rows = match db.lock().unwrap().query(&SpecFilter {
                kind: args.kind.clone(),
                status: args.status.clone(),
                ..Default::default()
            }) {
                Ok(r) => r,
                Err(e) => return ToolResult::Err(e),
            };
            if rows.is_empty() {
                return ToolResult::Ok(json!({ "commits": [], "elapsed_ms": 0 }));
            }
            rows.into_iter().map(|r| r.path).collect()
        } else {
            vec!["spec/".to_string()]
        };

        let started = Instant::now();
        let filter = TimelineFilter {
            pathspecs,
            since: args.since,
            until: args.until,
            author: args.author,
            limit: args.limit.map(|n| n.min(500)),
        };
        let commits = match Timeline::query(&workspace, &filter).await {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(e),
        };
        ToolResult::Ok(json!({
            "commits":    commits,
            "count":      commits.len(),
            "elapsed_ms": started.elapsed().as_millis(),
        }))
    }
}
