---
id: dock-layout
kind: component
parent: overview
order: 2
implements: []
depends_on: []
code:
  - crates/oxidant-gui/src/dock.rs
status: active
responsibility: |
  Build, persist, and restore the egui_dock dock tree, including default layout and "reset layout". The host viewport (see [[components/gui/viewport]]) owns this state; dock-layout itself is layering-neutral.
---

The dock manager lives via `egui_dock::DockArea`. Each exploration's viewport owns a `DockState<DockTab>` tree — the dependency direction is viewport → dock-layout, deliberately not the reverse.

## Default layout (per viewport)

```
LEFT:    [spec_tree, exploration_list, validate_warnings]    (tab group)
CENTRE:  [transcript, ...opened_files]                       (tab group; transcript is the home tab)
RIGHT:   [diagnostic_preview]
BOTTOM:  [chat_input]
```

## DockTab enum

```rust
pub enum DockTab {
    Transcript,
    SpecTree,
    ExplorationList,
    ValidateWarnings,
    DiagnosticPreview,
    ChatInput,
    File { path: PathBuf, source: FileSource },   // opened code or spec file
}
```

## Persistence

- The dock tree is serialised to `<worktree>/.oxidant/dock-layout.json` per exploration.
- Loaded on viewport open; written on close and on every dock change (debounced 500ms).
- Schema version field; mismatched versions reset to default.

## Window menu

The viewport's top menu bar carries a **Window** menu so a user can recover from closing a panel they later want back. Contents:

- One entry per **singleton** tab — `Transcript`, `Specs`, `Explorations`, `Diagnostics`, `Chat`. Each shows a checkmark when the tab is already open and is disabled in that state; clicking an unchecked entry re-inserts the tab into the focused leaf (or the first leaf if nothing is focused).
- **Reset layout** at the bottom of the menu rebuilds the default layout (see below). File tabs that are currently open are preserved as centre tabs — closing a file is still done via the `×` on the tab itself.

File tabs (`DockTab::File { … }`) are not listed in the Window menu — they are opened from the spec tree or by following a navigation result. A recent-files history is out of scope for the MVP.

## Reset layout

A menu command rebuilds the default layout and clears `dock-layout.json`. Opened files are preserved as centre tabs.

## Tab content delegation

Each `DockTab` variant's render is delegated to its panel component:
- `Transcript` → [[components/gui/transcript-tab]]
- `SpecTree` → [[components/gui/spec-tree-panel]]
- `ExplorationList` → [[components/gui/exploration-list]]
- `ValidateWarnings` → handled inline (small surface)
- `DiagnosticPreview` → [[components/gui/diagnostic-panel]]
- `ChatInput` → [[components/gui/chat-input-panel]]
- `File` → [[components/gui/file-tabs]]
