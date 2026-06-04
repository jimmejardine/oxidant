```yaml
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
  - health-check-panel-ui
  - health-check-panel-runner
code:
  - crates/oxidant-gui/src/panels/health_check.rs
status: active
responsibility: "Right-docked tree panel showing the workspace's CI-equivalent state — every check the project gates on, surfaced as a root node with a green ✔ if clean or a red ✗ with an auto-expanded subtree of issues. Replaces the earlier single-check Diagnostics panel."
---
```

## Checks

One root per `CheckKind`. v1 ships six — adding a check means one enum variant + one parser:

| CheckKind     | Tool invoked      | What counts as an issue                                                                                       |
|---------------|-------------------|---------------------------------------------------------------------------------------------------------------|
| CargoCheck    | `cargo_check`     | Every entry in the JSON `messages` array with `level: error | warning`.                                       |
| Clippy        | `cargo_clippy`    | Same compiler-message JSON shape as `cargo_check`; the parser is shared.                                      |
| Tests         | `cargo_test`      | Every entry in `failures: [...]`. Compile failures (`compile_messages`) also surface — they're real issues.   |
| SpecValidate  | `spec_validate`   | Every entry in `warnings: [{ kind, message, location: [path, line, col]? }]`.                                 |
| SpecDiff      | `spec_diff`       | Every entry in `drifts: [...]` — `MissingCodePath`, `MethodAdded`, `MethodRemoved`, `MethodSignatureChanged`. |
| SpecCoverage  | `spec_coverage`   | Every entry in `uncovered: [{ file, krate }]` (note, grouped by crate) + each `missing_seeds` path (warning). See [[tools/spec/spec-coverage]] / [[components/spec-tools/coverage]]. |

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
    /// True once the user has manually toggled this root's collapsed state.
    /// Suppresses auto-expand on the NEXT red transition so the user's collapse
    /// decision sticks.
    pub user_toggled: bool,
}

pub enum CheckStatus { Idle, Running, Done, Failed(String) }

pub struct HealthIssue {
    pub check: CheckKind,
    pub severity: IssueSeverity,
    /// First non-empty piece of context: a file path, a spec id, a test name.
    /// Drives subtree grouping.
    pub group_key: String,
    pub message: String,
    /// Optional file location for click-to-open.
    pub file: Option<String>,
    pub line: u32,
    pub character: u32,
}

pub enum IssueSeverity { Error, Warning, Note }
```

## Decomposition

The panel is split into three specs:

- **health-check-panel-ui** — UI rendering: glyphs per check, collapsible tree structure, subtree grouping, row interactions, failure isolation, auto-expand bookkeeping, and leaf-row rendering.
- **health-check-panel-runner** — run execution: Run-all flow, per-row run dispatch (`spawn_check`), `last_run_at` semantics, disabled-while-running state.

## Out of scope for v1

- File-watcher–driven auto-run (notify → re-run on save).
- Auto-run after a Mutating-tool turn finishes — pre-existing "Diagnostics doesn't auto-refresh" gap, separable.
- A `cargo_fmt --check` source — no `cargo_fmt` tool yet.
- Quick-fix actions (click a clippy suggestion to apply it).
- Per-check refresh buttons.
- Persisting the report across restarts.
- Severity-filter or by-file-filter chips in the header.