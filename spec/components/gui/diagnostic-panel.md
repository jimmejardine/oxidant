---
id: diagnostic-panel
kind: component
parent: overview
order: 6
implements: []
depends_on:
  - components/rust-tools/cargo-runner
  - components/rust-tools/lsp
code:
  - crates/oxidant-gui/src/panels/diagnostic.rs
status: active
responsibility: |
  Right-docked panel showing the most recent cargo diagnostics and rust-analyzer diagnostics for the active centre tab, with click-to-navigate.
---

## Data sources

Two streams merged:
- **cargo diagnostics**: from the most recent `cargo check|build|clippy|test` invocation in this exploration. Persisted in memory; cleared when the agent runs a successful build with no diagnostics.
- **rust-analyzer push diagnostics**: from `textDocument/publishDiagnostics` notifications. Latest set per file.

## Rendering

Each diagnostic:
```
[E0308] mismatched types
  src/edit.rs:42:14  expected `&str`, found `String`
                     [Show snippet ▾]  [Apply suggestion]
```

- Click the location → opens the file as a centre tab at that line.
- "Show snippet" → expand to show ~5 lines around the diagnostic.
- "Apply suggestion" appears only when `suggestion.replacement` is present; clicks build a `WorkspaceEdit` and submit via [[tools/edit/apply-edits]].

## Filtering

Default: errors only. Toggles for warnings, lints, and help.

## Follow-active-tab

When the active centre tab is a file at `path`, the panel filters to diagnostics for that file. When the active tab is `Transcript`, the panel shows tree-wide diagnostics (errors first).
