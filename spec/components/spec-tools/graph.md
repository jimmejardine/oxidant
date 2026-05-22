---
id: graph
kind: component
parent: overview
order: 2
implements: []
depends_on:
  - components/spec-tools/frontmatter
  - components/spec-tools/index-db
code:
  - crates/oxidant-spec-tools/src/graph.rs
tests:
  - crates/oxidant-spec-tools/src/graph.rs
status: active
responsibility: |
  Construct an in-memory directed graph of spec nodes and edges (parent, implements, depends_on, body refs), and answer traversal queries.
---

Sits between the SQLite index and the query/validate/diff tools. Builds a `petgraph::DiGraph<Node, EdgeKind>` from the index, or directly from the parsed `SpecFile`s when the index is unavailable.

## Node and edge model

```rust
pub struct Node {
    pub id: String,
    pub kind: SpecKind,
    pub status: SpecStatus,
    pub path: PathBuf,
}

pub enum EdgeKind { Parent, Implements, DependsOn, BodyRef }
```

## Queries

```rust
fn ancestors(&self, id: &str) -> Vec<&Node>;
fn descendants(&self, id: &str) -> Vec<&Node>;
fn inbound(&self, id: &str) -> Vec<(&Node, EdgeKind)>;
fn outbound(&self, id: &str) -> Vec<(&Node, EdgeKind)>;
fn topo_sort_by_deps(&self) -> Vec<&Node>;          // by DependsOn edges
fn unreachable_from(&self, root: &str) -> Vec<&Node>;
fn shortest_path(&self, from: &str, to: &str) -> Option<Vec<&Node>>;
```

## Use sites

- [[components/spec-tools/validate]] — orphans, cycles, unresolved refs
- [[components/spec-tools/diff]] — knows which contracts a component implements
- [[tools/spec/spec-tree]] — hierarchical view
- [[tools/spec/spec-resolve-links]] — inbound + outbound for one node
- GUI [[components/gui/spec-tree-panel]] — render

## Performance

Graphs are small (hundreds to low thousands of nodes). No need for incremental graph updates — rebuild from the SQLite index on each query is fine. The SQLite index, in turn, is incrementally maintained.
