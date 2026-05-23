// Realises spec/components/gui/file-tabs.md.
//
// MVP renders the file as plain text in a scrollable, monospace
// view. Syntect-tokenised highlighting and egui_commonmark rendering
// land in the next phase; for now, the agent reads via tools and
// the user sees the same content read-only.

use std::path::{Path, PathBuf};

use egui::{Color32, RichText};

use crate::dock::FileSource;

pub struct FileTabPanel;

impl FileTabPanel {
    pub fn render(
        &self,
        ui: &mut egui::Ui,
        path: &Path,
        _source: FileSource,
        workspace_root: &PathBuf,
    ) {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace_root.join(path)
        };
        let header_path = absolute
            .strip_prefix(workspace_root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| absolute.to_string_lossy().to_string());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&header_path).strong());
            ui.label(
                RichText::new("(read-only — edits go through the agent)")
                    .color(Color32::DARK_GRAY)
                    .small(),
            );
        });
        ui.separator();

        match std::fs::read_to_string(&absolute) {
            Ok(content) => {
                egui::ScrollArea::both()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.code(content);
                    });
            }
            Err(e) => {
                ui.label(
                    RichText::new(format!("could not read {}: {e}", absolute.display()))
                        .color(Color32::RED),
                );
            }
        }
    }
}
