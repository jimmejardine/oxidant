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

pub use cargo_runner::{CargoCheck, CargoClippy, CargoTest};
pub use lsp_client::{
    LspClient, RustDiagnostics, RustGotoDefinition, RustHover, RustWorkspaceSymbols,
};

/// Register the rust-tools currently realised:
/// - cargo: cargo_check, cargo_clippy, cargo_test
/// - lsp:   rust_hover, rust_goto_definition, rust_workspace_symbols, rust_diagnostics
/// Syn tools and the remaining LSP tools (rename, code_actions, find_references)
/// follow as their wrappers land.
pub fn register_standard_tools(registry: &mut ToolRegistry) {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(CargoCheck),
        Arc::new(CargoClippy),
        Arc::new(CargoTest),
        Arc::new(RustHover),
        Arc::new(RustGotoDefinition),
        Arc::new(RustWorkspaceSymbols),
        Arc::new(RustDiagnostics),
    ];
    for t in tools {
        registry.register(t);
    }
}
