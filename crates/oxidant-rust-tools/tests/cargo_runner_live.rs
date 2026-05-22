// Live integration tests that actually shell out to cargo. They build a
// throwaway Rust project in a temp dir to avoid interfering with the
// outer workspace's target/ or cargo lock state.

use camino::Utf8PathBuf;
use serde_json::json;
use std::path::Path;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use oxidant_core::{Tool, ToolContext, ToolResult};
use oxidant_rust_tools::{CargoCheck, CargoTest};

fn ctx_for(dir: &Path) -> ToolContext {
    ToolContext {
        workspace_root: Utf8PathBuf::from_path_buf(dunce::canonicalize(dir).unwrap()).unwrap(),
        exploration_id: "live-test".into(),
        cancellation: CancellationToken::new(),
    }
}

fn write_clean_lib(dir: &Path) {
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n\
         #[cfg(test)]\nmod tests {\n#[test]\nfn it_works() { assert_eq!(super::add(2, 2), 4); }\n}\n",
    )
    .unwrap();
}

fn write_broken_lib(dir: &Path) {
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"broken\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    // type mismatch — guaranteed E0308
    std::fs::write(
        dir.join("src/lib.rs"),
        "pub fn bad() -> i32 { \"not an int\" }\n",
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "live cargo subprocess; slow"]
async fn cargo_check_ok_on_clean_project() {
    let dir = TempDir::new().unwrap();
    write_clean_lib(dir.path());
    let v = match CargoCheck.invoke(json!({}), &ctx_for(dir.path())).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["ok"], true);
    assert_eq!(v["summary"]["errors"], 0);
}

#[tokio::test]
#[ignore = "live cargo subprocess; slow"]
async fn cargo_check_returns_diagnostics_on_broken_project() {
    let dir = TempDir::new().unwrap();
    write_broken_lib(dir.path());
    let v = match CargoCheck.invoke(json!({}), &ctx_for(dir.path())).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["ok"], false);
    let errs = v["summary"]["errors"].as_u64().unwrap();
    assert!(errs >= 1, "expected at least one error, got {}", errs);
    let msgs = v["messages"].as_array().unwrap();
    assert!(
        msgs.iter().any(|m| m["code"] == "E0308"),
        "expected E0308 in messages, got {msgs:?}"
    );
}

#[tokio::test]
#[ignore = "live cargo subprocess; slow"]
async fn cargo_test_runs_a_passing_test() {
    let dir = TempDir::new().unwrap();
    write_clean_lib(dir.path());
    let v = match CargoTest.invoke(json!({}), &ctx_for(dir.path())).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["ok"], true);
    assert_eq!(v["passed"], 1);
    assert_eq!(v["failed"], 0);
}
