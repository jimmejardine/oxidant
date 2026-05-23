// Realises spec/components/gui/diagnostic-panel.md.
//
// Right-docked panel showing diagnostics from the most recent cargo /
// rust-analyzer run. Click-to-open and apply-suggestion actions land
// once the panel can talk back to the agent loop; the MVP is a
// readable list view.

use egui::{Color32, RichText};

use crate::app::{DiagnosticEntry, SharedState};

pub struct DiagnosticPanel;

impl DiagnosticPanel {
    pub fn render(&self, ui: &mut egui::Ui, state: &SharedState) {
        ui.label(RichText::new("diagnostics").strong());
        ui.separator();
        if state.diagnostics.is_empty() {
            ui.label(
                RichText::new(
                    "no diagnostics yet — the agent populates this panel after a cargo_check / rust_diagnostics call",
                )
                .color(Color32::DARK_GRAY),
            );
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for d in &state.diagnostics {
                render_diag(ui, d);
            }
        });
    }
}

fn render_diag(ui: &mut egui::Ui, d: &DiagnosticEntry) {
    let color = match d.severity.as_str() {
        "error" => Color32::RED,
        "warning" => Color32::from_rgb(255, 200, 100),
        "info" => Color32::LIGHT_BLUE,
        _ => Color32::LIGHT_GRAY,
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("[{}]", d.severity)).color(color).strong());
        ui.label(
            RichText::new(format!("{}:{}:{}", d.file, d.line, d.character))
                .color(Color32::LIGHT_GRAY)
                .small(),
        );
    });
    ui.label(&d.message);
    ui.add_space(4.0);
}
