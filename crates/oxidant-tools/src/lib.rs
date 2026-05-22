// Generic agent tools: fs, edit, bash, and the WorkspaceEdit substrate.
//
// Specs:
//   spec/contracts/workspace-edit.md
//   spec/components/tools/fs.md
//   spec/components/tools/edit.md
//   spec/components/tools/bash-runner.md
//   spec/components/tools/workspace-edit-substrate.md
//   spec/invariants/edits-are-atomic.md
//   spec/invariants/rust-files-parse-after-edit.md

use std::sync::Arc;

use oxidant_core::{Tool, ToolRegistry};

pub mod edit;
pub mod fs;
pub mod workspace_edit;

pub use edit::{ApplyEdits, EditString};
pub use fs::{FsRead, FsWrite, Glob, Grep};
pub use workspace_edit::{
    ApplyError, ApplyResult, ByteRange, FileApplyResult, Position, Range, TextEdit, WorkspaceEdit,
    apply,
};

/// Register the read-only and mutating model-facing tools provided by this
/// crate: `fs_read`, `fs_write`, `glob`, `grep`, `edit_string`, `apply_edits`.
/// The `bash` tool lands separately once its spec is realised.
pub fn register_standard_tools(registry: &mut ToolRegistry) {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(FsRead),
        Arc::new(FsWrite),
        Arc::new(Glob),
        Arc::new(Grep),
        Arc::new(EditString),
        Arc::new(ApplyEdits),
    ];
    for t in tools {
        registry.register(t);
    }
}
