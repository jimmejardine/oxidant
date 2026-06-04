```yaml
---
id: 0003-egui-gui-over-tui
kind: decision
order: 3
status: active
date: 2026-05-21
responsibility: |
  Use egui+eframe (with egui_dock) for a native desktop GUI rather than a ratatui-based TUI.
---
```

# 0003 — egui+eframe desktop GUI, not a TUI

## Status

Active.

## Context

A code agent's UI carries dense, structured content: streaming markdown, syntax-highlighted code, diff views, multi-pane workspaces, tool-call cards, dockable file tabs. Terminal UIs (ratatui) can do a lot, but Unicode-cell rendering of diffs, multi-tab dock layouts, and embedded markdown push against the medium.

A native GUI via egui/eframe gets us: arbitrary fonts, real syntax highlighting via `syntect`, real markdown rendering via `egui_commonmark`, free drag/resize/dock semantics, multi-viewport (multi-OS-window) support, and cross-platform parity via wgpu. The cost is binary size, RAM, and compile time.

## Decision

GUI via `egui` + `eframe`, dock management via `egui_dock`, one OS window per exploration. Markdown via `egui_commonmark`, syntax highlighting via `syntect`.

See [[components/gui/dock-layout]] and [[components/gui/viewport]].

## Consequences

Positive: visual fidelity that suits the content; idiomatic IDE-like dock UX; multi-window for multi-monitor setups.

Negative: heavier than a TUI (~50MB binary vs ~5MB, ~200MB RAM idle vs ~30MB). Slower cold compile. Mitigated by `cargo` profile tuning (see [[decisions/0008-spec-is-canonical]] open-risk notes).
