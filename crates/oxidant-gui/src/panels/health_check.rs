// Realises spec/components/gui/health-check-panel.md.
//
// Tree-shaped CI-equivalent view: one root per CheckKind, green ✔ when
// clean, red ✗ with an auto-expanded subtree when something is wrong.
// Run-all spawns every check in parallel via the ToolRegistry; each
// task writes back into SharedState.health on completion.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Instant;

use egui::{Color32, RichText};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use oxidant_core::{ToolContext, ToolRegistry, ToolResult};

use crate::app::{
    CheckKind, CheckState, CheckStatus, HealthIssue, IssueSeverity, SharedState,
};
use crate::dock::{DockTab, FileSource};
use crate::theme;

pub struct HealthCheckPanel;

impl Default for HealthCheckPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthCheckPanel {
    pub fn new() -> Self {
        Self
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        state: &Arc<StdMutex<SharedState>>,
        tokio_handle: &Handle,
        workspace_root: &Path,
        egui_ctx: &egui::Context,
    ) {
        // Snapshot what we render this frame to keep the lock short.
        let (any_running, totals, last_run_at, snapshots) = {
            let s = state.lock().unwrap();
            let any_running = s
                .health
                .checks
                .values()
                .any(|c| matches!(c.status, CheckStatus::Running));
            let totals = roll_up_totals(&s.health);
            let last_run_at = s.health.last_run_at;
            let snapshots: Vec<(CheckKind, CheckState)> = ALL_CHECKS
                .iter()
                .map(|k| {
                    let st = s
                        .health
                        .checks
                        .get(k)
                        .cloned()
                        .unwrap_or_default();
                    (*k, st)
                })
                .collect();
            (any_running, totals, last_run_at, snapshots)
        };

        // Header.
        ui.horizontal(|ui| {
            ui.label(RichText::new("health check").strong());
            ui.label(
                RichText::new(format!(
                    "· {} error{}, {} warning{}{}",
                    totals.errors,
                    if totals.errors == 1 { "" } else { "s" },
                    totals.warnings,
                    if totals.warnings == 1 { "" } else { "s" },
                    match last_run_at {
                        Some(t) => format!(" · last run {}s ago", t.elapsed().as_secs()),
                        None => String::new(),
                    }
                ))
                .color(theme::muted_text()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let resp = ui.add_enabled(!any_running, egui::Button::new("Run all"));
                if resp.clicked() {
                    spawn_run_all(state, tokio_handle, workspace_root, egui_ctx);
                }
            });
        });
        ui.separator();

        // Body — one root per check.
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for (kind, st) in snapshots {
                    render_root(ui, state, kind, &st);
                    ui.add_space(2.0);
                }
            });
    }
}

const ALL_CHECKS: [CheckKind; 5] = [
    CheckKind::CargoCheck,
    CheckKind::Clippy,
    CheckKind::Tests,
    CheckKind::SpecValidate,
    CheckKind::SpecDiff,
];

#[derive(Default, Clone, Copy)]
struct Totals {
    errors: usize,
    warnings: usize,
}

fn roll_up_totals(report: &crate::app::HealthReport) -> Totals {
    let mut t = Totals::default();
    for st in report.checks.values() {
        for issue in &st.issues {
            match issue.severity {
                IssueSeverity::Error => t.errors += 1,
                IssueSeverity::Warning => t.warnings += 1,
                IssueSeverity::Note => {}
            }
        }
    }
    t
}

// ---------------------------------------------------------------- root rendering

fn render_root(
    ui: &mut egui::Ui,
    state: &Arc<StdMutex<SharedState>>,
    kind: CheckKind,
    st: &CheckState,
) {
    let (glyph, glyph_colour, default_open) = glyph_for(st);
    let header_text = root_header_text(kind, st);

    let id = ui.make_persistent_id(("health_root", kind.as_str()));

    // Auto-expand on first red transition. Once the user toggles, the
    // CheckState carries user_toggled=true and we respect it.
    let header_response = egui::CollapsingHeader::new(
        RichText::new(format!("{glyph}  {header_text}")).color(glyph_colour),
    )
    .id_salt(id)
    .default_open(default_open)
    .show(ui, |ui| render_subtree(ui, state, kind, st));

    // Detect user toggle. If the user collapsed/expanded a red root,
    // mark user_toggled so future runs don't fight them.
    if header_response.header_response.clicked() {
        let mut s = state.lock().unwrap();
        if let Some(entry) = s.health.checks.get_mut(&kind) {
            entry.user_toggled = true;
        }
    }
}

fn glyph_for(st: &CheckState) -> (&'static str, Color32, bool) {
    match &st.status {
        CheckStatus::Idle => ("·", theme::muted_text(), false),
        CheckStatus::Running => ("⟳", Color32::from_rgb(180, 200, 240), false),
        CheckStatus::Done => {
            if st.issues.is_empty() {
                ("✔", Color32::from_rgb(120, 220, 140), false)
            } else {
                ("✗", Color32::from_rgb(247, 118, 142), !st.user_toggled)
            }
        }
        CheckStatus::Failed(_) => ("✗", Color32::from_rgb(247, 118, 142), !st.user_toggled),
    }
}

fn root_header_text(kind: CheckKind, st: &CheckState) -> String {
    let label = kind.display_name();
    let elapsed = if st.finished_in_ms > 0 {
        format!(" ({:.1}s)", st.finished_in_ms as f32 / 1000.0)
    } else {
        String::new()
    };
    match &st.status {
        CheckStatus::Idle => format!("{label} · idle"),
        CheckStatus::Running => format!("{label} · running…"),
        CheckStatus::Done => {
            let errors = st
                .issues
                .iter()
                .filter(|i| matches!(i.severity, IssueSeverity::Error))
                .count();
            let warnings = st
                .issues
                .iter()
                .filter(|i| matches!(i.severity, IssueSeverity::Warning))
                .count();
            if errors == 0 && warnings == 0 {
                format!("{label} · clean{elapsed}")
            } else {
                let mut parts = Vec::new();
                if errors > 0 {
                    parts.push(format!("{errors} error{}", if errors == 1 { "" } else { "s" }));
                }
                if warnings > 0 {
                    parts.push(format!(
                        "{warnings} warning{}",
                        if warnings == 1 { "" } else { "s" }
                    ));
                }
                format!("{label} · {}{elapsed}", parts.join(", "))
            }
        }
        CheckStatus::Failed(msg) => format!("{label} · failed: {msg}{elapsed}"),
    }
}

fn render_subtree(
    ui: &mut egui::Ui,
    state: &Arc<StdMutex<SharedState>>,
    _kind: CheckKind,
    st: &CheckState,
) {
    if let CheckStatus::Failed(msg) = &st.status {
        ui.label(
            RichText::new(format!("tool failed: {msg}")).color(ui.visuals().error_fg_color),
        );
        return;
    }
    if st.issues.is_empty() {
        ui.label(RichText::new("no issues").color(theme::muted_text()));
        return;
    }
    // Group by HealthIssue.group_key, sort each group by severity then file/line.
    let mut groups: BTreeMap<String, Vec<&HealthIssue>> = BTreeMap::new();
    for issue in &st.issues {
        groups.entry(issue.group_key.clone()).or_default().push(issue);
    }
    for (group_key, mut issues) in groups {
        issues.sort_by_key(|i| {
            (
                match i.severity {
                    IssueSeverity::Error => 0,
                    IssueSeverity::Warning => 1,
                    IssueSeverity::Note => 2,
                },
                i.line,
            )
        });
        let group_has_error = issues
            .iter()
            .any(|i| matches!(i.severity, IssueSeverity::Error));
        let header = format!(
            "{group_key} · {} issue{}",
            issues.len(),
            if issues.len() == 1 { "" } else { "s" }
        );
        egui::CollapsingHeader::new(RichText::new(header).color(theme::muted_text()))
            .id_salt(("health_group", group_key.clone()))
            .default_open(group_has_error)
            .show(ui, |ui| {
                for issue in issues {
                    render_issue(ui, state, issue);
                }
            });
    }
}

fn render_issue(ui: &mut egui::Ui, state: &Arc<StdMutex<SharedState>>, issue: &HealthIssue) {
    let sev_label = match issue.severity {
        IssueSeverity::Error => RichText::new("[error]").color(Color32::from_rgb(247, 118, 142)),
        IssueSeverity::Warning => RichText::new("[warn]").color(Color32::from_rgb(255, 198, 109)),
        IssueSeverity::Note => RichText::new("[note]").color(theme::muted_text()),
    };
    let resp = ui
        .horizontal(|ui| {
            ui.label(sev_label.strong());
            ui.label(&issue.message);
            if let Some(file) = &issue.file {
                ui.label(
                    RichText::new(format!("· {}:{}:{}", file, issue.line, issue.character))
                        .color(theme::muted_text()),
                );
            }
        })
        .response;
    // Click anywhere on the row to open the file (if any).
    if resp.interact(egui::Sense::click()).clicked()
        && let Some(file) = &issue.file
    {
        push_open(state, file);
    }
}

fn push_open(state: &Arc<StdMutex<SharedState>>, rel_or_abs_path: &str) {
    let path = std::path::PathBuf::from(rel_or_abs_path);
    let tab = DockTab::File {
        path,
        source: FileSource::Code,
    };
    if let Ok(mut s) = state.lock()
        && !s.pending_centre_tabs.contains(&tab)
    {
        s.pending_centre_tabs.push(tab);
    }
}

// ---------------------------------------------------------------- run-all dispatch

fn spawn_run_all(
    state: &Arc<StdMutex<SharedState>>,
    tokio_handle: &Handle,
    workspace_root: &Path,
    egui_ctx: &egui::Context,
) {
    // Snapshot what we'll need from shared state. Mark every check as
    // Running on the GUI thread before crossing into tokio.
    let (registry, exploration_id) = {
        let mut s = state.lock().unwrap();
        s.health.last_run_at = Some(Instant::now());
        for kind in ALL_CHECKS {
            let entry = s
                .health
                .checks
                .entry(kind)
                .or_insert_with(CheckState::default);
            entry.status = CheckStatus::Running;
            entry.issues.clear();
            entry.finished_in_ms = 0;
        }
        (s.registry.clone(), s.exploration.id.to_string())
    };
    let workspace = workspace_root.to_path_buf();
    let egui_ctx = egui_ctx.clone();

    for kind in ALL_CHECKS {
        let state = state.clone();
        let registry = registry.clone();
        let workspace = workspace.clone();
        let exploration_id = exploration_id.clone();
        let egui_ctx = egui_ctx.clone();
        tokio_handle.spawn(async move {
            let started = Instant::now();
            let result = invoke_check(&registry, &workspace, &exploration_id, kind).await;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let (status, issues): (CheckStatus, Vec<HealthIssue>) = match result {
                Ok(value) => (CheckStatus::Done, parse_for(kind, &value)),
                Err(msg) => (CheckStatus::Failed(msg), Vec::new()),
            };
            if let Ok(mut s) = state.lock() {
                let entry = s
                    .health
                    .checks
                    .entry(kind)
                    .or_insert_with(CheckState::default);
                // Detect a new red transition so we can reset
                // user_toggled if the user fixed and broke again.
                let was_clean = matches!(entry.status, CheckStatus::Done) && entry.issues.is_empty();
                let now_red = !matches!(status, CheckStatus::Done) || !issues.is_empty();
                if was_clean && now_red {
                    entry.user_toggled = false;
                }
                entry.status = status;
                entry.issues = issues;
                entry.finished_in_ms = elapsed_ms;
            }
            egui_ctx.request_repaint();
        });
    }
}

async fn invoke_check(
    registry: &Arc<ToolRegistry>,
    workspace_root: &Path,
    exploration_id: &str,
    kind: CheckKind,
) -> Result<Value, String> {
    let canonical =
        dunce::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let workspace_camino = camino::Utf8PathBuf::from_path_buf(canonical.clone())
        .map_err(|_| format!("non-UTF-8 workspace path: {}", canonical.display()))?;
    let ctx = ToolContext {
        workspace_root: workspace_camino,
        exploration_id: exploration_id.to_string(),
        cancellation: CancellationToken::new(),
    };
    let tool_name = kind.tool_name();
    match registry.invoke(tool_name, serde_json::json!({}), &ctx).await {
        ToolResult::Ok(v) => Ok(v),
        ToolResult::Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------- parsers

pub(crate) fn parse_for(kind: CheckKind, value: &Value) -> Vec<HealthIssue> {
    match kind {
        CheckKind::CargoCheck | CheckKind::Clippy => parse_cargo_messages(kind, value),
        CheckKind::Tests => parse_test_failures(value),
        CheckKind::SpecValidate => parse_spec_validate(value),
        CheckKind::SpecDiff => parse_spec_diff(value),
    }
}

fn parse_cargo_messages(kind: CheckKind, value: &Value) -> Vec<HealthIssue> {
    let messages = match value.get("messages").and_then(|m| m.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for m in messages {
        let level = m.get("level").and_then(|v| v.as_str()).unwrap_or("info");
        let severity = match level {
            "error" => IssueSeverity::Error,
            "warning" => IssueSeverity::Warning,
            _ => continue, // skip "note", "help", "failure-note" etc. at the issue level
        };
        let message = m
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let span = m.get("spans").and_then(|s| s.as_array()).and_then(|spans| {
            spans
                .iter()
                .find(|s| {
                    s.get("is_primary")
                        .and_then(|p| p.as_bool())
                        .unwrap_or(false)
                })
                .or_else(|| spans.first())
        });
        let (file, line, character) = match span {
            Some(s) => (
                s.get("file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                s.get("start")
                    .and_then(|p| p.get("line"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                s.get("start")
                    .and_then(|p| p.get("character"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
            ),
            None => (String::new(), 0, 0),
        };
        let group_key = if file.is_empty() {
            "<no file>".to_string()
        } else {
            file.clone()
        };
        out.push(HealthIssue {
            check: kind,
            severity,
            group_key,
            message,
            file: if file.is_empty() { None } else { Some(file) },
            line,
            character,
        });
    }
    out
}

fn parse_test_failures(value: &Value) -> Vec<HealthIssue> {
    let mut out = Vec::new();
    // Compile-time errors land in compile_messages with the same shape
    // as cargo_check.
    if let Some(compile_msgs) = value.get("compile_messages") {
        let wrapped = serde_json::json!({ "messages": compile_msgs });
        let mut compile_issues = parse_cargo_messages(CheckKind::Tests, &wrapped);
        // Re-tag the group as "(compile)" so failing tests group
        // separately from compile errors.
        for issue in &mut compile_issues {
            issue.group_key = format!("(compile) {}", issue.group_key);
        }
        out.extend(compile_issues);
    }
    if let Some(failures) = value.get("failures").and_then(|f| f.as_array()) {
        for fail in failures {
            let test_name = fail
                .get("test")
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>")
                .to_string();
            // The libtest stdout often contains "thread '<name>' panicked at FILE:LINE:COL".
            let stdout = fail.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
            let (file, line) = extract_panic_location(stdout);
            // Group key = the test target prefix before "::", or "(unknown target)".
            let group_key = match test_name.find("::") {
                Some(i) => test_name[..i].to_string(),
                None => "(unknown target)".into(),
            };
            out.push(HealthIssue {
                check: CheckKind::Tests,
                severity: IssueSeverity::Error,
                group_key,
                message: test_name,
                file,
                line,
                character: 0,
            });
        }
    }
    out
}

fn extract_panic_location(stdout: &str) -> (Option<String>, u32) {
    // Match "panicked at FILE:LINE:COL" — first occurrence wins.
    let needle = "panicked at ";
    let idx = match stdout.find(needle) {
        Some(i) => i + needle.len(),
        None => return (None, 0),
    };
    // Pull until a whitespace, single quote, or close paren.
    let tail = &stdout[idx..];
    let end = tail
        .find(|c: char| c.is_whitespace() || c == '\'' || c == ',')
        .unwrap_or(tail.len());
    let token = tail[..end].trim_end_matches(':');
    // FILE:LINE:COL — split from the right.
    let parts: Vec<&str> = token.rsplitn(3, ':').collect();
    if parts.len() == 3 {
        let line = parts[1].parse::<u32>().unwrap_or(0);
        return (Some(parts[2].to_string()), line);
    }
    (None, 0)
}

fn parse_spec_validate(value: &Value) -> Vec<HealthIssue> {
    let warnings = match value.get("warnings").and_then(|w| w.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for w in warnings {
        let kind_str = w
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("Warning")
            .to_string();
        let message = w
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let (file, line, character) = match w.get("location").and_then(|l| l.as_array()) {
            Some(arr) if arr.len() >= 3 => {
                let path = arr[0].as_str().unwrap_or("").to_string();
                let line = arr[1].as_u64().unwrap_or(0) as u32;
                let col = arr[2].as_u64().unwrap_or(0) as u32;
                (Some(path), line, col)
            }
            _ => (None, 0, 0),
        };
        out.push(HealthIssue {
            check: CheckKind::SpecValidate,
            severity: IssueSeverity::Warning,
            group_key: kind_str,
            message,
            file,
            line,
            character,
        });
    }
    out
}

fn parse_spec_diff(value: &Value) -> Vec<HealthIssue> {
    let drifts = match value.get("drifts").and_then(|d| d.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for d in drifts {
        let kind_str = d
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("Drift")
            .to_string();
        // Pull whatever identifying fields exist; the four variants
        // serialise different shapes.
        let message = match kind_str.as_str() {
            "MissingCodePath" => format!(
                "{}: code path `{}` does not exist",
                d.get("spec_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<unknown>"),
                d.get("path").and_then(|v| v.as_str()).unwrap_or("?"),
            ),
            "MethodAdded" => format!(
                "{}: code adds method `{}` not in spec",
                d.get("contract_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<contract>"),
                d.get("method").and_then(|v| v.as_str()).unwrap_or("?"),
            ),
            "MethodRemoved" => format!(
                "{}: code removed method `{}` declared by spec",
                d.get("contract_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<contract>"),
                d.get("method").and_then(|v| v.as_str()).unwrap_or("?"),
            ),
            "MethodSignatureChanged" => format!(
                "{}::{} — spec declares {}, code has {}",
                d.get("contract_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<contract>"),
                d.get("method").and_then(|v| v.as_str()).unwrap_or("?"),
                d.get("spec").and_then(|v| v.as_str()).unwrap_or("?"),
                d.get("code").and_then(|v| v.as_str()).unwrap_or("?"),
            ),
            _ => serde_json::to_string(d).unwrap_or_default(),
        };
        let file = d
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        out.push(HealthIssue {
            check: CheckKind::SpecDiff,
            severity: IssueSeverity::Error,
            group_key: kind_str,
            message,
            file,
            line: 0,
            character: 0,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cargo_messages_picks_errors_and_warnings() {
        let v = serde_json::json!({
            "messages": [
                { "level": "error", "message": "mismatched types",
                  "spans": [{ "file": "src/a.rs", "start": { "line": 12, "character": 4 }, "is_primary": true }]
                },
                { "level": "warning", "message": "unused variable `x`",
                  "spans": [{ "file": "src/a.rs", "start": { "line": 30, "character": 8 }, "is_primary": true }]
                },
                { "level": "note", "message": "ignored" }
            ]
        });
        let issues = parse_cargo_messages(CheckKind::Clippy, &v);
        assert_eq!(issues.len(), 2, "note should be filtered out");
        assert!(matches!(issues[0].severity, IssueSeverity::Error));
        assert_eq!(issues[0].file.as_deref(), Some("src/a.rs"));
        assert_eq!(issues[0].group_key, "src/a.rs");
        assert!(matches!(issues[1].severity, IssueSeverity::Warning));
    }

    #[test]
    fn parse_test_failures_groups_by_target() {
        let v = serde_json::json!({
            "failures": [
                { "test": "oxidant_core::text_tool_calls::extract_qwen3_envelope",
                  "stdout": "thread 'oxidant_core::text_tool_calls::extract_qwen3_envelope' panicked at crates/oxidant-core/src/text_tool_calls.rs:301:9:\nbad" },
                { "test": "oxidant_gui::tests::other",
                  "stdout": "thread 'whatever' panicked at src/other.rs:42:1" },
            ]
        });
        let issues = parse_test_failures(&v);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].group_key, "oxidant_core");
        assert_eq!(
            issues[0].file.as_deref(),
            Some("crates/oxidant-core/src/text_tool_calls.rs")
        );
        assert_eq!(issues[0].line, 301);
        assert_eq!(issues[1].group_key, "oxidant_gui");
        assert_eq!(issues[1].file.as_deref(), Some("src/other.rs"));
    }

    #[test]
    fn parse_test_failures_includes_compile_messages_under_compile_group() {
        let v = serde_json::json!({
            "compile_messages": [
                { "level": "error", "message": "use of undeclared crate",
                  "spans": [{ "file": "src/lib.rs", "start": { "line": 5, "character": 0 }, "is_primary": true }] }
            ],
            "failures": []
        });
        let issues = parse_test_failures(&v);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].group_key.starts_with("(compile)"));
        assert!(matches!(issues[0].severity, IssueSeverity::Error));
    }

    #[test]
    fn parse_spec_validate_groups_by_warning_kind() {
        let v = serde_json::json!({
            "warnings": [
                { "spec_id": "components/x", "kind": "UnresolvedRef", "message": "no such ref `foo`",
                  "location": ["spec/components/x.md", 84, 1] },
                { "spec_id": "components/y", "kind": "MissingCodePath", "message": "src/x.rs missing",
                  "location": null },
                { "spec_id": "components/z", "kind": "UnresolvedRef", "message": "another miss",
                  "location": ["spec/components/z.md", 12, 1] },
            ]
        });
        let issues = parse_spec_validate(&v);
        assert_eq!(issues.len(), 3);
        let unresolved: Vec<&HealthIssue> = issues
            .iter()
            .filter(|i| i.group_key == "UnresolvedRef")
            .collect();
        assert_eq!(unresolved.len(), 2);
        let missing: Vec<&HealthIssue> = issues
            .iter()
            .filter(|i| i.group_key == "MissingCodePath")
            .collect();
        assert_eq!(missing.len(), 1);
        assert!(missing[0].file.is_none());
    }

    #[test]
    fn parse_spec_diff_groups_by_finding_kind() {
        let v = serde_json::json!({
            "count": 2,
            "drifts": [
                { "kind": "MissingCodePath", "spec_id": "components/foo", "path": "src/foo.rs" },
                { "kind": "MethodSignatureChanged", "contract_id": "contracts/provider", "method": "chat",
                  "spec": "fn chat(req) -> X", "code": "fn chat(req, opts) -> X" }
            ]
        });
        let issues = parse_spec_diff(&v);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].group_key, "MissingCodePath");
        assert!(issues[0].message.contains("src/foo.rs"));
        assert_eq!(issues[1].group_key, "MethodSignatureChanged");
        assert!(issues[1].message.contains("chat"));
    }

    #[test]
    fn extract_panic_location_pulls_file_and_line() {
        let (f, l) =
            extract_panic_location("thread 'x' panicked at crates/oxidant-core/src/lib.rs:42:3:\n");
        assert_eq!(f.as_deref(), Some("crates/oxidant-core/src/lib.rs"));
        assert_eq!(l, 42);
    }

    #[test]
    fn extract_panic_location_returns_none_when_no_panic_marker() {
        let (f, l) = extract_panic_location("running 1 test\ntest foo::bar ... ok\n");
        assert!(f.is_none());
        assert_eq!(l, 0);
    }
}
