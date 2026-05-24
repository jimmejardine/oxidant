---
id: file-tree-panel
kind: component
parent: overview
order: 10
implements: []
depends_on:
  - components/gui/dock-layout
  - components/gui/file-tabs
code:
  - crates/oxidant-gui/src/panels/file_tree.rs
status: active
responsibility: |
  Left-docked browser for the workspace filesystem. Lets the user navigate to any file under the workspace root and open it as an editable centre tab via double-click — the same flow [[components/gui/spec-tree-panel]] uses for specs.
---

## Layout

A scrollable tree rooted at the workspace root, rendered as `egui::CollapsingHeader` per directory. Sibling of [[components/gui/spec-tree-panel]] on the left side of the dock; both default to that left tab group.

## Filtering

Walks the filesystem on first paint and on Refresh. Excludes:
- `target/`, `.git/`, `node_modules/`, `dist/`, `build/` — build/VCS noise.
- Any path matching a `.gitignore` glob — the workspace's `ignore` crate walker already handles this.
- Files larger than 5 MiB — the editor isn't built for big binaries.
- Files with a binary content type (NUL bytes in the first 8 KiB).

The result is cached; ⟳ in the header rebuilds.

## Ordering

Within each directory:
1. Subdirectories alphabetically.
2. Then files alphabetically.

## Interactions

- Single-click: select (visual highlight; no tab opens). MVP renders as no-op.
- **Double-click on a file**: open it as a centre tab via the same pending-queue mechanism described in [[components/gui/spec-tree-panel]] — push a `DockTab::File { ..., source }` onto `SharedState::pending_centre_tabs`, the host viewport drains and `open_in_centre`'s it.
  - `source = FileSource::Spec` for `*.md` under `spec/`.
  - `source = FileSource::Code` for everything else.
- Double-click on a directory toggles it open/closed (free from `CollapsingHeader`).
- **Right-click on a directory header**: context menu with **New file** and **New directory**. Each opens the same modal dialog the spec tree uses ([[components/gui/spec-tree-panel]] documents the behaviour: name validation, error-inline render, Enter to create). **New file** pushes the created path onto `SharedState::pending_centre_tabs` so the editor opens immediately, sourced via the same `source_for` rule as double-click (spec markdown vs. code).
- **Right-click on a leaf**: context menu with:
  - **View history** — opens [[components/gui/diff-history-panel]] for the file. Same mechanism the spec tree uses; the only difference is the queued tab's `source` is `FileSource::Code` (unless the path happens to be a `*.md` under `spec/`, in which case `source_for` picks `FileSource::Spec`). See [[flows/view-spec-history]].
  - **Open in spec graph** — seeds [[components/gui/spec-graph-panel]] with the file's CodeFile node and opens the graph tab. The seed id is `"code:{rel_path}"` (forward-slashes, workspace-relative). If no spec's `code:` frontmatter claims this file, the universe has no matching node and the seed is a silent no-op — the graph keeps whatever was already there.
  
  Further leaf actions (Reveal in OS file manager, copy path, rename, delete) are deferred.

New-item creation runs directly through `std::fs::create_dir` / `std::fs::File::create` on the GUI thread, bypassing the permission engine — these are explicit user actions, not agent-initiated tool calls. After a successful creation the panel invalidates its cached tree.

## File-type marker

Each leaf displays a small kind tag before the filename so the user can scan by language:
- `.rs` → `[rs]`, cyan
- `.md` → `[md]`, orange (matches the spec tree's `contract` colour band)
- `.toml` → `[toml]`, faint
- `.json` / `.yml` / `.yaml` → `[data]`, faint
- everything else → no tag

Colours fall through `theme::muted_text()` and `theme::faint_text()` so they stay legible across the five shipped themes.

## Performance

The walk runs synchronously on the GUI thread for the MVP. For workspaces with >5 000 files this would block briefly; the threshold for promoting the walk to a tokio task is ~50 ms per refresh, which is well above what we expect for a Rust project.
