```yaml
---
id: spec-tree-panel
kind: component
parent: overview
order: 5
implements: []
depends_on:
  - components/spec-tools/index-db
  - components/spec-tools/graph
code:
  - crates/oxidant-gui/src/panels/spec_tree.rs
status: active
responsibility: |
  Left-docked tree view of spec/ organised by kind, with status/order ordering, recent-change badges, validate-warning indicators, and per-leaf Refs out / Refs in subtrees.
---
```

## Layout

```
spec/
├── overview                  (active)
├── glossary                  (active)
├── components/
│   ├── core/
│   │   ├── agent-loop        (active) ●
│   │   └── ...
│   └── ...
├── contracts/                ⚠
└── ...
```

## Ordering

Within each directory:
1. Direct child specs (leaves) first, sorted by `order:` ascending then alphabetical.
2. Sub-directories after, sorted alphabetically (BTreeMap iteration).

Why files-before-dirs: at `spec/` root this floats `overview` and `glossary` to the top — the natural "start here" reading order. The rule applies at every level so users don't have to learn one heuristic for the root and another for sub-directories.

## Badges

- `●` (filled dot): modified in the last 24h (from [[components/spec-tools/timeline]]).
- `⚠`: this subtree contains validate warnings (from [[components/spec-tools/validate]]).
- `(deprecated)`: status badge.

Tooltip on hover gives last-modified timestamp + commit subject.

## Interactions

- **Single-click**: preview the spec read-only in the [[components/gui/dock-layout]] **Selected** tab (a fast browse pane — content swaps in place, no new tab piles up). Double-click still opens an editable tab.
- **Double-click**: open the spec as an **editable** centre tab via [[components/gui/file-tabs]]. The tab dock-key is the spec's path, so double-clicking the same spec twice just focuses the already-open tab.
- **Right-click on a directory header**: context menu with **New spec** and **New folder**. Each opens a small modal dialog asking for the name; pressing Enter (or Create) makes the entry on disk under that directory. **New spec** additionally pushes the new path onto `SharedState::pending_centre_tabs` so the editor opens immediately. The dialog rejects empty names, names containing path separators, `.` / `..`, and names that already exist; the error renders inline above the input. New specs are created as empty files — the user adds the frontmatter — so they will show up as a validate warning until they grow a frontmatter block, which is the right default.
- **Right-click on a leaf**: context menu with:
  - **View history** — opens a read-only side-by-side diff viewer for the spec via [[components/gui/diff-history-panel]]. The tab is queued through `SharedState::pending_centre_tabs` the same way the double-click flow queues an editable File tab. See [[flows/view-spec-history]].
  - **Open in spec graph** — seeds [[components/gui/spec-graph-panel]] with this spec's `canonical_id` and opens the graph tab (or focuses it if already open). The previous graph state is discarded — the user is asking for a fresh exploration from this node. Implemented via the `pending_graph_seeds` queue described in the spec-graph panel doc.
  
  Further leaf actions (Reveal in code, Show drift) are deferred.
- **Expand a leaf** (the triangle): reveals its **Refs out** / **Refs in** subtrees — see below. Leaves with no associations render as plain rows with no expander.
- Drag onto a chat input: inserts the canonical ref as a `[[ref]]`.

New-item creation runs directly through `std::fs::create_dir` / `std::fs::File::create` on the GUI thread. The permission engine ([[components/config/permissions]]) doesn't gate it — these are explicit user actions, not agent-initiated tool calls. After a successful creation the panel invalidates its cached tree so the next frame's `walk_specs` picks up the new entry.

The double-click handler MUST NOT mutate the dock directly — the spec-tree panel is rendered inside `egui_dock`'s `TabViewer::ui`, which doesn't see the `DockState`. Instead it pushes a `DockTab::File { ..., source: Spec }` onto `SharedState::pending_centre_tabs`; the host viewport drains that queue after `DockArea::show` and inserts the tab via [[components/gui/dock-layout]]'s `open_in_centre` helper.

`open_in_centre` (NOT `open_tab`) is required because `push_to_focused_leaf` would put the new tab next to the spec tree on the left — the spec-tree leaf is what gained focus on the double-click. `open_in_centre` finds the leaf currently containing the `Transcript` tab and pushes there, so file tabs always dock alongside Transcript regardless of which side panel triggered the open.

## Refs subtrees

Each leaf with associations expands to two nested subtrees, backed by an in-memory [[components/spec-tools/graph]] built from the same `walk_specs` pass that builds the tree (cached, rebuilt on ⟳):

- **Refs out (N)** — what this spec points at: its outbound graph edges (`depends_on`, `implements`, `parent`, body `[[ref]]`), each row labelled with the edge kind and coloured by the target's kind, plus one `code → <file>` row per path in the spec's `code:` frontmatter.
- **Refs in (M)** — specs that point at this one (the inbound graph edges).

Rows under **Refs out** render as `<edge> → <target>`; rows under **Refs in** render as `<edge> ← <target>` — the arrow direction mirrors the section semantics so a glance tells the reader which way the relationship points. Word order stays the same (edge-then-target) in both directions so the columns line up.

Each ref row supports **single-click to preview** in the [[components/gui/dock-layout]] Selected tab and **double-click to open**: spec rows open the target spec ([[components/gui/file-tabs]], Spec source); `code:` rows open the source file (Code source). Leaves with no outbound edges, no `code:` files, and no inbound edges stay plain rows (no triangle), so the expander itself signals "has associations".

## Backing query

The panel reads from the SQLite index on every refresh (every ~500ms, cheap). Live updates via the same `notify` watcher driving the index.
