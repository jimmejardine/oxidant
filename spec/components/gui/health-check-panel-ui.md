---
id: health-check-panel-ui
kind: component
parent: health-check-panel
order: 7
implements: []
depends_on:
  - components/gui/chat-input-panel
  - components/gui/spec-tree-panel
  - components/gui/file-tree-panel
code:
  - crates/oxidant-gui/src/panels/health_check.rs
status: active
responsibility: "UI rendering for the health-check panel: status glyphs per CheckKind, collapsible tree structure, subtree grouping, row interactions (click-to-open, double-click auto-fill), failure isolation, auto-expand bookkeeping, and leaf-row rendering."
---

## Glyphs and states

The panel renders a collapsible header row per check (`CheckKind`). Glyphs:

| Glyph    | Condition                                       | Default open? |
|----------|-------------------------------------------------|---------------|
| ✔ green  | `Done` AND `issues.is_empty()`                  | collapsed       |
| ✗ red    | `Done` AND has issues, OR `Failed(_)`           | auto-expand on first red transition |
| ⟳ spinner| `Running`                                        | collapsed       |
| · grey   | `Idle` (initial)                                 | collapsed       |

Root header text examples:

```
✗ clippy · 3 errors, 12 warnings (1.4s)
✔ cargo check · clean (0.8s)
⟳ tests · running…
✗ spec diff · 1 finding (0.2s)
```

## Failure isolation

If a check's tool returns `ToolResult::Err` or the spawned task panics, the root renders `✗ {kind} · failed: {message}` and does NOT auto-expand any subtree. The other roots are unaffected — one broken checker does not poison Run-all.

## Auto-expand bookkeeping

Auto-expand fires **only on the transition** from non-red to red AND when `user_toggled == false`. Tracked per-`CheckState`. Once the user explicitly collapses a red root, `user_toggled = true` and the panel respects that across subsequent Run-all cycles. No flapping.

## Subtree grouping

Each red root expands into a check-specific subtree. Grouping is keyed on `HealthIssue.group_key`, picked at parse time:

- **CargoCheck / Clippy** — `group_key = file_path`. Issues under each file row sort errors before warnings, then by line. File rows with at least one error auto-expand; warnings-only rows stay collapsed.
- **Tests** — `group_key = test_binary`. Leaves are failing test names with the panic site as `file:line`.
- **SpecValidate** — `group_key = WarningKind`. Letting the user fix a whole class at once is more useful than per-spec grouping.
- **SpecDiff** — `group_key = Drift::kind` discriminant. One leaf per finding.

## Row interactions

Every row (root, group header, leaf) shows `CursorIcon::PointingHand` on hover. Leaf rows carry two actions:

- **Single-click** opens the issue's file in a centre tab via `pending_centre_tabs`. No-op when `issue.file` is `None` (e.g. `SpecValidate::Orphan` without a source location).
- **Double-click** auto-fills the chat input with a structured prompt via `SharedState::pending_chat_prompt` (see [[components/gui/chat-input-panel]]), setting `mode = AgentMode::Plan`.

The prompt template rendered by `build_issue_prompt`:

```
Help me address this Health Check issue. Investigate first, then describe (don't make) the fix you'd apply.

Check:    clippy
Severity: warning
File:     crates/oxidant-gui/src/panels/spec_graph.rs:765:13
Group:    src/foo.rs
Message:  unused variable `near`
```

Fields omitted when empty: `File:` dropped when `issue.file` is `None`; `Group:` dropped when `group_key` equals the file path (duplicates `File:` line).

Forcing Plan mode is deliberate — double-clicking an issue is a "help me think" gesture. The user can flip to Implement after reading the proposal.

## Leaf rendering

Each leaf row is rendered as a single `egui::SelectableLabel` over a multi-segment `egui::text::LayoutJob` (severity tag in the severity colour, message in default text, location suffix in muted). This matches the pattern in [[components/gui/spec-tree-panel]] and [[components/gui/file-tree-panel]] — separate `Label` widgets each consume the hover for their own rect, making the pointer cursor land on the actual text rather than only on inter-label gaps.