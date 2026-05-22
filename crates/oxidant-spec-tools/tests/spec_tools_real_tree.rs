// Integration tests for the spec_read / spec_for_file tools against the
// real spec tree of this repository.

use std::path::PathBuf;

use camino::Utf8PathBuf;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use oxidant_core::{Tool, ToolContext, ToolResult};
use oxidant_spec_tools::{SpecForFile, SpecRead, SpecResolveLinks, SpecTree, SpecValidate};

fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap().parent().unwrap().to_path_buf()
}

fn ctx() -> ToolContext {
    ToolContext {
        workspace_root: Utf8PathBuf::from_path_buf(repo_root()).unwrap(),
        exploration_id: "test".into(),
        cancellation: CancellationToken::new(),
    }
}

#[tokio::test]
async fn spec_read_canonical_ref() {
    let v = match SpecRead
        .invoke(json!({"ref": "contracts/tool"}), &ctx())
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["canonical_id"], "contracts/tool");
    assert_eq!(v["kind"], "contract");
    assert_eq!(v["id"], "tool");
    assert!(v["path"].as_str().unwrap().ends_with("spec/contracts/tool.md"));
    assert!(v["body"].as_str().unwrap().contains("Tool"));
    // outbound_refs is sorted; for contracts/tool we expect components and other contracts.
    let outbound = v["outbound_refs"].as_array().unwrap();
    assert!(!outbound.is_empty(), "contracts/tool should have body refs");
}

#[tokio::test]
async fn spec_read_short_ref() {
    let v = match SpecRead.invoke(json!({"ref": "tool"}), &ctx()).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    // `tool` as a short ref should uniquely resolve to contracts/tool
    // (frontmatter id == "tool"). If a future spec introduces another file
    // with id "tool", this test will catch the ambiguity.
    assert_eq!(v["canonical_id"], "contracts/tool");
}

#[tokio::test]
async fn spec_read_unknown_ref_errors() {
    let result = SpecRead
        .invoke(json!({"ref": "this/does/not/exist"}), &ctx())
        .await;
    assert!(matches!(result, ToolResult::Err(_)));
}

#[tokio::test]
async fn spec_for_file_finds_workspace_edit_substrate() {
    let v = match SpecForFile
        .invoke(
            json!({"path": "crates/oxidant-tools/src/workspace_edit.rs"}),
            &ctx(),
        )
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    let count = v["count"].as_u64().unwrap();
    assert!(count >= 2, "expected >= 2 specs claiming workspace_edit.rs, got {count}");
    let refs: Vec<&str> = v["specs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["ref"].as_str().unwrap())
        .collect();
    assert!(refs.contains(&"contracts/workspace-edit"));
    assert!(refs.contains(&"components/tools/workspace-edit-substrate"));
}

#[tokio::test]
async fn spec_for_file_with_windows_style_slashes() {
    // The model might emit either slash style; both must work.
    let v = match SpecForFile
        .invoke(
            json!({"path": "crates\\oxidant-core\\src\\registry.rs"}),
            &ctx(),
        )
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    let count = v["count"].as_u64().unwrap();
    assert!(count >= 2, "expected specs for registry.rs; got {count}");
}

#[tokio::test]
async fn spec_for_file_unknown_path_returns_empty() {
    let v = match SpecForFile
        .invoke(json!({"path": "does/not/exist.rs"}), &ctx())
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["count"], 0);
}

#[tokio::test]
async fn spec_tree_default_args_returns_overview_rooted() {
    let v = match SpecTree.invoke(json!({}), &ctx()).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["root"]["ref"], "overview");
    assert_eq!(v["root"]["kind"], "overview");
    let children = v["root"]["children"].as_array().expect("children array");
    assert!(!children.is_empty(), "overview should have parent-children");
}

#[tokio::test]
async fn spec_tree_depends_on_root_at_a_substrate() {
    let v = match SpecTree
        .invoke(
            json!({
                "from_ref": "components/tools/workspace-edit-substrate",
                "edge_kind": "depends_on",
                "depth": 2
            }),
            &ctx(),
        )
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["root"]["ref"], "components/tools/workspace-edit-substrate");
    // edit and apply-edits depend on the substrate — both should appear as children.
    let children = v["root"]["children"].as_array().expect("children");
    let child_refs: Vec<&str> = children.iter().map(|c| c["ref"].as_str().unwrap()).collect();
    assert!(
        child_refs.contains(&"components/tools/edit"),
        "expected components/tools/edit as child; got {child_refs:?}"
    );
}

#[tokio::test]
async fn spec_tree_rejects_unknown_edge_kind() {
    let result = SpecTree
        .invoke(json!({ "edge_kind": "implements" }), &ctx())
        .await;
    assert!(matches!(result, ToolResult::Err(_)));
}

#[tokio::test]
async fn spec_resolve_links_for_tool_contract() {
    let v = match SpecResolveLinks
        .invoke(json!({"ref": "contracts/tool"}), &ctx())
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["ref"], "contracts/tool");

    // contracts/tool has many specs depending on it (every tool's `implements`
    // points here) and at least the registry component depends on it.
    let inbound = v["inbound"].as_array().expect("inbound array");
    assert!(
        !inbound.is_empty(),
        "contracts/tool should have inbound edges from implementing tools and the registry component"
    );

    let outbound = v["outbound"].as_array().expect("outbound array");
    // Outbound at minimum contains the parent (`overview`).
    let has_overview_parent = outbound.iter().any(|e| {
        e["to"].as_str() == Some("overview") && e["edge"].as_str() == Some("parent")
    });
    assert!(has_overview_parent, "expected parent edge to overview");
}

#[tokio::test]
async fn spec_resolve_links_unknown_ref_errors() {
    let result = SpecResolveLinks
        .invoke(json!({"ref": "does/not/exist"}), &ctx())
        .await;
    assert!(matches!(result, ToolResult::Err(_)));
}

#[tokio::test]
async fn spec_validate_tree_wide_returns_warnings() {
    let v = match SpecValidate.invoke(json!({}), &ctx()).await {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["ok"], true);
    let count = v["count"].as_u64().unwrap();
    assert!(count > 0, "expected the baseline to surface real warnings");
    let counts = v["counts"].as_object().unwrap();
    assert!(counts.contains_key("MissingCodePath"));
}

#[tokio::test]
async fn spec_validate_kind_filter_works() {
    let v = match SpecValidate
        .invoke(json!({ "kinds": ["MissingCodePath"] }), &ctx())
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    let warnings = v["warnings"].as_array().unwrap();
    for w in warnings {
        assert_eq!(w["kind"], "MissingCodePath");
    }
}

#[tokio::test]
async fn spec_validate_unknown_kind_filter_yields_empty() {
    let v = match SpecValidate
        .invoke(json!({ "kinds": ["NotARealKind"] }), &ctx())
        .await
    {
        ToolResult::Ok(v) => v,
        ToolResult::Err(e) => panic!("err: {e}"),
    };
    assert_eq!(v["count"], 0);
}
