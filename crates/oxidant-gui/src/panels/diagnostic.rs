// Realises spec/components/gui/diagnostic-panel.md.
//
// Right-docked panel showing diagnostics from the most recent cargo /
// rust-analyzer run. The header carries a Refresh button that runs
// cargo_check against the active exploration on demand — without it,
// the panel sits empty until the model decides to call a Rust tool.
// Click-to-open and apply-suggestion actions land once the panel can
// talk back to the agent loop.

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Color32, RichText};
use tokio::runtime::Handle;

use oxidant_core::{ToolContext, ToolRegistry, ToolResult};
use tokio_util::sync::CancellationToken;

use crate::app::{DiagnosticEntry, SharedState};
use crate::theme;

pub struct DiagnosticPanel {
    refreshing: Arc<AtomicBool>,
    last_error: Arc<StdMutex<Option<String>>>,
}

impl Default for DiagnosticPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticPanel {
    pub fn new() -> Self {
        Self {
            refreshing: Arc::new(AtomicBool::new(false)),
            last_error: Arc::new(StdMutex::new(None)),
        }
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        state: &Arc<StdMutex<SharedState>>,
        tokio_handle: &Handle,
        workspace_root: &Path,
        egui_ctx: &egui::Context,
    ) {
        let refreshing = self.refreshing.load(Ordering::Relaxed);

        ui.horizontal(|ui| {
            ui.label(RichText::new("diagnostics").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if refreshing {
                    ui.spinner();
                    ui.add_enabled(false, egui::Button::new("Refreshing…"));
                } else if ui.button("Refresh").clicked() {
                    self.spawn_refresh(state, tokio_handle, workspace_root, egui_ctx);
                }
            });
        });
        ui.separator();

        if let Ok(err) = self.last_error.lock()
            && let Some(msg) = err.as_ref()
        {
            ui.label(
                RichText::new(format!("cargo check failed: {msg}"))
                    .color(ui.visuals().error_fg_color),
            );
            ui.add_space(4.0);
        }

        let state = state.lock().unwrap();
        if state.diagnostics.is_empty() {
            ui.label(
                RichText::new(
                    "no diagnostics yet — click Refresh to run cargo check, or wait for the agent to call cargo_check / rust_diagnostics",
                )
                .color(theme::muted_text()),
            );
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for d in &state.diagnostics {
                    render_diag(ui, d);
                }
            });
    }

    fn spawn_refresh(
        &self,
        state: &Arc<StdMutex<SharedState>>,
        tokio_handle: &Handle,
        workspace_root: &Path,
        egui_ctx: &egui::Context,
    ) {
        // Snapshot what we need from shared state before crossing threads:
        // the registry (Arc) and the exploration_id (cheap clone).
        let (registry, exploration_id) = {
            let s = state.lock().unwrap();
            (s.registry.clone(), s.exploration.id.to_string())
        };
        let workspace = workspace_root.to_path_buf();
        let state = state.clone();
        let refreshing = self.refreshing.clone();
        let last_error = self.last_error.clone();
        let egui_ctx = egui_ctx.clone();

        refreshing.store(true, Ordering::Relaxed);
        if let Ok(mut e) = last_error.lock() {
            *e = None;
        }

        tokio_handle.spawn(async move {
            let result = run_cargo_check(&registry, &workspace, &exploration_id).await;
            match result {
                Ok(diags) => {
                    let mut s = state.lock().unwrap();
                    s.diagnostics = diags;
                }
                Err(msg) => {
                    if let Ok(mut e) = last_error.lock() {
                        *e = Some(msg);
                    }
                }
            }
            refreshing.store(false, Ordering::Relaxed);
            egui_ctx.request_repaint();
        });
    }
}

async fn run_cargo_check(
    registry: &Arc<ToolRegistry>,
    workspace_root: &Path,
    exploration_id: &str,
) -> Result<Vec<DiagnosticEntry>, String> {
    let canonical =
        dunce::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let workspace_camino = camino::Utf8PathBuf::from_path_buf(canonical.clone())
        .map_err(|_| format!("non-UTF-8 workspace path: {}", canonical.display()))?;

    let ctx = ToolContext {
        workspace_root: workspace_camino,
        exploration_id: exploration_id.to_string(),
        cancellation: CancellationToken::new(),
    };

    match registry
        .invoke("cargo_check", serde_json::json!({}), &ctx)
        .await
    {
        ToolResult::Ok(value) => Ok(extract_diagnostics(&value)),
        ToolResult::Err(e) => Err(e),
    }
}

fn extract_diagnostics(value: &serde_json::Value) -> Vec<DiagnosticEntry> {
    let messages = match value.get("messages").and_then(|m| m.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for m in messages {
        let level = m
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("info")
            .to_string();
        let message = m
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // Prefer the primary span, else the first.
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
            Some(s) => {
                let file = s
                    .get("file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                let line = s
                    .get("start")
                    .and_then(|p| p.get("line"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let character = s
                    .get("start")
                    .and_then(|p| p.get("character"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                (file, line, character)
            }
            None => (String::new(), 0, 0),
        };
        out.push(DiagnosticEntry {
            file,
            line,
            character,
            message,
            severity: level,
        });
    }
    out
}

fn render_diag(ui: &mut egui::Ui, d: &DiagnosticEntry) {
    let color = match d.severity.as_str() {
        "error" => Color32::RED,
        "warning" => Color32::from_rgb(255, 200, 100),
        "info" => Color32::LIGHT_BLUE,
        _ => theme::muted_text(),
    };
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("[{}]", d.severity))
                .color(color)
                .strong(),
        );
        ui.label(
            RichText::new(format!("{}:{}:{}", d.file, d.line, d.character))
                .color(theme::muted_text()),
        );
    });
    ui.label(&d.message);
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_picks_primary_span_and_keeps_level() {
        let v = serde_json::json!({
            "messages": [
                {
                    "level": "error",
                    "message": "mismatched types",
                    "spans": [
                        { "file": "src/lib.rs", "start": { "line": 12, "character": 4 }, "is_primary": false },
                        { "file": "src/main.rs", "start": { "line": 99, "character": 2 }, "is_primary": true }
                    ]
                }
            ]
        });
        let diags = extract_diagnostics(&v);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, "error");
        assert_eq!(diags[0].file, "src/main.rs");
        assert_eq!(diags[0].line, 99);
        assert_eq!(diags[0].character, 2);
        assert_eq!(diags[0].message, "mismatched types");
    }

    #[test]
    fn extract_falls_back_to_first_span_when_no_primary() {
        let v = serde_json::json!({
            "messages": [
                {
                    "level": "warning",
                    "message": "unused variable",
                    "spans": [
                        { "file": "src/a.rs", "start": { "line": 1, "character": 0 }, "is_primary": false }
                    ]
                }
            ]
        });
        let diags = extract_diagnostics(&v);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "src/a.rs");
        assert_eq!(diags[0].severity, "warning");
    }

    #[test]
    fn extract_handles_message_without_spans() {
        let v = serde_json::json!({
            "messages": [
                { "level": "help", "message": "consider adding ;", "spans": [] }
            ]
        });
        let diags = extract_diagnostics(&v);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "");
        assert_eq!(diags[0].line, 0);
        assert_eq!(diags[0].severity, "help");
    }

    #[test]
    fn extract_returns_empty_when_no_messages_field() {
        let v = serde_json::json!({ "ok": true });
        assert!(extract_diagnostics(&v).is_empty());
    }
}
