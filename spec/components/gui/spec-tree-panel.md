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
  Left-docked tree view of spec/ organised by kind, with status/order ordering, recent-change badges, and validate-warning indicators.
---

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
1. Specs with explicit `order:` ascending.
2. Then alphabetical.

Directory groups follow the same convention.

## Badges

- `●` (filled dot): modified in the last 24h (from [[components/spec-tools/timeline]]).
- `⚠`: this subtree contains validate warnings (from [[components/spec-tools/validate]]).
- `(deprecated)`: status badge.

Tooltip on hover gives last-modified timestamp + commit subject.

## Interactions

- Single-click: select the leaf (visual highlight; no tab opens). MVP renders this as no-op — selection state lands when the right-click context menu does.
- **Double-click**: open the spec as an **editable** centre tab via [[components/gui/file-tabs]]. The tab dock-key is the spec's path, so double-clicking the same spec twice just focuses the already-open tab.
- **Right-click on a directory header**: context menu with **New spec** and **New folder**. Each opens a small modal dialog asking for the name; pressing Enter (or Create) makes the entry on disk under that directory. **New spec** additionally pushes the new path onto `SharedState::pending_centre_tabs` so the editor opens immediately. The dialog rejects empty names, names containing path separators, `.` / `..`, and names that already exist; the error renders inline above the input. New specs are created as empty files — the user adds the frontmatter — so they will show up as a validate warning until they grow a frontmatter block, which is the right default.
- Right-click on a leaf: deferred (Reveal in code, Show inbound refs, Show outbound refs, Show drift).
- Drag onto a chat input: inserts the canonical ref as a `[[ref]]`.

New-item creation runs directly through `std::fs::create_dir` / `std::fs::File::create` on the GUI thread. The permission engine ([[components/config/permissions]]) doesn't gate it — these are explicit user actions, not agent-initiated tool calls. After a successful creation the panel invalidates its cached tree so the next frame's `walk_specs` picks up the new entry.

The double-click handler MUST NOT mutate the dock directly — the spec-tree panel is rendered inside `egui_dock`'s `TabViewer::ui`, which doesn't see the `DockState`. Instead it pushes a `DockTab::File { ..., source: Spec }` onto `SharedState::pending_centre_tabs`; the host viewport drains that queue after `DockArea::show` and inserts the tab via [[components/gui/dock-layout]]'s `open_in_centre` helper.

`open_in_centre` (NOT `open_tab`) is required because `push_to_focused_leaf` would put the new tab next to the spec tree on the left — the spec-tree leaf is what gained focus on the double-click. `open_in_centre` finds the leaf currently containing the `Transcript` tab and pushes there, so file tabs always dock alongside Transcript regardless of which side panel triggered the open.

## Backing query

The panel reads from the SQLite index on every refresh (every ~500ms, cheap). Live updates via the same `notify` watcher driving the index.
