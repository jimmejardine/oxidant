// Realises spec/tools/spec/spec-tree.md.

use std::collections::HashSet;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::graph::{EdgeKind, GraphInput, Resolution, SpecGraph, resolve};
use crate::walker::walk_specs;

pub struct SpecTree;

#[derive(Deserialize)]
struct Args {
    #[serde(default = "default_from_ref")]
    from_ref: String,
    #[serde(default = "default_depth")]
    depth: usize,
    #[serde(default = "default_edge_kind")]
    edge_kind: String,
}

fn default_from_ref() -> String {
    "overview".into()
}
fn default_depth() -> usize {
    4
}
fn default_edge_kind() -> String {
    "parent".into()
}

#[async_trait]
impl Tool for SpecTree {
    fn name(&self) -> &str {
        "spec_tree"
    }
    fn description(&self) -> &str {
        "Return a hierarchical tree view of the spec graph rooted at a ref, walking either parent or depends_on edges. Edge `parent` gives the document hierarchy; edge `depends_on` gives the impact tree (`what hangs off X`)."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "from_ref":  { "type": "string", "default": "overview" },
                "depth":     { "type": "integer", "default": 4, "maximum": 12 },
                "edge_kind": { "type": "string", "enum": ["parent", "depends_on"], "default": "parent" }
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
        let depth = args.depth.min(12);
        let edge_kind = match args.edge_kind.as_str() {
            "parent" => EdgeKind::Parent,
            "depends_on" => EdgeKind::DependsOn,
            other => {
                return ToolResult::Err(format!(
                    "edge_kind must be 'parent' or 'depends_on'; got {other:?}"
                ));
            }
        };

        let repo = PathBuf::from(ctx.workspace_root.as_std_path());
        let records = walk_specs(&repo);
        let all_ids: Vec<String> = records.iter().map(|r| r.canonical_id.clone()).collect();

        let root = match resolve(&args.from_ref, &all_ids) {
            Resolution::Resolved(id) => id,
            Resolution::Unresolved => {
                return ToolResult::Err(format!("no spec found for ref {:?}", args.from_ref));
            }
            Resolution::Ambiguous(cs) => {
                return ToolResult::Err(format!(
                    "ambiguous ref {:?}: matches {}: {}",
                    args.from_ref,
                    cs.len(),
                    cs.join(", ")
                ));
            }
        };

        // Build the graph once; the tools layer doesn't currently cache
        // (when index-db lands this turns into a SQLite query).
        let inputs: Vec<GraphInput> = records
            .iter()
            .map(|r| GraphInput {
                canonical_id: r.canonical_id.clone(),
                file: r.file.clone(),
                path: r.path.clone(),
            })
            .collect();
        let graph = SpecGraph::build(&inputs);

        let mut visited: HashSet<String> = HashSet::new();
        let tree = build_subtree(&graph, &root, edge_kind, depth, &mut visited);

        ToolResult::Ok(json!({ "root": tree }))
    }
}

/// Children of X = nodes whose declared edge of `kind` points at X. e.g.
/// for kind=Parent, children are nodes whose frontmatter `parent: X`
/// (inbound Parent edges to X).
fn build_subtree(
    graph: &SpecGraph,
    id: &str,
    kind: EdgeKind,
    remaining_depth: usize,
    visited: &mut HashSet<String>,
) -> Value {
    visited.insert(id.to_string());
    let node = graph.node(id);
    let kind_str = node.map(|n| n.kind.as_str()).unwrap_or("?");
    let status_str = node.map(|n| n.status.as_str()).unwrap_or("?");

    let mut children = Vec::<Value>::new();
    if remaining_depth > 0 {
        // inbound edges from the perspective of `id`: someone points AT id
        // with `kind`. Sort by id for deterministic output.
        let mut neighbours: Vec<&str> = graph
            .inbound(id)
            .into_iter()
            .filter(|(_, ek)| *ek == kind)
            .map(|(n, _)| n.id.as_str())
            .filter(|child_id| !visited.contains(*child_id))
            .collect();
        neighbours.sort();
        for child_id in neighbours {
            children.push(build_subtree(
                graph,
                child_id,
                kind,
                remaining_depth - 1,
                visited,
            ));
        }
    }

    json!({
        "ref": id,
        "kind": kind_str,
        "status": status_str,
        "children": children,
    })
}
