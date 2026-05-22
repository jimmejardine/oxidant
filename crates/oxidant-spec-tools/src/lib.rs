// Spec graph: frontmatter parsing, link graph, validation, drift detection,
// SQLite metadata index, Tantivy full-text search, git-derived timeline.
//
// Specs:
//   spec/components/spec-tools/frontmatter.md
//   spec/components/spec-tools/graph.md
//   spec/components/spec-tools/validate.md
//   spec/components/spec-tools/diff.md
//   spec/components/spec-tools/index-db.md
//   spec/components/spec-tools/search-index.md
//   spec/components/spec-tools/timeline.md
//   spec/decisions/0008-spec-is-canonical.md
//   spec/decisions/0010-spec-index-and-search.md

use std::sync::Arc;

use oxidant_core::{Tool, ToolRegistry};

pub mod diff;
pub mod frontmatter;
pub mod graph;
pub mod tools;
pub mod validate;
pub mod walker;

pub use diff::{Drift, diff_all, diff_spec};
pub use frontmatter::{
    FencedBlock, FrontmatterRecord, ParseError, RefMention, SpecFile, SpecKind, SpecStatus,
    extract_fenced_blocks, parse,
};
pub use graph::{EdgeKind, GraphInput, Node, Resolution, SpecGraph, resolve};
pub use tools::{SpecDiff, SpecForFile, SpecRead, SpecResolveLinks, SpecTree, SpecValidate};
pub use validate::{Warning, WarningKind, validate};
pub use walker::{SpecRecord, walk_specs};

/// Register every model-facing spec tool declared under spec/tools/spec/:
/// spec_diff, spec_read, spec_for_file, spec_tree, spec_resolve_links,
/// spec_validate.
pub fn register_standard_tools(registry: &mut ToolRegistry) {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(SpecDiff),
        Arc::new(SpecRead),
        Arc::new(SpecForFile),
        Arc::new(SpecTree),
        Arc::new(SpecResolveLinks),
        Arc::new(SpecValidate),
    ];
    for t in tools {
        registry.register(t);
    }
}
