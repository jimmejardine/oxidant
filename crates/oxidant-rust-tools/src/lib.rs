// First-class Rust toolchain: rust-analyzer LSP client, cargo runner, syn-based AST queries.
//
// Specs:
//   spec/components/rust-tools/lsp.md
//   spec/components/rust-tools/cargo-runner.md
//   spec/components/rust-tools/syn-tools.md
//   spec/decisions/0009-no-ra-ap-crates-lsp-suffices.md

use std::sync::Arc;

use oxidant_core::{Tool, ToolRegistry};

pub mod cargo_runner;
pub mod lsp_client;
pub mod syn_query;

pub use cargo_runner::{CargoCheck, CargoClippy, CargoTest};
pub use lsp_client::{
    LspClient, RustCodeActions, RustDiagnostics, RustFindReferences, RustGotoDefinition, RustHover,
    RustRename, RustWorkspaceSymbols,
};
pub use syn_query::{SynAddDerive, SynAddUse, SynFindItems, SynRenameLocal};

/// Register every model-facing rust-tool currently realised:
/// - cargo: cargo_check, cargo_clippy, cargo_test
/// - lsp:   rust_hover, rust_goto_definition, rust_find_references,
///   rust_workspace_symbols, rust_rename, rust_code_actions, rust_diagnostics
/// - syn:   syn_find_items, syn_add_use, syn_add_derive, syn_rename_local
pub fn register_standard_tools(registry: &mut ToolRegistry) {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(CargoCheck),
        Arc::new(CargoClippy),
        Arc::new(CargoTest),
        Arc::new(RustHover),
        Arc::new(RustGotoDefinition),
        Arc::new(RustFindReferences),
        Arc::new(RustWorkspaceSymbols),
        Arc::new(RustRename),
        Arc::new(RustCodeActions),
        Arc::new(RustDiagnostics),
        Arc::new(SynFindItems),
        Arc::new(SynAddUse),
        Arc::new(SynAddDerive),
        Arc::new(SynRenameLocal),
    ];
    for t in tools {
        registry.register(t);
    }
}
