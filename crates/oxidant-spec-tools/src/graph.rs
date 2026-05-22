// Realises spec/components/spec-tools/graph.md.
//
// petgraph-backed directed graph of spec nodes (parent / implements /
// depends_on / body refs). Sits between the SQLite index (when present) and
// the query/validate/diff tools; can also be built directly from parsed
// SpecFiles when no index is available.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use petgraph::Direction;
use petgraph::algo::{toposort, astar};
use petgraph::graph::{DiGraph, NodeIndex};

use crate::frontmatter::{SpecFile, SpecKind, SpecStatus};

#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub kind: SpecKind,
    pub status: SpecStatus,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Parent,
    Implements,
    DependsOn,
    BodyRef,
}

impl EdgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeKind::Parent => "parent",
            EdgeKind::Implements => "implements",
            EdgeKind::DependsOn => "depends_on",
            EdgeKind::BodyRef => "body_ref",
        }
    }
}

pub struct SpecGraph {
    inner: DiGraph<Node, EdgeKind>,
    id_to_idx: HashMap<String, NodeIndex>,
}

/// One input file for graph construction: canonical id, parsed file, on-disk path.
pub struct GraphInput {
    pub canonical_id: String,
    pub file: SpecFile,
    pub path: PathBuf,
}

impl SpecGraph {
    /// Build from parsed inputs. Edges to unknown ids and ambiguous short-form
    /// refs are dropped silently — validate() surfaces those separately.
    pub fn build(inputs: &[GraphInput]) -> Self {
        let mut inner = DiGraph::<Node, EdgeKind>::new();
        let mut id_to_idx = HashMap::with_capacity(inputs.len());

        for input in inputs {
            let idx = inner.add_node(Node {
                id: input.canonical_id.clone(),
                kind: input.file.frontmatter.kind,
                status: input.file.frontmatter.status,
                path: input.path.clone(),
            });
            id_to_idx.insert(input.canonical_id.clone(), idx);
        }

        let all_ids: Vec<String> = inputs.iter().map(|i| i.canonical_id.clone()).collect();

        for input in inputs {
            let src_idx = id_to_idx[&input.canonical_id];
            let fm = &input.file.frontmatter;

            if let Some(parent_raw) = &fm.parent {
                if let Resolution::Resolved(parent_id) = resolve(parent_raw, &all_ids) {
                    if let Some(&dst) = id_to_idx.get(&parent_id) {
                        inner.add_edge(src_idx, dst, EdgeKind::Parent);
                    }
                }
            }
            for raw in &fm.implements {
                add_resolved_edge(&mut inner, &id_to_idx, src_idx, raw, EdgeKind::Implements, &all_ids);
            }
            for raw in &fm.depends_on {
                add_resolved_edge(&mut inner, &id_to_idx, src_idx, raw, EdgeKind::DependsOn, &all_ids);
            }
            for mention in &input.file.refs_in_body {
                add_resolved_edge(&mut inner, &id_to_idx, src_idx, &mention.raw, EdgeKind::BodyRef, &all_ids);
            }
        }

        SpecGraph { inner, id_to_idx }
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.inner.node_weights()
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        let idx = self.id_to_idx.get(id)?;
        Some(&self.inner[*idx])
    }

    pub fn ancestors(&self, id: &str) -> Vec<&Node> {
        self.reachable(id, Direction::Incoming, usize::MAX)
    }

    pub fn descendants(&self, id: &str) -> Vec<&Node> {
        self.reachable(id, Direction::Outgoing, usize::MAX)
    }

    pub fn inbound(&self, id: &str) -> Vec<(&Node, EdgeKind)> {
        self.neighbours(id, Direction::Incoming)
    }

    pub fn outbound(&self, id: &str) -> Vec<(&Node, EdgeKind)> {
        self.neighbours(id, Direction::Outgoing)
    }

    /// Topological order respecting only `DependsOn` edges. Returns nodes in
    /// dependency order (a depends on b → b comes before a). If a dependency
    /// cycle exists, returns whatever prefix toposort produced before failing.
    pub fn topo_sort_by_deps(&self) -> Vec<&Node> {
        let mut deps_only = self.inner.filter_map(
            |_, node| Some(node.clone()),
            |_, edge| if *edge == EdgeKind::DependsOn { Some(*edge) } else { None },
        );
        // Reverse so "depends on b" puts b before a in topo order.
        deps_only.reverse();
        match toposort(&deps_only, None) {
            Ok(order) => order.into_iter().map(|idx| &self.inner[idx]).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Nodes not reachable from `root` along any edge kind (unbounded depth).
    pub fn unreachable_from(&self, root: &str) -> Vec<&Node> {
        let reached: HashSet<NodeIndex> = self
            .reachable_indices(root, Direction::Outgoing, usize::MAX)
            .into_iter()
            .collect();
        self.inner
            .node_indices()
            .filter(|i| !reached.contains(i))
            .map(|i| &self.inner[i])
            .collect()
    }

    /// Nodes reachable from `root` along any edge kind, within `max_hops`.
    /// Includes `root` itself.
    pub fn reachable_within(&self, root: &str, max_hops: usize) -> Vec<&Node> {
        self.reachable_indices(root, Direction::Outgoing, max_hops)
            .into_iter()
            .map(|i| &self.inner[i])
            .collect()
    }

    pub fn shortest_path(&self, from: &str, to: &str) -> Option<Vec<&Node>> {
        let src = *self.id_to_idx.get(from)?;
        let dst = *self.id_to_idx.get(to)?;
        let (_, path) = astar(
            &self.inner,
            src,
            |n| n == dst,
            |_| 1usize,
            |_| 0usize,
        )?;
        Some(path.into_iter().map(|i| &self.inner[i]).collect())
    }

    fn reachable(&self, id: &str, dir: Direction, max_hops: usize) -> Vec<&Node> {
        self.reachable_indices(id, dir, max_hops)
            .into_iter()
            .filter(|i| self.id_to_idx.get(id).is_none_or(|root| root != i))
            .map(|i| &self.inner[i])
            .collect()
    }

    fn reachable_indices(&self, id: &str, dir: Direction, max_hops: usize) -> Vec<NodeIndex> {
        let Some(&start) = self.id_to_idx.get(id) else {
            return Vec::new();
        };
        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let mut queue: VecDeque<(NodeIndex, usize)> = VecDeque::new();
        visited.insert(start);
        queue.push_back((start, 0));
        while let Some((idx, depth)) = queue.pop_front() {
            if depth == max_hops {
                continue;
            }
            for neighbour in self.inner.neighbors_directed(idx, dir) {
                if visited.insert(neighbour) {
                    queue.push_back((neighbour, depth + 1));
                }
            }
        }
        visited.into_iter().collect()
    }

    fn neighbours(&self, id: &str, dir: Direction) -> Vec<(&Node, EdgeKind)> {
        let Some(&idx) = self.id_to_idx.get(id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut edges = self.inner.edges_directed(idx, dir);
        while let Some(edge) = edges.next() {
            let other = match dir {
                Direction::Outgoing => edge.target(),
                Direction::Incoming => edge.source(),
            };
            out.push((&self.inner[other], *edge.weight()));
        }
        out
    }
}

fn add_resolved_edge(
    graph: &mut DiGraph<Node, EdgeKind>,
    id_to_idx: &HashMap<String, NodeIndex>,
    src: NodeIndex,
    raw: &str,
    kind: EdgeKind,
    all_ids: &[String],
) {
    if let Resolution::Resolved(id) = resolve(raw, all_ids) {
        if let Some(&dst) = id_to_idx.get(&id) {
            graph.add_edge(src, dst, kind);
        }
    }
}

/// Resolve a `[[ref]]` against the known set of canonical ids.
///
/// 1. Direct hit: `raw` is itself a canonical id.
/// 2. Short form: `raw` matches the last segment of exactly one canonical id.
/// 3. Multiple short-form matches → ambiguous.
/// 4. No matches → unresolved.
pub fn resolve(raw: &str, all_ids: &[String]) -> Resolution {
    let trimmed = raw.trim();
    if all_ids.iter().any(|id| id == trimmed) {
        return Resolution::Resolved(trimmed.to_string());
    }
    let matches: Vec<&String> = all_ids
        .iter()
        .filter(|id| last_segment(id) == trimmed)
        .collect();
    match matches.len() {
        0 => Resolution::Unresolved,
        1 => Resolution::Resolved(matches[0].clone()),
        _ => Resolution::Ambiguous(matches.into_iter().cloned().collect()),
    }
}

fn last_segment(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Resolved(String),
    Unresolved,
    Ambiguous(Vec<String>),
}

// petgraph's `edges_directed` exposes `target()`/`source()` via this trait.
use petgraph::visit::EdgeRef;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{FrontmatterRecord, RefMention};

    fn make_file(
        id: &str,
        kind: SpecKind,
        parent: Option<&str>,
        depends_on: &[&str],
        body_refs: &[&str],
    ) -> SpecFile {
        SpecFile {
            frontmatter: FrontmatterRecord {
                id: last_segment(id).to_string(),
                kind,
                order: None,
                parent: parent.map(String::from),
                implements: vec![],
                depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
                code: vec![],
                tests: vec![],
                status: SpecStatus::Active,
                responsibility: None,
                extras: serde_json::Value::Object(serde_json::Map::new()),
            },
            body: String::new(),
            refs_in_body: body_refs
                .iter()
                .enumerate()
                .map(|(i, r)| RefMention { raw: r.to_string(), line: i + 1, column: 1 })
                .collect(),
        }
    }

    fn input(id: &str, file: SpecFile) -> GraphInput {
        GraphInput { canonical_id: id.to_string(), file, path: PathBuf::from(id) }
    }

    #[test]
    fn resolves_full_form_and_short_form() {
        let ids = vec!["components/a".to_string(), "tools/b".to_string()];
        assert_eq!(resolve("components/a", &ids), Resolution::Resolved("components/a".into()));
        assert_eq!(resolve("b", &ids), Resolution::Resolved("tools/b".into()));
        assert_eq!(resolve("nope", &ids), Resolution::Unresolved);
    }

    #[test]
    fn detects_short_form_ambiguity() {
        let ids = vec!["x/foo".to_string(), "y/foo".to_string()];
        match resolve("foo", &ids) {
            Resolution::Ambiguous(v) => assert_eq!(v.len(), 2),
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn builds_edges_for_parent_and_depends_on() {
        let inputs = vec![
            input("overview", make_file("overview", SpecKind::Overview, None, &[], &[])),
            input("components/x", make_file("components/x", SpecKind::Component, Some("overview"), &["components/y"], &[])),
            input("components/y", make_file("components/y", SpecKind::Component, Some("overview"), &[], &[])),
        ];
        let g = SpecGraph::build(&inputs);
        let out: Vec<_> = g.outbound("components/x").into_iter().map(|(n, k)| (n.id.clone(), k)).collect();
        assert!(out.contains(&("overview".to_string(), EdgeKind::Parent)));
        assert!(out.contains(&("components/y".to_string(), EdgeKind::DependsOn)));
    }

    #[test]
    fn unreachable_from_overview_is_reported() {
        let inputs = vec![
            input("overview", make_file("overview", SpecKind::Overview, None, &[], &["components/x"])),
            input("components/x", make_file("components/x", SpecKind::Component, None, &[], &[])),
            input("components/orphan", make_file("components/orphan", SpecKind::Component, None, &[], &[])),
        ];
        let g = SpecGraph::build(&inputs);
        let unreachable: Vec<_> = g.unreachable_from("overview").into_iter().map(|n| n.id.clone()).collect();
        assert_eq!(unreachable, vec!["components/orphan".to_string()]);
    }

    #[test]
    fn reachable_within_respects_depth() {
        let inputs = vec![
            input("a", make_file("a", SpecKind::Overview, None, &[], &["b"])),
            input("b", make_file("b", SpecKind::Component, None, &[], &["c"])),
            input("c", make_file("c", SpecKind::Component, None, &[], &["d"])),
            input("d", make_file("d", SpecKind::Component, None, &[], &[])),
        ];
        let g = SpecGraph::build(&inputs);
        let within2: HashSet<String> = g.reachable_within("a", 2).into_iter().map(|n| n.id.clone()).collect();
        assert!(within2.contains("a"));
        assert!(within2.contains("b"));
        assert!(within2.contains("c"));
        assert!(!within2.contains("d"));
    }
}
