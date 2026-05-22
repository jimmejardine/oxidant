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

pub use cargo_runner::{CargoCheck, CargoClippy, CargoTest};

/// Register the cargo-* tools currently realised: cargo_check, cargo_test,
/// cargo_clippy. LSP and syn tools follow when their components land.
pub fn register_standard_tools(registry: &mut ToolRegistry) {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(CargoCheck),
        Arc::new(CargoClippy),
        Arc::new(CargoTest),
    ];
    for t in tools {
        registry.register(t);
    }
}
