// Live LSP integration tests. They spawn rust-analyzer and wait for the
// `experimental/serverStatus { quiescent: true }` notification (plus
// `publishDiagnostics` for the relevant file) before issuing semantic
// queries. Cold start is ~5-10s on a tempdir crate.
//
// Requires rust-analyzer on PATH: `rustup component add rust-analyzer`.

use std::path::Path;

use camino::Utf8PathBuf;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use oxidant_core::{Tool, ToolContext, ToolResult};
use oxidant_rust_tools::{
    RustCodeActions, RustFindReferences, RustHover, RustRename, RustWorkspaceSymbols,
};

fn ctx_for(dir: &Path) -> ToolContext {
    ToolContext {
        workspace_root: Utf8PathBuf::from_path_buf(dunce::canonicalize(dir).unwrap()).unwrap(),
        exploration_id: "lsp-live".into(),
        cancellation: CancellationToken::new(),
        ui: None,
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
        "/// Adds two integers together.\npub fn add(a: i32, b: i32) -> i32 { a + b }\n\npub fn double(x: i32) -> i32 { add(x, x) }\n",
    )
    .unwrap();
}

#[tokio::test]
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
        sig.contains("fn add"),
        "expected signature for `fn add`, got {sig:?}"
    );
}

#[tokio::test]
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

#[tokio::test]
async fn find_references_returns_call_and_definition() {
    let dir = TempDir::new().unwrap();
    write_sample_crate(dir.path());
    let ctx = ctx_for(dir.path());

    // Position on `add` definition (line 1 char 7). The body has another
    // `add(x, x)` call on line 3.
    let result = RustFindReferences
        .invoke(
            json!({ "file": "src/lib.rs", "line": 1, "character": 7 }),
            &ctx,
        )
        .await;
    let v = match result {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("rust_find_references failed: {e}"),
    };
    let count = v["count"].as_u64().unwrap();
    assert!(
        count >= 2,
        "expected at least 2 references (def + call), got {count}"
    );
}

#[tokio::test]
async fn rename_preview_returns_workspace_edit() {
    let dir = TempDir::new().unwrap();
    write_sample_crate(dir.path());
    let ctx = ctx_for(dir.path());

    let result = RustRename
        .invoke(
            json!({
                "file": "src/lib.rs",
                "line": 1,
                "character": 7,
                "new_name": "sum"
            }),
            &ctx,
        )
        .await;
    let v = match result {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("rust_rename failed: {e}"),
    };
    assert_eq!(v["applied"], false);
    let edits_total = v["edits_total"].as_u64().unwrap();
    assert!(
        edits_total >= 2,
        "expected >=2 edits for definition + call site, got {edits_total}"
    );
    // File was untouched (apply=false).
    let original = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(original.contains("fn add"));
}

#[tokio::test]
async fn rename_apply_routes_through_substrate() {
    let dir = TempDir::new().unwrap();
    write_sample_crate(dir.path());
    let ctx = ctx_for(dir.path());

    let result = RustRename
        .invoke(
            json!({
                "file": "src/lib.rs",
                "line": 1,
                "character": 7,
                "new_name": "sum",
                "apply": true
            }),
            &ctx,
        )
        .await;
    let v = match result {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("rust_rename apply failed: {e}"),
    };
    assert_eq!(v["applied"], true);
    let content = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(
        content.contains("pub fn sum"),
        "expected file to contain `pub fn sum`, got:\n{content}"
    );
    assert!(
        content.contains("sum(x, x)"),
        "expected call site renamed too:\n{content}"
    );
}

#[tokio::test]
async fn rename_rejects_invalid_identifier() {
    let dir = TempDir::new().unwrap();
    write_sample_crate(dir.path());
    let ctx = ctx_for(dir.path());

    let result = RustRename
        .invoke(
            json!({
                "file": "src/lib.rs",
                "line": 1,
                "character": 7,
                "new_name": "1bad"
            }),
            &ctx,
        )
        .await;
    assert!(matches!(result, ToolResult::Err(_)));
}

#[tokio::test]
async fn code_actions_returns_list() {
    let dir = TempDir::new().unwrap();
    write_sample_crate(dir.path());
    let ctx = ctx_for(dir.path());

    // Point cursor on the function name. rust-analyzer assists fire on
    // point selections; wide ranges return fewer (or zero) actions.
    let result = RustCodeActions
        .invoke(
            json!({
                "file": "src/lib.rs",
                "range": {
                    "start": { "line": 1, "character": 7 },
                    "end":   { "line": 1, "character": 7 }
                }
            }),
            &ctx,
        )
        .await;
    let v = match result {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("rust_code_actions failed: {e}"),
    };
    // We only assert response shape — whether rust-analyzer offers assists
    // for this specific trivial function in a tempdir varies with the
    // server's lazy assist-table population. The tool's framing is the
    // contract we own; rust-analyzer's heuristics aren't.
    assert!(v["actions"].is_array(), "expected actions array, got {v}");
    assert!(v["count"].is_number(), "expected count number, got {v}");
    let actions = v["actions"].as_array().unwrap();
    for a in actions {
        assert!(
            a.get("title").and_then(|v| v.as_str()).is_some(),
            "every action should have a title"
        );
    }
}
