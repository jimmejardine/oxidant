// Realises spec/components/gui/exploration-list.md.
//
// MVP: shows the current worktree as a single "main" row. Spawning,
// merging, and discarding sub-explorations are GUI-only ops on the
// oxidant-vcs side; the buttons hook up here once multi-viewport
// lands. For now the row is informational.

use std::path::PathBuf;

use egui::{Color32, RichText};

pub struct ExplorationListPanel;

impl ExplorationListPanel {
    pub fn render(&self, ui: &mut egui::Ui, workspace_root: &PathBuf) {
        ui.label(RichText::new("explorations").strong());
        ui.separator();
        let label = workspace_root
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| workspace_root.to_string_lossy().to_string());
        ui.horizontal(|ui| {
            ui.label(RichText::new("[main]").color(Color32::LIGHT_GREEN).strong());
            ui.label(label);
        });
        ui.label(
            RichText::new(workspace_root.to_string_lossy())
                .color(Color32::DARK_GRAY)
                .small(),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Sub-exploration spawn / merge / discard are GUI-only ops landing once multi-viewport ships. The vcs_explorations_list tool exposes the read view to the agent.",
            )
            .color(Color32::DARK_GRAY)
            .small(),
        );
    }
}
