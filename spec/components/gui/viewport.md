---
id: viewport
kind: component
parent: overview
order: 1
implements: []
depends_on:
  - components/core/exploration
  - components/gui/dock-layout
code:
  - crates/oxidant-gui/src/viewport.rs
status: active
responsibility: |
  Manage one OS window per exploration via eframe's multi-viewport API; coordinate window lifecycle, title bar, and dock layout reset.
---

One OS window = one exploration. The viewport component is the eframe-side glue.

## Multi-viewport implementation

- Main exploration opens with `eframe::run_native(...)` on the primary viewport.
- Each sub-exploration: `ctx.show_viewport_deferred(...)` from any open viewport. The deferred variant runs the update closure in parallel with the spawner — non-blocking.
- Viewport ID = exploration ID (typed as `egui::ViewportId`).

## Title bar

Format:
- Main: `"oxidant — <repo-name> (main)"`
- Sub: `"oxidant — <repo-name> [sub: <branch-slug>]"`
- Cancelled task indicator added when present.

Rendered by setting `ViewportBuilder::with_title` and updating via `egui_ctx.send_viewport_cmd(SetTitle)` when the exploration's state changes.

## Lifecycle handling

- Window close on **main**: confirm if other explorations are open; then quit the app.
- Window close on **sub**: end the session for that exploration; LSP and agent loop shut down. Worktree and transcript persist on disk.
- Re-open a closed sub: from the exploration list ([[components/gui/exploration-list]]).

## Resource budget

Each open viewport draws an egui canvas at vsync. Repaint requests come from `ChatEvent` arrivals (see [[components/gui/transcript-tab]]); idle viewports do not redraw. 10 open viewports idle = negligible CPU.
