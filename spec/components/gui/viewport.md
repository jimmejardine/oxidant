```yaml
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
```

One OS window = one exploration. The viewport component is the eframe-side glue.

## Multi-viewport implementation

- Main exploration opens with `eframe::run_native(...)` on the primary viewport.
- Each sub-exploration: `ctx.show_viewport_deferred(...)` from the main viewport's `App::update`. The deferred variant runs the update closure when egui needs to redraw the sub-viewport — non-blocking.
- Viewport ID = `egui::ViewportId::from_hash_of(exploration_id)`.
- The double-click flow: exploration-list pushes the id onto `SharedState.pending_open_windows`; `App::update` drains and inserts an `Arc<Mutex<SubWindow>>` into `App.sub_windows`; the per-frame iteration re-registers each entry with `show_viewport_deferred`. Duplicate opens are no-ops (entry already present).
- Sub-viewport close: the closure checks `viewport_ctx.input(|i| i.viewport().close_requested())` and pushes the id onto `SharedState.pending_close_windows`. The next `App::update` drains and removes the matching entry from `sub_windows`.

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

## GPU load readout

The top bar shows GPU utilisation + VRAM (e.g. `GPU 42% · 3.1/24.0 GB`) via a small `GpuMonitor`
abstraction (`crates/oxidant-gui/src/gpu.rs`). The only backend today is **NVIDIA NVML**
(`nvml-wrapper`), which loads the NVML library at runtime — so the readout is simply **absent** when
NVML/NVIDIA isn't present (non-NVIDIA GPUs, no driver, CI). The abstraction (`GpuBackend` trait) is
the seam for future vendor/OS backends (Windows PDH for any vendor, Linux AMD sysfs, macOS IOReport)
without touching the call site. Sampled ~1 Hz.

## Resource budget

Each open viewport draws an egui canvas at vsync. Repaint requests come from `ChatEvent` arrivals (see [[components/gui/transcript-tab]]); idle viewports do not redraw — **except** when the GPU readout is active, which requests a repaint ~1×/sec to refresh the number (negligible). 10 open viewports idle = negligible CPU.
