// Generic agent tools: fs, edit, bash, and the WorkspaceEdit substrate.
//
// Specs:
//   spec/contracts/workspace-edit.md
//   spec/components/tools/fs.md
//   spec/components/tools/edit.md
//   spec/components/tools/bash-runner.md
//   spec/components/tools/workspace-edit-substrate.md
//   spec/invariants/edits-are-atomic.md

pub mod workspace_edit;

pub use workspace_edit::{Position, Range, TextEdit, WorkspaceEdit};
