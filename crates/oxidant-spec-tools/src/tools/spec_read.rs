// Realises spec/tools/spec/spec-read.md.

use std::collections::BTreeSet;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::frontmatter::{SpecFile, SpecKind, SpecStatus};
use crate::graph::{Resolution, resolve};
use crate::walker::{SpecRecord, walk_specs};

pub struct SpecRead;

#[derive(Deserialize)]
struct Args {
    #[serde(rename = "ref")]
    ref_id: String,
}

#[async_trait]
impl Tool for SpecRead {
    fn name(&self) -> &str {
        "spec_read"
    }
    fn description(&self) -> &str {
        "Fetch one spec by canonical ref (e.g. `tools/edit/apply-edits`) or short ref (`apply-edits`). Returns parsed frontmatter, body, and the spec's outbound refs."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["ref"],
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "canonical (tools/edit/apply-edits) or short (apply-edits)"
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
        let repo = PathBuf::from(ctx.workspace_root.as_std_path());
        let records = walk_specs(&repo);
        let all_ids: Vec<String> = records.iter().map(|r| r.canonical_id.clone()).collect();

        let canonical = match resolve(&args.ref_id, &all_ids) {
            Resolution::Resolved(id) => id,
            Resolution::Unresolved => {
                return ToolResult::Err(format!(
                    "no spec found for ref {:?} (tried canonical match and short-form lookup)",
                    args.ref_id
                ));
            }
            Resolution::Ambiguous(candidates) => {
                return ToolResult::Err(format!(
                    "ambiguous short ref {:?}: matches {} specs: {}",
                    args.ref_id,
                    candidates.len(),
                    candidates.join(", ")
                ));
            }
        };
        let Some(record) = records.iter().find(|r| r.canonical_id == canonical) else {
            return ToolResult::Err(format!("internal: resolved id {canonical} not found in walk"));
        };

        ToolResult::Ok(json!({
            "id":           record.file.frontmatter.id,
            "canonical_id": record.canonical_id,
            "kind":         record.file.frontmatter.kind.as_str(),
            "status":       record.file.frontmatter.status.as_str(),
            "path":         relative_to_repo(&repo, &record.path),
            "frontmatter":  frontmatter_to_json(&record.file),
            "body":         record.file.body,
            "outbound_refs": outbound_refs(record),
        }))
    }
}

fn relative_to_repo(repo: &std::path::Path, path: &std::path::Path) -> String {
    let rel = path.strip_prefix(repo).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

fn frontmatter_to_json(file: &SpecFile) -> Value {
    let fm = &file.frontmatter;
    let mut obj = serde_json::Map::new();
    obj.insert("id".into(), json!(fm.id));
    obj.insert("kind".into(), json!(fm.kind.as_str()));
    obj.insert("status".into(), json!(fm.status.as_str()));
    if let Some(o) = fm.order {
        obj.insert("order".into(), json!(o));
    }
    if let Some(p) = &fm.parent {
        obj.insert("parent".into(), json!(p));
    }
    if !fm.implements.is_empty() {
        obj.insert("implements".into(), json!(fm.implements));
    }
    if !fm.depends_on.is_empty() {
        obj.insert("depends_on".into(), json!(fm.depends_on));
    }
    if !fm.code.is_empty() {
        let paths: Vec<String> = fm
            .code
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        obj.insert("code".into(), json!(paths));
    }
    if let Some(r) = &fm.responsibility {
        obj.insert("responsibility".into(), json!(r));
    }
    if let Value::Object(extras) = &fm.extras {
        for (k, v) in extras {
            obj.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    // Suppress unused-variant warnings for SpecKind/SpecStatus referenced via as_str()
    let _ = SpecKind::Component;
    let _ = SpecStatus::Active;
    Value::Object(obj)
}

fn outbound_refs(record: &SpecRecord) -> Vec<String> {
    // Outbound = body refs + parent + implements + depends_on. Deduplicated,
    // sorted for stable output.
    let mut set = BTreeSet::<String>::new();
    let fm = &record.file.frontmatter;
    if let Some(p) = &fm.parent {
        set.insert(p.clone());
    }
    for r in &fm.implements {
        set.insert(r.clone());
    }
    for r in &fm.depends_on {
        set.insert(r.clone());
    }
    for r in &record.file.refs_in_body {
        set.insert(r.raw.clone());
    }
    set.into_iter().collect()
}
