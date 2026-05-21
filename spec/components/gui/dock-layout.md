---
id: dock-layout
kind: component
parent: overview
order: 2
implements: []
depends_on:
  - components/gui/viewport
code:
  - crates/oxidant-gui/src/dock.rs
status: active
responsibility: |
  Build, persist, and restore the egui_dock dock tree inside an exploration's viewport, including default layout and "reset layout".
---

The dock manager lives via `egui_dock::DockArea`. Each exploration's viewport owns a `DockState<DockTab>` tree.

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
