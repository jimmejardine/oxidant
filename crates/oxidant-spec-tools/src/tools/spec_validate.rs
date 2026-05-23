// Realises spec/tools/spec/spec-validate.md.

use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::validate::{Warning, WarningKind, validate};

pub struct SpecValidate;

#[derive(Deserialize)]
struct Args {
    #[serde(default, rename = "ref")]
    ref_id: Option<String>,
    #[serde(default)]
    kinds: Option<Vec<String>>,
}

#[async_trait]
impl Tool for SpecValidate {
    fn name(&self) -> &str {
        "spec_validate"
    }
    fn description(&self) -> &str {
        "Run the full spec validator and return structured warnings (frontmatter completeness, link integrity, length budgets, orphans, code-path existence, reachability, cycles). Warnings never fail the tool — informational, used for GUI badges and agent prompting."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ref":   { "type": "string", "description": "limit to this spec id; omit for tree-wide" },
                "kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "filter to these warning kinds (FrontmatterMissingRequired, MissingCodePath, Cycle, ...)"
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
        let mut warnings = validate(&repo);

        if let Some(ref_id) = &args.ref_id {
            warnings.retain(|w| w.spec_id.as_deref() == Some(ref_id.as_str()));
        }

        if let Some(kind_filter) = &args.kinds {
            let allowed: Vec<WarningKind> =
                kind_filter.iter().filter_map(|s| parse_kind(s)).collect();
            warnings.retain(|w| allowed.contains(&w.kind));
        }

        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for w in &warnings {
            *counts.entry(format!("{:?}", w.kind)).or_default() += 1;
        }

        let warnings_json: Vec<Value> = warnings.iter().map(warning_to_json).collect();
        ToolResult::Ok(json!({
            "ok":       true,
            "count":    warnings.len(),
            "counts":   counts,
            "warnings": warnings_json,
        }))
    }
}

fn warning_to_json(w: &Warning) -> Value {
    let location = w
        .location
        .as_ref()
        .map(|(path, line, col)| json!([path.to_string_lossy().replace('\\', "/"), line, col]));
    json!({
        "spec_id":  w.spec_id,
        "kind":     format!("{:?}", w.kind),
        "message":  w.message,
        "location": location,
    })
}

fn parse_kind(s: &str) -> Option<WarningKind> {
    match s {
        "FrontmatterMissingRequired" => Some(WarningKind::FrontmatterMissingRequired),
        "FrontmatterInvalidValue" => Some(WarningKind::FrontmatterInvalidValue),
        "DuplicateId" => Some(WarningKind::DuplicateId),
        "UnresolvedRef" => Some(WarningKind::UnresolvedRef),
        "ShortFormAmbiguous" => Some(WarningKind::ShortFormAmbiguous),
        "Orphan" => Some(WarningKind::Orphan),
        "Cycle" => Some(WarningKind::Cycle),
        "LengthBudgetExceeded" => Some(WarningKind::LengthBudgetExceeded),
        "MissingCodePath" => Some(WarningKind::MissingCodePath),
        "Reachability" => Some(WarningKind::Reachability),
        "ParseError" => Some(WarningKind::ParseError),
        _ => None,
    }
}
