// CLI surface over the spec_tools crate. Each subcommand wraps the same
// Tool impl the agent uses, so behaviour stays in sync. Default output is
// human-readable; `--json` emits the tool's raw JSON envelope. For the two
// drift detectors (`validate`, `diff`), `--strict` flips warnings into a
// non-zero exit so this can gate CI per
// spec/decisions/0011-specs-claim-their-tests.md style guidance.

use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use clap::Subcommand;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use oxidant_core::{Tool, ToolContext, ToolResult};
use oxidant_spec_tools::{
    SpecCoverage, SpecDiff, SpecForFile, SpecRead, SpecResolveLinks, SpecTree, SpecValidate,
};

#[derive(Subcommand)]
pub enum SpecCommand {
    /// Lint the spec tree: frontmatter, links, length budgets, orphans, code paths.
    Validate {
        /// Limit to a single spec ref (e.g. tools/edit/apply-edits).
        #[arg(long, value_name = "REF")]
        r#ref: Option<String>,
        /// Filter to specific warning kinds (comma-separated, e.g. MissingCodePath,Orphan).
        #[arg(long, value_delimiter = ',')]
        kinds: Option<Vec<String>>,
        /// Exit with code 1 if any warnings are produced. Use in CI.
        #[arg(long)]
        strict: bool,
        /// Emit the raw JSON envelope instead of the human view.
        #[arg(long)]
        json: bool,
    },
    /// Detect spec ↔ code drift (trait-method drift, missing code paths).
    Diff {
        /// Limit to a single spec ref.
        #[arg(long, value_name = "REF")]
        r#ref: Option<String>,
        /// Exit with code 1 if any drifts are reported. Use in CI.
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        json: bool,
    },
    /// Fetch one spec by canonical or short ref.
    Read {
        /// Spec ref (e.g. tools/edit/apply-edits or apply-edits).
        #[arg(value_name = "REF")]
        r#ref: String,
        #[arg(long)]
        json: bool,
    },
    /// Hierarchical view of the spec graph rooted at a ref.
    Tree {
        /// Root ref.
        #[arg(long, default_value = "overview")]
        from: String,
        /// Maximum depth (capped at 12).
        #[arg(long, default_value_t = 4)]
        depth: usize,
        /// Edge kind to follow: parent (doc hierarchy) or depends_on (impact tree).
        #[arg(long, default_value = "parent")]
        edge: String,
        #[arg(long)]
        json: bool,
    },
    /// Reverse lookup: which specs declare this code file in their `code:` frontmatter?
    ForFile {
        /// Path relative to workspace root.
        path: String,
        #[arg(long)]
        json: bool,
    },
    /// Inbound + outbound edges for one spec (impact analysis).
    ResolveLinks {
        #[arg(value_name = "REF")]
        r#ref: String,
        #[arg(long)]
        json: bool,
    },
    /// Find Rust source files no spec transitively reaches (code not anchored to any spec).
    Coverage {
        /// Exit with code 1 if any uncovered files are found. Use in CI.
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        json: bool,
    },
}

pub fn run(workspace: PathBuf, command: SpecCommand) -> anyhow::Result<ExitCode> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(dispatch(workspace, command))
}

async fn dispatch(workspace: PathBuf, command: SpecCommand) -> anyhow::Result<ExitCode> {
    let ctx = build_ctx(&workspace)?;

    match command {
        SpecCommand::Validate {
            r#ref,
            kinds,
            strict,
            json: as_json,
        } => {
            let args = json!({ "ref": r#ref, "kinds": kinds });
            let value = invoke(&SpecValidate, args, &ctx).await?;
            let count = value.get("count").and_then(Value::as_u64).unwrap_or(0);
            if as_json {
                print_json(&value);
            } else {
                print_validate(&value);
            }
            Ok(strict_exit(strict, count))
        }
        SpecCommand::Diff {
            r#ref,
            strict,
            json: as_json,
        } => {
            let args = json!({ "ref": r#ref });
            let value = invoke(&SpecDiff, args, &ctx).await?;
            let count = value.get("count").and_then(Value::as_u64).unwrap_or(0);
            if as_json {
                print_json(&value);
            } else {
                print_diff(&value);
            }
            Ok(strict_exit(strict, count))
        }
        SpecCommand::Read {
            r#ref,
            json: as_json,
        } => {
            let value = invoke(&SpecRead, json!({ "ref": r#ref }), &ctx).await?;
            if as_json {
                print_json(&value);
            } else {
                print_read(&value);
            }
            Ok(ExitCode::SUCCESS)
        }
        SpecCommand::Tree {
            from,
            depth,
            edge,
            json: as_json,
        } => {
            let args = json!({ "from_ref": from, "depth": depth, "edge_kind": edge });
            let value = invoke(&SpecTree, args, &ctx).await?;
            if as_json {
                print_json(&value);
            } else if let Some(root) = value.get("root") {
                print_tree(root, 0);
            }
            Ok(ExitCode::SUCCESS)
        }
        SpecCommand::ForFile {
            path,
            json: as_json,
        } => {
            let value = invoke(&SpecForFile, json!({ "path": path }), &ctx).await?;
            if as_json {
                print_json(&value);
            } else {
                print_for_file(&value);
            }
            Ok(ExitCode::SUCCESS)
        }
        SpecCommand::ResolveLinks {
            r#ref,
            json: as_json,
        } => {
            let value = invoke(&SpecResolveLinks, json!({ "ref": r#ref }), &ctx).await?;
            if as_json {
                print_json(&value);
            } else {
                print_resolve_links(&value);
            }
            Ok(ExitCode::SUCCESS)
        }
        SpecCommand::Coverage {
            strict,
            json: as_json,
        } => {
            let value = invoke(&SpecCoverage, json!({}), &ctx).await?;
            let count = value.get("count").and_then(Value::as_u64).unwrap_or(0);
            if as_json {
                print_json(&value);
            } else {
                print_coverage(&value);
            }
            Ok(strict_exit(strict, count))
        }
    }
}

fn build_ctx(workspace: &std::path::Path) -> anyhow::Result<ToolContext> {
    let canonical = dunce::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let utf8 = Utf8PathBuf::from_path_buf(canonical)
        .map_err(|p| anyhow::anyhow!("workspace path is not valid UTF-8: {}", p.display()))?;
    Ok(ToolContext {
        workspace_root: utf8,
        exploration_id: "cli".to_string(),
        cancellation: CancellationToken::new(),
        ui: None,
    })
}

async fn invoke(tool: &dyn Tool, args: Value, ctx: &ToolContext) -> anyhow::Result<Value> {
    match tool.invoke(args, ctx).await {
        ToolResult::Ok(v) => Ok(v),
        ToolResult::Err(e) => anyhow::bail!("{}: {}", tool.name(), e),
    }
}

fn strict_exit(strict: bool, count: u64) -> ExitCode {
    if strict && count > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

fn print_validate(value: &Value) {
    let count = value.get("count").and_then(Value::as_u64).unwrap_or(0);
    let warnings = value.get("warnings").and_then(Value::as_array);
    if count == 0 {
        println!("spec validate: clean (0 warnings)");
        return;
    }
    println!("spec validate: {count} warning(s)");
    if let Some(arr) = warnings {
        for w in arr {
            let kind = w.get("kind").and_then(Value::as_str).unwrap_or("?");
            let id = w.get("spec_id").and_then(Value::as_str).unwrap_or("-");
            let msg = w.get("message").and_then(Value::as_str).unwrap_or("");
            let loc = w
                .get("location")
                .and_then(Value::as_array)
                .map(|a| {
                    let path = a.first().and_then(Value::as_str).unwrap_or("?");
                    let line = a.get(1).and_then(Value::as_u64).unwrap_or(0);
                    let col = a.get(2).and_then(Value::as_u64).unwrap_or(0);
                    format!(" ({path}:{line}:{col})")
                })
                .unwrap_or_default();
            println!("  [{kind}] {id}: {msg}{loc}");
        }
    }
}

fn print_diff(value: &Value) {
    let count = value.get("count").and_then(Value::as_u64).unwrap_or(0);
    let drifts = value.get("drifts").and_then(Value::as_array);
    if count == 0 {
        println!("spec diff: no drift");
        return;
    }
    println!("spec diff: {count} drift(s)");
    if let Some(arr) = drifts {
        for d in arr {
            let kind = d.get("kind").and_then(Value::as_str).unwrap_or("?");
            match kind {
                "MethodAdded" | "MethodRemoved" => {
                    let id = d.get("contract_id").and_then(Value::as_str).unwrap_or("-");
                    let m = d.get("method").and_then(Value::as_str).unwrap_or("-");
                    println!("  [{kind}] {id}::{m}");
                }
                "MethodSignatureChanged" => {
                    let id = d.get("contract_id").and_then(Value::as_str).unwrap_or("-");
                    let m = d.get("method").and_then(Value::as_str).unwrap_or("-");
                    let spec = d.get("spec").and_then(Value::as_str).unwrap_or("");
                    let code = d.get("code").and_then(Value::as_str).unwrap_or("");
                    println!("  [{kind}] {id}::{m}\n      spec: {spec}\n      code: {code}");
                }
                "MissingCodePath" => {
                    let id = d.get("spec_id").and_then(Value::as_str).unwrap_or("-");
                    let path = d.get("path").and_then(Value::as_str).unwrap_or("-");
                    println!("  [{kind}] {id}: {path}");
                }
                _ => println!("  {d}"),
            }
        }
    }
}

fn print_read(value: &Value) {
    let id = value
        .get("canonical_id")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let kind = value.get("kind").and_then(Value::as_str).unwrap_or("?");
    let status = value.get("status").and_then(Value::as_str).unwrap_or("?");
    let path = value.get("path").and_then(Value::as_str).unwrap_or("?");
    println!("# {id}");
    println!("kind: {kind}    status: {status}    path: {path}");
    println!();
    if let Some(body) = value.get("body").and_then(Value::as_str) {
        println!("{body}");
    }
}

fn print_tree(node: &Value, depth: usize) {
    let r = node.get("ref").and_then(Value::as_str).unwrap_or("?");
    let kind = node.get("kind").and_then(Value::as_str).unwrap_or("?");
    let status = node.get("status").and_then(Value::as_str).unwrap_or("?");
    println!(
        "{:indent$}- {r}  ({kind}, {status})",
        "",
        indent = depth * 2
    );
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            print_tree(child, depth + 1);
        }
    }
}

fn print_for_file(value: &Value) {
    let path = value.get("path").and_then(Value::as_str).unwrap_or("?");
    let count = value.get("count").and_then(Value::as_u64).unwrap_or(0);
    if count == 0 {
        println!("{path}: no spec declares this code path");
        return;
    }
    println!("{path}: {count} spec(s)");
    if let Some(arr) = value.get("specs").and_then(Value::as_array) {
        for s in arr {
            let r = s.get("ref").and_then(Value::as_str).unwrap_or("?");
            let kind = s.get("kind").and_then(Value::as_str).unwrap_or("?");
            let resp = s
                .get("responsibility")
                .and_then(Value::as_str)
                .unwrap_or("");
            println!("  [{kind}] {r}");
            if !resp.is_empty() {
                println!("    {}", resp.lines().next().unwrap_or(""));
            }
        }
    }
}

fn print_coverage(value: &Value) {
    let count = value.get("count").and_then(Value::as_u64).unwrap_or(0);
    let covered = value
        .get("covered_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let seeds = value.get("seed_count").and_then(Value::as_u64).unwrap_or(0);
    if let Some(missing) = value.get("missing_seeds").and_then(Value::as_array)
        && !missing.is_empty()
    {
        println!("missing seed files (declared in a spec's code: but absent):");
        for m in missing {
            if let Some(p) = m.as_str() {
                println!("  {p}");
            }
        }
    }
    if count == 0 {
        println!(
            "spec coverage: every source file is reachable from a spec ({seeds} seeds, {covered} covered)"
        );
        return;
    }
    println!(
        "spec coverage: {count} file(s) not reachable from any spec ({seeds} seeds, {covered} covered)"
    );
    let mut last_crate = String::new();
    if let Some(arr) = value.get("uncovered").and_then(Value::as_array) {
        for u in arr {
            let krate = u.get("krate").and_then(Value::as_str).unwrap_or("");
            let file = u.get("file").and_then(Value::as_str).unwrap_or("?");
            if krate != last_crate {
                println!("  [{krate}]");
                last_crate = krate.to_string();
            }
            println!("    {file}");
        }
    }
}

fn print_resolve_links(value: &Value) {
    let r = value.get("ref").and_then(Value::as_str).unwrap_or("?");
    println!("# {r}");
    println!("inbound:");
    if let Some(arr) = value.get("inbound").and_then(Value::as_array) {
        if arr.is_empty() {
            println!("  (none)");
        }
        for e in arr {
            let from = e.get("from").and_then(Value::as_str).unwrap_or("?");
            let edge = e.get("edge").and_then(Value::as_str).unwrap_or("?");
            println!("  {from}  [{edge}]");
        }
    }
    println!("outbound:");
    if let Some(arr) = value.get("outbound").and_then(Value::as_array) {
        if arr.is_empty() {
            println!("  (none)");
        }
        for e in arr {
            let to = e.get("to").and_then(Value::as_str).unwrap_or("?");
            let edge = e.get("edge").and_then(Value::as_str).unwrap_or("?");
            println!("  {to}  [{edge}]");
        }
    }
}
