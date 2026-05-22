// Realises spec/tools/spec/spec-for-file.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::walker::walk_specs;

pub struct SpecForFile;

#[derive(Deserialize)]
struct Args {
    path: String,
}

#[async_trait]
impl Tool for SpecForFile {
    fn name(&self) -> &str {
        "spec_for_file"
    }
    fn description(&self) -> &str {
        "Given a code file path (relative to workspace root), return the specs that declare it in their `code:` frontmatter. Use this before editing source to ground the change in the relevant component/tool spec."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "code file path relative to workspace root"
                }
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
        let needle = normalise_path(&args.path);
        let repo = PathBuf::from(ctx.workspace_root.as_std_path());
        let records = walk_specs(&repo);

        let specs: Vec<Value> = records
            .iter()
            .filter(|r| {
                r.file.frontmatter.code.iter().any(|p| {
                    normalise_path(&p.to_string_lossy()) == needle
                })
            })
            .map(|r| {
                json!({
                    "ref":            r.canonical_id,
                    "kind":           r.file.frontmatter.kind.as_str(),
                    "responsibility": r.file.frontmatter.responsibility,
                })
            })
            .collect();

        ToolResult::Ok(json!({
            "path":  args.path,
            "specs": specs,
            "count": specs.len(),
        }))
    }
}

fn normalise_path(p: &str) -> String {
    p.replace('\\', "/")
}
