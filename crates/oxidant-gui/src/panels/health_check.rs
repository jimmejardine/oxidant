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

use egui::text::LayoutJob;
use egui::{Color32, CursorIcon, RichText, SelectableLabel, TextFormat};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use oxidant_core::{AgentMode, ToolContext, ToolRegistry, ToolResult};

use crate::app::{
    CheckKind, CheckState, CheckStatus, HealthIssue, IssueSeverity, PendingChatPrompt, SharedState,
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
                    let st = s.health.checks.get(k).cloned().unwrap_or_default();
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
                    render_root(ui, state, tokio_handle, workspace_root, egui_ctx, kind, &st);
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

#[allow(clippy::too_many_arguments)]
fn render_root(
    ui: &mut egui::Ui,
    state: &Arc<StdMutex<SharedState>>,
    tokio_handle: &Handle,
    workspace_root: &Path,
    egui_ctx: &egui::Context,
    kind: CheckKind,
    st: &CheckState,
) {
    let (glyph, glyph_colour, default_open) = glyph_for(st);
    let header_text = root_header_text(kind, st);

    let id = ui.make_persistent_id(("health_root", kind.as_str()));

    // Drive the collapsing tree manually so we can put a per-row ▶
    // button before the chevron, on the same row as the header. See
    // spec/components/gui/health-check-panel.md "Per-row run".
    let mut collapsing = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );

    let row = ui.horizontal(|ui| {
        let is_running = matches!(st.status, CheckStatus::Running);
        let button_text = if is_running { "⟳" } else { "▶" };
        let btn = ui.add_enabled(
            !is_running,
            egui::Button::new(RichText::new(button_text).monospace()).small(),
        );
        let btn = btn.on_hover_text(format!("Run {}", kind.display_name()));
        if btn.clicked() {
            spawn_check(state, tokio_handle, workspace_root, egui_ctx, kind);
        }
        let toggle = collapsing.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
        let label_resp =
            ui.label(RichText::new(format!("{glyph}  {header_text}")).color(glyph_colour));
        // Treat a click on the label (not just the chevron) as a toggle
        // so the user can hit either to open/close — matches the
        // affordance the previous CollapsingHeader gave for free.
        if label_resp
            .interact(egui::Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .clicked()
        {
            collapsing.toggle(ui);
            let mut s = state.lock().unwrap();
            if let Some(entry) = s.health.checks.get_mut(&kind) {
                entry.user_toggled = true;
            }
        }
        if toggle.clicked() {
            let mut s = state.lock().unwrap();
            if let Some(entry) = s.health.checks.get_mut(&kind) {
                entry.user_toggled = true;
            }
        }
    });

    collapsing.show_body_indented(&row.response, ui, |ui| {
        render_subtree(ui, state, kind, st);
    });
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
                    parts.push(format!(
                        "{errors} error{}",
                        if errors == 1 { "" } else { "s" }
                    ));
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
        ui.label(RichText::new(format!("tool failed: {msg}")).color(ui.visuals().error_fg_color));
        return;
    }
    if st.issues.is_empty() {
        ui.label(RichText::new("no issues").color(theme::muted_text()));
        return;
    }
    // Group by HealthIssue.group_key, sort each group by severity then file/line.
    let mut groups: BTreeMap<String, Vec<&HealthIssue>> = BTreeMap::new();
    for issue in &st.issues {
        groups
            .entry(issue.group_key.clone())
            .or_default()
            .push(issue);
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
        let group_response =
            egui::CollapsingHeader::new(RichText::new(header).color(theme::muted_text()))
                .id_salt(("health_group", group_key.clone()))
                .default_open(group_has_error)
                .show(ui, |ui| {
                    for issue in issues {
                        render_issue(ui, state, issue);
                    }
                });
        group_response
            .header_response
            .on_hover_cursor(CursorIcon::PointingHand);
    }
}

fn render_issue(ui: &mut egui::Ui, state: &Arc<StdMutex<SharedState>>, issue: &HealthIssue) {
    // Build one LayoutJob containing the whole leaf — severity tag,
    // message, optional location suffix — then add as a single
    // SelectableLabel. Mirrors panels/spec_tree.rs::render_leaf.
    //
    // The reason for one widget instead of three labels in a
    // horizontal layout: separate Label widgets each consume the
    // hover for their own rect, leaving the outer horizontal-row
    // response unhovered over the text — so the pointer cursor only
    // appears in the gaps. SelectableLabel covers the whole text as
    // a single hit region, which is what we want.
    let (tag, tag_colour) = match issue.severity {
        IssueSeverity::Error => ("[error] ", Color32::from_rgb(247, 118, 142)),
        IssueSeverity::Warning => ("[warn] ", Color32::from_rgb(255, 198, 109)),
        IssueSeverity::Note => ("[note] ", theme::muted_text()),
    };
    let mut job = LayoutJob::default();
    job.append(
        tag,
        0.0,
        TextFormat {
            color: tag_colour,
            ..Default::default()
        },
    );
    job.append(
        &issue.message,
        0.0,
        TextFormat {
            color: ui.visuals().text_color(),
            ..Default::default()
        },
    );
    if let Some(file) = &issue.file {
        job.append(
            &format!("  · {}:{}:{}", file, issue.line, issue.character),
            0.0,
            TextFormat {
                color: theme::muted_text(),
                ..Default::default()
            },
        );
    }

    let resp = ui
        .add(SelectableLabel::new(false, job))
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("click to open · double-click to ask the agent in Plan mode");

    // Double-click wins over single-click: a user double-clicking
    // for the "ask the agent" gesture is briefly clicking once, and
    // we don't want to flip-flop to the file-open action mid-gesture.
    if resp.double_clicked() {
        push_chat_prompt(state, build_issue_prompt(issue), AgentMode::Plan);
    } else if resp.clicked()
        && let Some(file) = &issue.file
    {
        push_open(state, file);
    }
}

/// Build the structured "address this issue" prompt the chat input
/// is auto-filled with on a leaf double-click. Pure function — the
/// HealthIssue value carries everything needed.
pub(crate) fn build_issue_prompt(issue: &HealthIssue) -> String {
    let severity = match issue.severity {
        IssueSeverity::Error => "error",
        IssueSeverity::Warning => "warning",
        IssueSeverity::Note => "note",
    };
    let mut out = String::from(
        "Help me address this Health Check issue. Investigate first, then describe (don't make) the fix you'd apply.\n\n",
    );
    out.push_str(&format!("Check:    {}\n", issue.check.display_name()));
    out.push_str(&format!("Severity: {severity}\n"));
    if let Some(file) = &issue.file {
        out.push_str(&format!(
            "File:     {}:{}:{}\n",
            file, issue.line, issue.character
        ));
    }
    // Drop Group when it just duplicates the file path (cargo / clippy
    // groups by file, so group_key == file in that case).
    let group_dup_of_file = issue
        .file
        .as_deref()
        .map(|f| f == issue.group_key)
        .unwrap_or(false);
    if !group_dup_of_file {
        out.push_str(&format!("Group:    {}\n", issue.group_key));
    }
    out.push_str(&format!("Message:  {}\n", issue.message));
    out
}

fn push_chat_prompt(state: &Arc<StdMutex<SharedState>>, prompt: String, mode: AgentMode) {
    if let Ok(mut s) = state.lock() {
        s.pending_chat_prompt = Some(PendingChatPrompt { prompt, mode });
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
    // last_run_at refers to the most recent batch run — per-row runs
    // (via spawn_check below) don't touch it.
    state.lock().unwrap().health.last_run_at = Some(Instant::now());
    for kind in ALL_CHECKS {
        spawn_check(state, tokio_handle, workspace_root, egui_ctx, kind);
    }
}

/// Fire a single check. Shared by Run-all and the per-row ▶ buttons.
/// See spec/components/gui/health-check-panel.md "Per-row run".
fn spawn_check(
    state: &Arc<StdMutex<SharedState>>,
    tokio_handle: &Handle,
    workspace_root: &Path,
    egui_ctx: &egui::Context,
    kind: CheckKind,
) {
    let (registry, exploration_id) = {
        let mut s = state.lock().unwrap();
        let entry = s.health.checks.entry(kind).or_default();
        entry.status = CheckStatus::Running;
        entry.issues.clear();
        entry.finished_in_ms = 0;
        (s.registry.clone(), s.exploration.id.to_string())
    };
    let state = state.clone();
    let workspace = workspace_root.to_path_buf();
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
            let entry = s.health.checks.entry(kind).or_default();
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
    match registry
        .invoke(tool_name, serde_json::json!({}), &ctx)
        .await
    {
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

    fn clippy_issue_with_file() -> HealthIssue {
        HealthIssue {
            check: CheckKind::Clippy,
            severity: IssueSeverity::Warning,
            group_key: "crates/oxidant-gui/src/panels/spec_graph.rs".into(),
            message: "unused variable `near`".into(),
            file: Some("crates/oxidant-gui/src/panels/spec_graph.rs".into()),
            line: 765,
            character: 13,
        }
    }

    #[test]
    fn build_issue_prompt_includes_check_severity_message() {
        let issue = clippy_issue_with_file();
        let p = build_issue_prompt(&issue);
        assert!(p.contains("Check:    clippy"));
        assert!(p.contains("Severity: warning"));
        assert!(p.contains("unused variable `near`"));
        assert!(
            p.contains("File:     crates/oxidant-gui/src/panels/spec_graph.rs:765:13"),
            "prompt missing file line: {p}"
        );
        // group_key == file → no Group: line at all
        assert!(!p.contains("Group:"));
    }

    #[test]
    fn build_issue_prompt_omits_file_when_none() {
        let issue = HealthIssue {
            check: CheckKind::SpecValidate,
            severity: IssueSeverity::Warning,
            group_key: "Orphan".into(),
            message: "components/foo has no inbound refs".into(),
            file: None,
            line: 0,
            character: 0,
        };
        let p = build_issue_prompt(&issue);
        assert!(!p.contains("File:"));
        // Group is still meaningful when there's no file to duplicate.
        assert!(p.contains("Group:    Orphan"));
        assert!(p.contains("Check:    spec validate"));
    }

    #[test]
    fn build_issue_prompt_keeps_group_when_distinct_from_file() {
        // Tests case: group_key is the test target (oxidant_core),
        // file is the panic site path — they should NOT coincide.
        let issue = HealthIssue {
            check: CheckKind::Tests,
            severity: IssueSeverity::Error,
            group_key: "oxidant_core".into(),
            message: "text_tool_calls::extract_qwen3_envelope".into(),
            file: Some("crates/oxidant-core/src/text_tool_calls.rs".into()),
            line: 301,
            character: 9,
        };
        let p = build_issue_prompt(&issue);
        assert!(p.contains("Group:    oxidant_core"));
        assert!(p.contains("File:     crates/oxidant-core/src/text_tool_calls.rs:301:9"));
    }

    #[test]
    fn build_issue_prompt_starts_with_a_plan_instruction() {
        let p = build_issue_prompt(&clippy_issue_with_file());
        assert!(
            p.starts_with("Help me address this Health Check issue."),
            "first line should be the instruction: {p}"
        );
        assert!(
            p.contains("describe (don't make) the fix"),
            "should explicitly tell the model not to mutate: {p}"
        );
    }
}
