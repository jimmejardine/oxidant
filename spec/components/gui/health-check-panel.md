---
id: health-check-panel
kind: component
parent: overview
order: 6
implements: []
depends_on:
  - components/rust-tools/cargo-runner
  - components/spec-tools/validate
  - components/spec-tools/diff
code:
  - crates/oxidant-gui/src/panels/health_check.rs
status: active
responsibility: |
  Right-docked tree panel showing the workspace's CI-equivalent state — every check the project gates on, surfaced as a root node with a green ✔ if clean or a red ✗ with an auto-expanded subtree of issues. Replaces the earlier single-check Diagnostics panel.
---

## Checks

One root per `CheckKind`. v1 ships five — adding a check means one enum variant + one parser:

| CheckKind     | Tool invoked      | What counts as an issue                                                                                       |
|---------------|-------------------|---------------------------------------------------------------------------------------------------------------|
| CargoCheck    | `cargo_check`     | Every entry in the JSON `messages` array with `level: error | warning`.                                       |
| Clippy        | `cargo_clippy`    | Same compiler-message JSON shape as `cargo_check`; the parser is shared.                                      |
| Tests         | `cargo_test`      | Every entry in `failures: [...]`. Compile failures (`compile_messages`) also surface — they're real issues.   |
| SpecValidate  | `spec_validate`   | Every entry in `warnings: [{ kind, message, location: [path, line, col]? }]`.                                 |
| SpecDiff      | `spec_diff`       | Every entry in `drifts: [...]` — `MissingCodePath`, `MethodAdded`, `MethodRemoved`, `MethodSignatureChanged`. |

## Data model (in `app.rs`)

```rust
pub struct HealthReport {
    pub checks: BTreeMap<CheckKind, CheckState>,
    /// Wall-clock start of the most recent Run-all. Header shows "last run Xs ago".
    pub last_run_at: Option<std::time::Instant>,
}

pub enum CheckKind { CargoCheck, Clippy, Tests, SpecValidate, SpecDiff }

pub struct CheckState {
    pub status: CheckStatus,
    pub issues: Vec<HealthIssue>,
    pub finished_in_ms: u64,
    /// True once the user has manually toggled this root's collapsed
    /// state. Suppresses auto-expand on the NEXT red transition so the
    /// user's collapse decision sticks.
    pub user_toggled: bool,
}

pub enum CheckStatus { Idle, Running, Done, Failed(String) }

pub struct HealthIssue {
    pub check: CheckKind,
    pub severity: IssueSeverity,
    /// First non-empty piece of context: a file path, a spec id, a
    /// test name. Drives subtree grouping.
    pub group_key: String,
    pub message: String,
    /// Optional file location for click-to-open.
    pub file: Option<String>,
    pub line: u32,
    pub character: u32,
}

pub enum IssueSeverity { Error, Warning, Note }
```

## Run-all

The Refresh button becomes **Run all**. On click:

1. Snapshot the `ToolRegistry`, `workspace_root`, `exploration_id` from `SharedState`.
2. For each `CheckKind`, set `status = Running` and `tokio::spawn` a task that calls `registry.invoke(tool_name, json!({}), &ctx)`, parses the result, and writes back into `health.checks[kind]` with the parsed issues and `status = Done` (or `Failed(msg)` on tool error).
3. Each task is independent — they run in parallel. Each calls `egui_ctx.request_repaint()` when done so the panel updates live.
4. `last_run_at = Some(Instant::now())` is set at the start of the spawn so the header can render elapsed.

Run-all is disabled while any check is `Running`. Per-check refresh buttons are a v2 affordance.

## UI — tree

Header row (right-aligned action):
```
health check · 4 errors · 12 warnings · last run 14s ago    [Run all]
```

Body is a list of `egui::CollapsingHeader` roots, one per `CheckKind`. The header glyph is the only visual the user needs to scan:

| Glyph    | Condition                                       | Default open? |
|----------|-------------------------------------------------|----------------|
| ✔ green  | `Done` AND `issues.is_empty()`                  | collapsed       |
| ✗ red    | `Done` AND has issues, OR `Failed(_)`           | auto-expand on first red transition (unless `user_toggled`) |
| ⟳ spinner| `Running`                                        | collapsed       |
| · grey   | `Idle` (initial)                                 | collapsed       |

Root header text:
```
✗ clippy · 3 errors, 12 warnings (1.4s)
✔ cargo check · clean (0.8s)
⟳ tests · running…
✗ spec diff · 1 finding (0.2s)
```

### Subtree per check

Each red root expands into a check-specific subtree. Grouping is keyed on `HealthIssue.group_key`, picked at parse time to be the most useful first axis for that check:

- **CargoCheck / Clippy** — `group_key = file_path`. Issues under each file row sort errors before warnings, then by line. File rows with at least one error auto-expand; warnings-only file rows stay collapsed.
- **Tests** — `group_key = test_binary` (cargo target). Leaves are failing test names with the panic site as `file:line`.
- **SpecValidate** — `group_key = WarningKind`. Letting the user fix a whole class at once is more useful than per-spec grouping.
- **SpecDiff** — `group_key = Drift::kind` discriminant (one of the four variants). One leaf per finding.

Each leaf row shows `[severity] <message>` with the optional `file:line:character` rendered in muted text. Clicking a leaf with a file location pushes a `DockTab::File { path, source }` onto `pending_centre_tabs` — same flow the trees use.

### Failure isolation

If a check's tool returns `ToolResult::Err` or the spawned task panics, the root renders `✗ {kind} · failed: {message}` and does NOT auto-expand any subtree. The other roots are unaffected — one broken checker does not poison Run-all.

### Auto-expand bookkeeping

Auto-expand fires **only on the transition** from non-red to red AND when `user_toggled == false`. Tracked per-`CheckState`. Once the user explicitly collapses a red root, `user_toggled = true` and the panel respects that across subsequent Run-all cycles. No flapping.

## Out of scope for v1

- File-watcher–driven auto-run (notify → re-run on save).
- Auto-run after a Mutating-tool turn finishes — pre-existing "Diagnostics doesn't auto-refresh" gap, separable.
- A `cargo_fmt --check` source — no `cargo_fmt` tool yet.
- Quick-fix actions (click a clippy suggestion to apply it).
- Per-check refresh buttons.
- Persisting the report across restarts.
- Severity-filter or by-file-filter chips in the header.
