// Realises spec/tools/spec/spec-resolve-links.md.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::graph::{GraphInput, Resolution, SpecGraph, resolve};
use crate::walker::walk_specs;

pub struct SpecResolveLinks;

#[derive(Deserialize)]
struct Args {
    #[serde(rename = "ref")]
    ref_id: String,
}

#[async_trait]
impl Tool for SpecResolveLinks {
    fn name(&self) -> &str {
        "spec_resolve_links"
    }
    fn description(&self) -> &str {
        "Return all inbound and outbound edges (parent / implements / depends_on / body_ref) for one spec. Use for impact analysis (`who points at this?`) and orientation (`what does this point at?`)."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["ref"],
            "properties": {
                "ref": { "type": "string" }
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
        let repo = PathBuf::from(ctx.workspace_root.as_std_path());
        let records = walk_specs(&repo);
        let all_ids: Vec<String> = records.iter().map(|r| r.canonical_id.clone()).collect();

        let canonical = match resolve(&args.ref_id, &all_ids) {
            Resolution::Resolved(id) => id,
            Resolution::Unresolved => {
                return ToolResult::Err(format!("no spec found for ref {:?}", args.ref_id));
            }
            Resolution::Ambiguous(cs) => {
                return ToolResult::Err(format!(
                    "ambiguous ref {:?}: matches {}: {}",
                    args.ref_id,
                    cs.len(),
                    cs.join(", ")
                ));
            }
        };

        let inputs: Vec<GraphInput> = records
            .iter()
            .map(|r| GraphInput {
                canonical_id: r.canonical_id.clone(),
                file: r.file.clone(),
                path: r.path.clone(),
            })
            .collect();
        let graph = SpecGraph::build(&inputs);

        let mut inbound: Vec<Value> = graph
            .inbound(&canonical)
            .into_iter()
            .map(|(n, ek)| json!({ "from": n.id, "edge": ek.as_str() }))
            .collect();
        inbound.sort_by(|a, b| {
            let order = a["from"].as_str().cmp(&b["from"].as_str());
            if order == std::cmp::Ordering::Equal {
                a["edge"].as_str().cmp(&b["edge"].as_str())
            } else {
                order
            }
        });

        let mut outbound: Vec<Value> = graph
            .outbound(&canonical)
            .into_iter()
            .map(|(n, ek)| json!({ "to": n.id, "edge": ek.as_str() }))
            .collect();
        outbound.sort_by(|a, b| {
            let order = a["to"].as_str().cmp(&b["to"].as_str());
            if order == std::cmp::Ordering::Equal {
                a["edge"].as_str().cmp(&b["edge"].as_str())
            } else {
                order
            }
        });

        ToolResult::Ok(json!({
            "ref":      canonical,
            "inbound":  inbound,
            "outbound": outbound,
        }))
    }
}
