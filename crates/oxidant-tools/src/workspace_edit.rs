// Realises spec/contracts/workspace-edit.md.
//
// Atomic multi-file edit payload — the lingua franca for every code-changing
// path in oxidant (LSP refactors, syn transforms, model-driven edits). The
// workspace-edit substrate (see spec/components/tools/workspace-edit-substrate.md)
// consumes this; producers live in the various *-tools crates.

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct WorkspaceEdit {
    pub changes: HashMap<PathBuf, Vec<TextEdit>>,
}

#[derive(Debug, Clone)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
    /// Optional optimistic-concurrency check: if set, the current bytes at
    /// `range` must match this string or the substrate aborts the edit.
    pub expected_text: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Copy)]
pub struct Position {
    /// 0-indexed line number.
    pub line: u32,
    /// 0-indexed UTF-16 code unit offset within the line (LSP convention).
    /// The substrate converts to byte offsets once at apply time.
    pub character: u32,
}
