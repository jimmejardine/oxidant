// Live LSP integration tests. Marked #[ignore] because rust-analyzer takes
// ~10–30s to spawn and index, and may not be installed (`rustup component
// add rust-analyzer`). Run manually:
//
//   cargo test -p oxidant-rust-tools --test lsp_live -- --ignored --nocapture

use std::path::Path;

use camino::Utf8PathBuf;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use oxidant_core::{Tool, ToolContext, ToolResult};
use oxidant_rust_tools::{RustHover, RustWorkspaceSymbols};

fn ctx_for(dir: &Path) -> ToolContext {
    ToolContext {
        workspace_root: Utf8PathBuf::from_path_buf(dunce::canonicalize(dir).unwrap()).unwrap(),
        exploration_id: "lsp-live".into(),
        cancellation: CancellationToken::new(),
    }
}

fn write_sample_crate(dir: &Path) {
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "/// Adds two integers together.\npub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "requires rust-analyzer on PATH; slow ~30s cold start"]
async fn hover_on_function_signature() {
    let dir = TempDir::new().unwrap();
    write_sample_crate(dir.path());
    let ctx = ctx_for(dir.path());

    // line 1 = "pub fn add(...)", character 7 lands on `add`.
    let result = RustHover
        .invoke(
            json!({ "file": "src/lib.rs", "line": 1, "character": 7 }),
            &ctx,
        )
        .await;
    let v = match result {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("rust_hover failed: {e}"),
    };
    let sig = v["type_signature"].as_str().unwrap_or("");
    assert!(
        sig.contains("fn add") || sig.contains("add"),
        "expected signature for `add`, got {sig:?}"
    );
}

#[tokio::test]
#[ignore = "requires rust-analyzer on PATH; slow ~30s cold start"]
async fn workspace_symbols_finds_add() {
    let dir = TempDir::new().unwrap();
    write_sample_crate(dir.path());
    let ctx = ctx_for(dir.path());

    let result = RustWorkspaceSymbols
        .invoke(json!({ "query": "add" }), &ctx)
        .await;
    let v = match result {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("rust_workspace_symbols failed: {e}"),
    };
    let count = v["count"].as_u64().unwrap();
    assert!(count >= 1, "expected at least one symbol named `add`");
}
