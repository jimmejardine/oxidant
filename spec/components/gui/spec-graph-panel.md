```yaml
id: spec-graph-panel
kind: component
parent: overview
order: 11
implements: []
depends_on:
  - components/spec-tools/graph
  - components/gui/dock-layout
  - components/gui/file-tabs
code:
  - crates/oxidant-gui/src/panels/spec_graph.rs
  - crates/oxidant-gui/src/graph_layout.rs
status: active
responsibility: |
  Interactive force-directed graph of specs, the code files that realise them, and the tests that prove them. Each graph is a centre-area `DockTab::SpecGraph { seed }` tab keyed by its starting node — multiple graphs can coexist, each seeded from a different right-click. Populates progressively — starts with the seed and grows when the user clicks per-node expand icons. Complements (does not replace) the spec-tree and file-tree panels.
```

## Data sources

The panel maintains two collections:

### Universe (built once on panel open and on `⟳` refresh)

A static lookup table containing every node and every edge that *could* appear:

- **Spec nodes + four edge kinds** — `Parent`, `Implements`, `DependsOn`, `BodyRef`. Pulled directly from `oxidant_spec_tools::graph::SpecGraph::build` (see [[components/spec-tools/graph]]). Conversion is the same `walk_specs → GraphInput` loop the validate flow uses.
- **CodeFile nodes + `RealisedBy` edges** — for each spec, every entry in `frontmatter.code` becomes a node (deduped on absolute path) and a spec→code edge.
- **Test nodes + `Tests` edges** — for each spec, every entry in `frontmatter.tests` becomes a node and a spec→test edge. `TestRef::Function { path, name }` is rendered as `{path}::{name}`; `TestRef::WholeFile { path }` as `{path}::*` (per decision 0011-specs-claim-their-tests).
- **NeighbourBuckets** per node, splitting the 1-hop neighbours into three categories: `specs` (the union of inbound + outbound across the four spec→spec edge kinds), `source` (outgoing `RealisedBy`), `tests` (outgoing `Tests`). This is what makes the expand-action O(1).

### External seeding from the trees — one tab per seed

No central Window-menu entry; every graph launches from a right-click in one of the trees. Each click opens a **new** `DockTab::SpecGraph { seed }` tab (de-dup on the seed via `DockTab::PartialEq`, so re-clicking focuses; two different seeds give two side-by-side tabs).

- Spec-tree → "Open in spec graph" → pushes `DockTab::SpecGraph { seed: <canonical_id> }` onto `pending_centre_tabs`.
- File-tree → same, with `seed = "code:{rel_path}"`. Files no spec claims render an empty canvas with a hint.

`SpecGraphPanel::new(workspace_root, seed)` builds the universe and inserts the seed as the only visible node (seed-protected so it can't be collapsed away).

### Visible subgraph (mutated by user expand/collapse)

Only the nodes the user has expanded their way to. Physics state lives here:

```rust
pub struct VisibleGraph {
    nodes: HashMap<NodeId, VisibleNode>,       // pos, vel, pinned
    edges: HashSet<(NodeId, NodeId, EdgeKind)>,
    refcounts: HashMap<NodeId, u32>,           // expansion-keep-alive count
    expanded: HashMap<NodeId, ExpandedFlags>,  // drives + / − icon state
}
```

## Progressive disclosure

- **Initial seed**: whatever id the `DockTab::SpecGraph { seed }` carries. Seed-protected: `refcount[seed] ≥ 1` so collapse never removes it.
- **Expand "+S" / "+C" / "+T" on node X** — look up `X`'s `NeighbourBuckets.{specs|source|tests}`. NEW (not-yet-visible) neighbours are placed on a ring around `X` (see "Spawn placement"); EXISTING ones keep their position. Either way `refcounts[Y]` is bumped. After positions settle, every touched node runs an edge-refresh that re-scans the universe edge list and inserts any edge whose endpoints are now both visible — that's what makes cross-edges between a new neighbour and a previously-existing visible node appear immediately. Flip `expanded[X]`.
- **Collapse "−S" / "−C" / "−T" on node X** — for each neighbour `Y` in the matching bucket: decrement `refcounts[Y]`; if 0 *and* `Y` is not seed-protected, drop `Y` and any edges to/from it. Flip `expanded[X]` back.

Refcounting keeps a node alive when it was reached via two expansion paths and only one is collapsed.

## Layout

Force-directed physics in `crates/oxidant-gui/src/graph_layout.rs`. Per `step(dt)`: per-node degree is computed once and feeds both repulsion (`F = k_pair / d²`, `k_pair` scaled by `deg_i + deg_j` so hubs push harder) and spring rest length (`rest_ij` scaled the same way, so dense areas get breathing room). Per-edge-kind `k_spring` is Parent-stiffest / BodyRef-slackest. Centre gravity keeps disconnected components on-canvas; damping `vel *= 0.85`; pinned nodes skip force integration and follow drag delta. The panel calls `step(dt)` once per frame and stops requesting repaint once kinetic energy drops below threshold.

### Spawn placement

N new neighbours around a parent at position `near` spawn evenly around a circle of radius `max(80, 50 * N / 2π)` — not piled at `near + tiny_offset`. Gives the simulation a head start and avoids zero-length-stub edges while physics separates the pile. Start angle is jittered per expansion so the same expansion twice doesn't stripe on the x-axis.

## Interactions

Hit-test priority: per-node expand chips (`±S` / `±C` / `±T`) first, then node body (square distance vs. radius).

- **Click on an expand chip**: run expand/collapse for that bucket.
- **Single-click on node body**: `selected = Some(id)`; focus-highlight node + visible 1-hop neighbours, dim everything else.
- **Double-click on node body**: same as the spec-tree double-click — push `DockTab::File { path, source }` onto `pending_centre_tabs` (see [[components/gui/dock-layout]]). Spec → `FileSource::Spec`; CodeFile → file-tree's `source_for`; Test → containing file.
- **Drag on node body**: `pinned = true`, position follows drag delta.
- **Right-click on node**: Open, View history ([[components/gui/diff-history-panel]]), Hide (force-collapse), Unpin / Reset position, Expand all.
- **Right-click on empty canvas**: Reset all pins, Centre view, Collapse all to seed(s).

## Per-node expand chips

Each visible node draws up to three small chip-buttons below its label:

```
   ┌─────────────┐
   │ ● file-tabs │     ← node body, kind colour
   └─────────────┘
     [±S] [±C] [±T]    ← only present when the bucket is non-empty
```

- `S` toggles all spec neighbours (the most useful single action; splitting per-edge-kind is deferred to v2 — would land in the right-click menu first).
- `C` toggles code-file children (this spec's `code:` paths).
- `T` toggles test children (this spec's `tests:` entries).
- A chip is **hidden** when the corresponding bucket is empty — a `Decision` spec with no code shows just `[±S]`; a CodeFile or Test node shows no chips at all (they're leaves in the spec-graph sense).
- Chip glyph: `+` when not expanded, `−` when expanded. Chip rects are hit-tested before node bodies.

## Edge type filters and node kinds

Header toggles control which edge kinds are *rendered* on visible nodes; they don't affect `VisibleGraph`. BodyRef is off by default — the body-ref noise dominates otherwise.

| Kind | Colour | Width | Default |
|---|---|---|---|
| Parent | blue | 2.0 | on |
| Implements | green | 2.0 | on |
| DependsOn | orange | 1.5 | on |
| BodyRef | faint grey | 1.0 | **off** |
| RealisedBy | cyan, dashed | 1.5 | on |
| Tests | dim green, dashed | 1.5 | on |

Node colours reuse the spec-tree palette (`kind_color` in `panels/spec_tree.rs::render_leaf`); CodeFile and Test colours come from the file-tree tag palette.

## Camera and performance

Pan via middle-mouse / two-finger drag; zoom via scroll wheel around the cursor; "Fit view" frames every visible node. The graph stays small because every node is opt-in — target 60 fps for a 100-node subgraph on a 4-year-old laptop, naive O(n²) repulsion fine at this scale.

If a user manages to expand their way past 300 nodes (e.g. "Expand all" on the overview), the header flashes a one-time hint banner ("Lots of nodes — try `−` to collapse, or 'Collapse all to seeds'"). Quadtree-accelerated repulsion (Barnes-Hut) is recorded as a follow-up if real workloads start hitting that.
