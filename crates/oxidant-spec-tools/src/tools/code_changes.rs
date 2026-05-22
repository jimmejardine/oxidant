// Realises spec/tools/timeline/code-changes.md.

use std::path::PathBuf;
use std::time::Instant;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::timeline::{Timeline, TimelineFilter};

pub struct CodeChanges;

#[derive(Deserialize, Default)]
struct Args {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    lang: Option<String>,
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
impl Tool for CodeChanges {
    fn name(&self) -> &str {
        "code_changes"
    }
    fn description(&self) -> &str {
        "Return chronological change history for code files. Backed by `git log`. Filter by path (file or directory), lang (e.g. rs, toml), since/until/author."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path":   { "type": "string", "description": "file or directory path under worktree" },
                "lang":   { "type": "string", "description": "file extension family (rs, toml, md, ...)" },
                "since":  { "type": "string" },
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

        // pathspec selection: explicit path → that path; lang → *.{ext}
        // pathspec; both → both; neither → "crates/" (the default code
        // location for this workspace).
        let mut pathspecs: Vec<String> = Vec::new();
        if let Some(p) = &args.path {
            pathspecs.push(p.clone());
        }
        if let Some(lang) = &args.lang {
            pathspecs.push(format!(":(glob)**/*.{lang}"));
        }
        if pathspecs.is_empty() {
            pathspecs.push("crates/".to_string());
        }

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
