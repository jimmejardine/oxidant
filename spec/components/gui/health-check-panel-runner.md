```yaml
---
id: health-check-panel-runner
kind: component
parent: health-check-panel
order: 8
implements: []
depends_on:
  - components/rust-tools/cargo-runner
  - components/spec-tools/validate
  - components/spec-tools/diff
code:
  - crates/oxidant-gui/src/panels/health_check.rs
status: active
responsibility: "Run execution for the health-check panel: Run-all flow, per-row run dispatch via spawn_check, last_run_at semantics, and disabled-while-running state."
---
```

## Run-all flow

The Refresh button becomes **Run all**. On click:

1. Snapshot the `ToolRegistry`, `workspace_root`, `exploration_id` from `SharedState`.
2. For each `CheckKind`, set `status = Running` and `tokio::spawn` a task that calls `registry.invoke(tool_name, json!({}), &ctx)`, parses the result, and writes back into `health.checks[kind]` with the parsed issues and `status = Done` (or `Failed(msg)` on tool error).
3. Each task is independent — they run in parallel. Each calls `egui_ctx.request_repaint()` when done so the panel updates live.
4. `last_run_at = Some(Instant::now())` is set at the start of the spawn so the header can render elapsed.

Run-all is disabled while any check is `Running`.

## Per-row run

Each root carries a leading ▶ button (immediately to the left of the status glyph) that kicks off only that check. While the check is `Running` the button switches to a disabled ⟳ so the user can't double-fire. The dispatch helper is shared with Run-all: a private `spawn_check(state, tokio_handle, workspace_root, egui_ctx, kind)` marks the single `checks[kind]` entry as `Running`, then spawns the same `invoke → parse → write-back → request_repaint` future Run-all uses. Run-all loops over `ALL_CHECKS` calling `spawn_check`.

`last_run_at` (the "last run Xs ago" header) only updates on Run-all — it refers to the most recent *batch* run. Per-row runs don't touch it.