// Realises spec/tools/search/text-search.md.

use std::path::PathBuf;
use std::time::Instant;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::search_index::{SearchIndex, SearchQuery, SearchSource};

pub struct TextSearch;

#[derive(Deserialize)]
struct Args {
    query: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for TextSearch {
    fn name(&self) -> &str {
        "text_search"
    }
    fn description(&self) -> &str {
        "Full-text BM25 search across both spec markdown and Rust source. Optional source/kind/lang/limit filters. For exact symbol lookup in code, prefer rust_workspace_symbols; for structured spec metadata queries (kind/status/parent), prefer spec_search."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query":  { "type": "string", "minLength": 1 },
                "source": { "type": "string", "enum": ["spec", "code", "both"], "default": "both" },
                "kind":   { "type": "string", "description": "spec kind filter" },
                "lang":   { "type": "string", "description": "code language filter (e.g. rs)" },
                "limit":  { "type": "integer", "default": 20, "minimum": 1, "maximum": 100 }
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
        let source = match args.source.as_deref() {
            Some("spec") => SearchSource::Spec,
            Some("code") => SearchSource::Code,
            None | Some("both") => SearchSource::Both,
            Some(other) => {
                return ToolResult::Err(format!(
                    "source must be spec | code | both; got {other:?}"
                ));
            }
        };
        let limit = args.limit.unwrap_or(20).clamp(1, 100);
        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let index = match SearchIndex::for_workspace(&workspace) {
            Ok(i) => i,
            Err(e) => return ToolResult::Err(e),
        };

        let started = Instant::now();
        let hits = match index.search(&SearchQuery {
            text: args.query,
            source,
            kind: args.kind,
            lang: args.lang,
            limit,
        }) {
            Ok(h) => h,
            Err(e) => return ToolResult::Err(e),
        };
        let elapsed_ms = started.elapsed().as_millis();
        let truncated = hits.len() == limit;

        ToolResult::Ok(json!({
            "hits":       hits,
            "count":      hits.len(),
            "elapsed_ms": elapsed_ms,
            "truncated":  truncated,
        }))
    }
}
