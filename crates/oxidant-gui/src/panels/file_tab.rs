// Realises spec/components/gui/file-tabs.md.
//
// Two modes:
//   Code source  → read-only monospace view. The agent makes the edits.
//   Spec source  → multi-line editor with Save. Specs are canonical
//                  (decision 0008) and the user editing one is the
//                  normal path. Per-tab buffers live in
//                  SharedState::editor_buffers so closing+reopening a
//                  tab preserves unsaved work.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::SystemTime;

use egui::{Color32, RichText};

use crate::app::{EditorBuffer, SharedState};
use crate::dock::FileSource;
use crate::theme;

pub struct FileTabPanel;

impl FileTabPanel {
    pub fn render(
        &self,
        ui: &mut egui::Ui,
        path: &Path,
        source: FileSource,
        workspace_root: &PathBuf,
        state: &Arc<StdMutex<SharedState>>,
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

        match source {
            FileSource::Code => render_code(ui, &header_path, &absolute),
            FileSource::Spec => render_spec_editor(ui, &header_path, &absolute, state),
        }
    }
}

fn render_code(ui: &mut egui::Ui, header_path: &str, absolute: &Path) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(header_path).strong());
        ui.label(
            RichText::new("(read-only — edits go through the agent)").color(theme::muted_text()),
        );
    });
    ui.separator();

    match std::fs::read_to_string(absolute) {
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

fn render_spec_editor(
    ui: &mut egui::Ui,
    header_path: &str,
    absolute: &Path,
    state: &Arc<StdMutex<SharedState>>,
) {
    // Load on first sight, or surface a load error.
    {
        let need_load = !state
            .lock()
            .map(|s| s.editor_buffers.contains_key(absolute))
            .unwrap_or(true);
        if need_load {
            let (text, mtime, error) = match std::fs::read_to_string(absolute) {
                Ok(t) => (
                    t,
                    std::fs::metadata(absolute).and_then(|m| m.modified()).ok(),
                    None,
                ),
                Err(e) => (
                    String::new(),
                    None,
                    Some(format!("could not read {}: {e}", absolute.display())),
                ),
            };
            if let Ok(mut s) = state.lock() {
                s.editor_buffers.insert(
                    absolute.to_path_buf(),
                    EditorBuffer {
                        text,
                        dirty: false,
                        mtime_at_load: mtime,
                        last_save_error: error,
                    },
                );
            }
        }
    }

    // Pull a working copy out, render, write back. Holding the lock
    // across the render closure would deadlock if the closure ever
    // re-entered SharedState (e.g. via egui events).
    let mut buf = match state
        .lock()
        .ok()
        .and_then(|s| s.editor_buffers.get(absolute).cloned())
    {
        Some(b) => b,
        None => {
            ui.label(
                RichText::new(format!("could not load {}", absolute.display())).color(Color32::RED),
            );
            return;
        }
    };

    // Header row: filename, dirty marker, Save / Reload buttons.
    let on_disk_mtime = std::fs::metadata(absolute).and_then(|m| m.modified()).ok();
    let disk_changed = mtimes_differ(buf.mtime_at_load, on_disk_mtime);

    let mut do_save = false;
    let mut do_reload = false;
    let mut do_discard = false;

    ui.horizontal(|ui| {
        let title = if buf.dirty {
            format!("● {header_path}")
        } else {
            header_path.to_string()
        };
        ui.label(RichText::new(title).strong());
        ui.label(RichText::new("(editable spec)").color(theme::muted_text()));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let save_resp = ui.add_enabled(buf.dirty, egui::Button::new("Save"));
            if save_resp.clicked() {
                do_save = true;
            }
            if buf.dirty {
                let discard_resp = ui.button("Discard");
                if discard_resp.clicked() {
                    do_discard = true;
                }
            }
        });
    });

    if disk_changed && buf.dirty {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("⚠ this file changed on disk while you were editing")
                    .color(ui.visuals().warn_fg_color),
            );
            if ui.button("Reload").clicked() {
                do_reload = true;
            }
        });
    } else if disk_changed && !buf.dirty {
        // No conflict — silently re-sync from disk.
        do_reload = true;
    }

    if let Some(err) = &buf.last_save_error {
        ui.label(RichText::new(format!("save failed: {err}")).color(ui.visuals().error_fg_color));
    }

    ui.separator();

    // Editor body.
    let text_before = buf.text.clone();
    egui::ScrollArea::both()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.add_sized(
                [ui.available_width(), ui.available_height().max(120.0)],
                egui::TextEdit::multiline(&mut buf.text)
                    .code_editor()
                    .desired_rows(20),
            );
        });
    if buf.text != text_before {
        buf.dirty = true;
    }

    // Apply actions in this order: discard / reload (destructive) → save.
    // Save reads the freshly-edited buf.text below.
    if do_discard || do_reload {
        match std::fs::read_to_string(absolute) {
            Ok(t) => {
                buf.text = t;
                buf.mtime_at_load = on_disk_mtime;
                buf.dirty = false;
                buf.last_save_error = None;
            }
            Err(e) => {
                buf.last_save_error = Some(format!("could not reload {}: {e}", absolute.display()));
            }
        }
    } else if do_save {
        match std::fs::write(absolute, &buf.text) {
            Ok(_) => {
                buf.dirty = false;
                buf.mtime_at_load = std::fs::metadata(absolute).and_then(|m| m.modified()).ok();
                buf.last_save_error = None;
            }
            Err(e) => {
                buf.last_save_error = Some(e.to_string());
            }
        }
    }

    // Write the working copy back into shared state.
    if let Ok(mut s) = state.lock() {
        s.editor_buffers.insert(absolute.to_path_buf(), buf);
    }
}

fn mtimes_differ(a: Option<SystemTime>, b: Option<SystemTime>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x != y,
        _ => false,
    }
}
