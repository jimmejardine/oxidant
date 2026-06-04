```yaml
---
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
---
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

There is no central Window-menu entry for the spec graph; every graph is launched from a right-click in one of the trees. Each click opens a **new** `DockTab::SpecGraph { seed }` tab — clicking the same seed twice focuses the existing tab (de-dup falls out of `DockTab`'s `PartialEq` on the seed), but two different seeds give two different tabs side by side.

The spec-tree's right-click context menu carries an **Open in spec graph** item. Clicking it pushes `DockTab::SpecGraph { seed: <canonical_id> }` onto `pending_centre_tabs`; the host viewport drains and `open_in_centre`'s it.

The file-tree's right-click on a code file does the same with `seed = "code:{rel_path}"` (matching the CodeFile node format the universe builder uses). Files that no spec claims via `code:` produce no matching universe node — the graph renders an empty canvas with "no spec claims this file" as a hint. The menu item shows regardless.

`SpecGraphPanel::new(workspace_root, seed)` builds the universe and inserts the seed as the only visible node. The seed is marked seed-protected so the user can't accidentally collapse it to nothing.

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

- **Initial seed**: whatever id the `DockTab::SpecGraph { seed }` carries. The seed is marked seed-protected: `refcount[seed]` is held at ≥1 so collapse actions never remove it.
- **Expand "+S" / "+C" / "+T" on node X** — look up `X`'s `NeighbourBuckets.{specs|source|tests}`. Partition the bucket into NEW (not yet visible) and EXISTING (already in the visible set). NEW nodes are placed on a ring around `X` (see "Spawn placement" below); EXISTING nodes keep their current position. Either way, `refcounts[Y]` is bumped. After all positions settle, every touched node — both the freshly-added NEW set and `X` itself — runs an edge-refresh that re-scans the universe edge list and inserts any edge whose endpoints are now both visible. That refresh is what makes cross-edges between a new neighbour and a previously-existing visible node appear immediately; without it, those edges would only render after the user expanded the OTHER endpoint, which earlier revisions did and which looked exactly like a "missing edges" bug. Flip `expanded[X]` flag.
- **Collapse "−S" / "−C" / "−T" on node X** — for each neighbour `Y` in the matching bucket: decrement `refcounts[Y]`; if 0 *and* `Y` is not seed-protected, drop `Y` and any edges to/from it. Flip `expanded[X]` flag back.

Refcounting is the whole point — if a node has been pulled in via two different expansion paths, collapsing one path keeps it alive for the other.

## Layout

Force-directed physics in `crates/oxidant-gui/src/graph_layout.rs`. Per `step(dt)`:

- **Per-node degree** is computed once at the top of each step from the edge list. Used by both repulsion and spring forces below — hub nodes (high degree) need more room than leaves.
- **Repulsion** between every node pair: `F = k_pair / d²`, clamped at `d_min`. `k_pair = k_rep * (1 + degree_repulsion_alpha * (deg_i + deg_j))` — two hubs push each other apart harder than two leaves do.
- **Spring attraction** along each edge: `F = (d - rest_ij) * k_spring`. `rest_ij = rest_length * (1 + degree_rest_alpha * (deg_i + deg_j))` — edges between hubs settle longer, so dense areas of the graph get visible breathing room. Per-edge-kind `k_spring` unchanged: Parent stiffest, BodyRef slackest.
- **Centre gravity**: `F = c * (pos - centre)` so disconnected components don't drift off-canvas.
- **Damping**: `vel *= 0.85` each step.
- **Pinned nodes** (dragged by user) skip force integration; pos comes from drag delta.
- The panel calls `step(dt)` once per frame; once kinetic energy drops below a threshold, `step` is skipped and the panel stops requesting repaint.

### Spawn placement

When `expand` adds N new neighbours around a parent at position `near`, they spawn evenly around a circle of radius `max(80, 50 * N / 2π)` rather than piled at `near + tiny_offset`. This both:
- Gives the simulation a head start — nodes already have plausible positions instead of converging from coincidence.
- Makes the newly-rendered edges immediately visible — a tight pile-up at spawn would make new edges look like zero-length stubs until the physics separated the nodes, which is what made earlier revisions look like edges weren't being added at all on subsequent expansions.

The spawn ring's start angle is jittered per expansion to avoid all-nodes-on-the-x-axis stripes when the same expansion runs twice.

## Interactions

Hit-test priority, topmost first:
1. Per-node expand chips (`+S` / `−S`, `+C` / `−C`, `+T` / `−T`)
2. Node body (square distance to centre, against radius)

Dispatch:
- **Click on an expand chip**: run the expand or collapse algorithm for that bucket.
- **Single-click on node body**: `selected = Some(id)`. Focus highlight on that node and its currently-visible 1-hop neighbours; other nodes/edges dim.
- **Double-click on node body**: same as the spec-tree double-click — push a `DockTab::File { path, source }` onto `SharedState::pending_centre_tabs`. The host viewport drains and `open_in_centre`'s the tab (see [[components/gui/dock-layout]]). Spec nodes route as `FileSource::Spec`, CodeFile nodes route via the same `source_for` rule the file-tree uses, Test nodes open the containing file.
- **Drag on node body**: `pinned = true`, position updates per frame to match drag delta.
- **Right-click on node**: context menu — Open, View history ([[components/gui/diff-history-panel]]), Hide (force-collapse: refcount → 0 regardless), Unpin / Reset position, Expand all (specs + src + tests at once).
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

Filter toggles in the header control which edge kinds are *rendered* on visible nodes — they do not affect what's in `VisibleGraph`. BodyRef edges are off by default; with them on, the body-ref noise often dominates the screen.

Edge colours (drawn via `egui::Painter::line_segment`):

| Kind         | Colour                 | Width | Default |
|--------------|------------------------|-------|---------|
| Parent       | blue                   | 2.0   | on      |
| Implements   | green                  | 2.0   | on      |
| DependsOn    | orange                 | 1.5   | on      |
| BodyRef      | faint grey             | 1.0   | **off** |
| RealisedBy   | cyan, dashed           | 1.5   | on      |
| Tests        | dim green, dashed      | 1.5   | on      |

Node colours reuse the spec-tree palette (`kind_color` match in `panels/spec_tree.rs::render_leaf`) so a Component is the same green in both panels. CodeFile and Test colours come from the file-tree tag palette.

## Camera

- Pan: middle-mouse drag, or two-finger trackpad drag.
- Zoom: scroll wheel around the cursor position.
- "Fit view" button frames all currently-visible nodes.

## Performance

The graph stays small because every node is opt-in. Target: render and animate a 100-node visible subgraph at 60 fps on a 4-year-old laptop. Naive O(n²) repulsion is fine at this scale.

If a user manages to expand their way past 300 nodes (e.g. "Expand all" on the overview), the header flashes a one-time hint banner ("Lots of nodes — try `−` to collapse, or 'Collapse all to seeds'"). Quadtree-accelerated repulsion (Barnes-Hut) is recorded as a follow-up if real workloads start hitting that.
