---
id: open-file-from-tree
kind: flow
parent: overview
order: 11
status: active
responsibility: |
  Double-clicking a file in the spec tree or file tree opens it as a centre dock tab with an editable buffer. The tab tracks mtime so an external change (e.g. the agent editing the file behind the user's back) surfaces a reload banner.
depends_on:
  - components/gui/spec-tree-panel
  - components/gui/file-tree-panel
  - components/gui/file-tabs
  - components/gui/dock-layout
---

# Open a file from a tree panel into a centre tab

The user-facing path for reading + editing a file from inside oxidant. Distinct from the spec graph's read-only `spec_read` tool: this is the GUI affordance that produces an actual edit buffer.

## Trigger

A double-click on a leaf node inside either tree panel:

- [[components/gui/spec-tree-panel]] — reachable spec under `spec/`.
- [[components/gui/file-tree-panel]] — any non-excluded file under the worktree.

Both panels run inside left-docked leaves, so the file tab they want to open must end up in the *centre* dock area, not next to the tree they were just clicking in.

## Steps

1. **Resolve the absolute path.** The clicked tree node holds a relative path; the panel joins it with `workspace_root` and canonicalises (via `dunce::canonicalize` so Windows UNC paths don't surprise downstream code).

2. **Build the dock tab.** `DockTab::File { path, source }` where `source` is `FileSource::Spec` if the path lives under `spec/`, otherwise `FileSource::Code`. The variant carries source so the file-tab renderer can switch syntax highlighting and add spec-only affordances (e.g. an "open in spec graph" link in future iterations).

3. **Queue the centre-tab open.** The tree panel doesn't own the `DockState`, so it pushes the tab onto `SharedState.pending_centre_tabs` instead of inserting directly. (Inserting from inside the tree's panel render would put the new tab next to the tree, which is the wrong dock leaf — see [[components/gui/dock-layout]] for why centre placement is special.)

4. **Drain pending after `DockArea::show`.** The App's per-frame update runs the dock first, then takes `pending_centre_tabs` and calls `open_in_centre(dock, tab)` for each. `open_in_centre` finds the leaf currently holding the `Transcript` tab (the canonical centre leaf), falls back to any leaf already holding a `File` tab, then to the focused leaf as a last resort, and pushes the new tab there. If the tab is already open, it focuses the existing tab instead of duplicating.

5. **File tab renders.** [[components/gui/file-tabs]] takes over:
   - Loads the file into `SharedState.editor_buffers[path]` if not already loaded; records `mtime_at_load`.
   - Renders the contents with syntect-driven syntax highlighting matched on extension (or, for files with no extension like `.gitignore`, by token).
   - Marks the buffer `dirty` on any edit.

6. **Save (Ctrl+S).** The panel writes the buffer to disk and refreshes `mtime_at_load`. For `.rs` files, the substrate's syn-parse guard ([[invariants/rust-files-parse-after-edit]]) runs on save so a syntactically broken file can't replace a valid one on disk.

7. **External change banner.** Each render, the panel compares the current filesystem mtime against `mtime_at_load`. Mismatch → a "file changed on disk" banner with "Reload" / "Keep my version" actions. This is how the user notices the agent edited a file underneath the tab during a turn.

## Why centre placement is special

The dock has three left/right/bottom leaves around a centre area. The trees live in the left leaf; clicking inside the tree's panel leaves the *left* leaf focused. egui_dock's `push_to_focused_leaf` would therefore insert the file tab next to the tree, which is the wrong place — file tabs should appear next to Transcript in the centre. `open_in_centre` exists exactly for this case and is the only correct insertion path for tree-driven opens.

## Edge cases

- **File too large to load.** The tree panels filter out files above `MAX_FILE_BYTES` during walk; an oversized file simply isn't shown. If one slipped through (race with an external write), the file-tab renderer shows a "file too large to preview" placeholder instead of attempting to load.
- **Binary file.** `looks_binary(path)` (heuristic: NUL byte in the first chunk) excludes from the tree walk. Same fallback as oversize if encountered post-walk.
- **Path with non-UTF-8 segments.** Tree panels skip these entirely; oxidant assumes UTF-8 paths throughout (see `camino::Utf8PathBuf` in `ToolContext`).

## See also

- [[components/gui/file-tabs]] — the file-tab renderer
- [[components/gui/dock-layout]] — why `open_in_centre` exists
- [[components/gui/spec-tree-panel]] / [[components/gui/file-tree-panel]] — the trigger surfaces
