// Realises spec/tools/spec/spec-coverage.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::coverage::analyze;

pub struct SpecCoverage;

#[async_trait]
impl Tool for SpecCoverage {
    fn name(&self) -> &str {
        "spec_coverage"
    }
    fn description(&self) -> &str {
        "Report workspace Rust source files that no spec transitively reaches. Seeds the file-level \
         import graph from every spec's `code:` frontmatter and BFSes `use` / `crate::` edges; files \
         nothing reaches are 'uncovered' (code not anchored to any spec). Heuristic — misses \
         macro-generated and external-re-export-only edges."
    }
    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, _args: Value, ctx: &ToolContext) -> ToolResult {
        let repo = PathBuf::from(ctx.workspace_root.as_std_path());
        let report = analyze(&repo);
        match serde_json::to_value(&report) {
            Ok(mut v) => {
                // Surface a flat count for CLI `--strict` / Health Check parsing.
                if let Value::Object(map) = &mut v {
                    map.insert("count".into(), json!(report.uncovered.len()));
                }
                ToolResult::Ok(v)
            }
            Err(e) => ToolResult::Err(format!("failed to serialise coverage report: {e}")),
        }
    }
}
