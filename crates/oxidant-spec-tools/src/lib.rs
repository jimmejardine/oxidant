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

pub mod diff;
pub mod frontmatter;
pub mod graph;
pub mod tools;
pub mod validate;

pub use diff::{Drift, diff_all, diff_spec};
pub use frontmatter::{
    FencedBlock, FrontmatterRecord, ParseError, RefMention, SpecFile, SpecKind, SpecStatus,
    extract_fenced_blocks, parse,
};
pub use graph::{EdgeKind, GraphInput, Node, Resolution, SpecGraph, resolve};
pub use tools::SpecDiff;
pub use validate::{Warning, WarningKind, validate};
