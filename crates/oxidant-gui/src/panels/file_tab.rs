// Realises spec/components/gui/file-tabs.md.
//
// One mode now: an editable text editor with syntect-driven syntax
// highlighting (per the spec's "Render" section). The FileSource tag
// only changes the editor's "(editable spec)" / "(editable code)"
// hint and which syntax the highlighter picks. Code edits are
// allowed — the user accepted that trade-off when the file tree
// shipped; see "Editability across sources" in the spec.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::SystemTime;

use egui::{Color32, FontId, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use crate::app::{EditorBuffer, SelectedPreview, SharedState, ViewMode};
use crate::dock::FileSource;
use crate::highlighter;
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
        markdown_cache: &mut CommonMarkCache,
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

        render_editor(ui, &header_path, &absolute, source, state, markdown_cache);
    }
}

/// True for files we render as markdown (`.md` / `.markdown`).
fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
        .unwrap_or(false)
}

fn render_editor(
    ui: &mut egui::Ui,
    header_path: &str,
    absolute: &Path,
    source: FileSource,
    state: &Arc<StdMutex<SharedState>>,
    markdown_cache: &mut CommonMarkCache,
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
                        view_mode: if is_markdown(absolute) {
                            ViewMode::Preview
                        } else {
                            ViewMode::Source
                        },
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
        ui.label(RichText::new(source_label(source)).color(theme::muted_text()));

        // Preview | Source toggle — markdown files only.
        if is_markdown(absolute) {
            ui.separator();
            ui.selectable_value(&mut buf.view_mode, ViewMode::Preview, "Preview");
            ui.selectable_value(&mut buf.view_mode, ViewMode::Source, "Source");
        }

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

    if is_markdown(absolute) && buf.view_mode == ViewMode::Preview {
        // Rendered markdown — read-only; edits happen in Source mode.
        egui::ScrollArea::both()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                CommonMarkViewer::new().show(ui, markdown_cache, &buf.text);
            });
    } else {
        // Editable source view: syntect highlighting + a line-number gutter.
        let text_before = buf.text.clone();
        render_code_with_gutter(ui, absolute, &mut buf.text, true);
        if buf.text != text_before {
            buf.dirty = true;
        }
    }

    // Apply actions in this order: discard / reload (destructive) → save.
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

fn source_label(source: FileSource) -> &'static str {
    match source {
        FileSource::Code => "(editable code — competes with the agent's edits)",
        FileSource::Spec => "(editable spec)",
    }
}

fn mtimes_differ(a: Option<SystemTime>, b: Option<SystemTime>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x != y,
        _ => false,
    }
}

/// Render the read-only `Selected` preview tab. Markdown is rendered via
/// `egui_commonmark`; other files show syntect-highlighted, non-editable
/// text. Content comes from `SharedState::selected_preview` (loaded once
/// at single-click time). See spec/components/gui/dock-layout.md.
pub fn render_selected(
    ui: &mut egui::Ui,
    preview: Option<&SelectedPreview>,
    workspace_root: &Path,
    markdown_cache: &mut CommonMarkCache,
) {
    let preview = match preview {
        Some(p) => p,
        None => {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                ui.label(
                    RichText::new("Click a file or spec in the explorer to preview it here.")
                        .color(theme::muted_text()),
                );
            });
            return;
        }
    };

    let header_path = preview
        .path
        .strip_prefix(workspace_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| preview.path.to_string_lossy().to_string());

    ui.horizontal(|ui| {
        ui.label(RichText::new(header_path).strong());
        ui.label(
            RichText::new("(preview — double-click in the tree to edit)")
                .color(theme::muted_text()),
        );
    });
    ui.separator();

    if let Some(err) = &preview.error {
        ui.label(RichText::new(format!("could not read file: {err}")).color(Color32::RED));
        return;
    }

    if is_markdown(&preview.path) {
        egui::ScrollArea::both()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                CommonMarkViewer::new().show(ui, markdown_cache, &preview.text);
            });
    } else {
        // Read-only highlighted view with a line-number gutter. TextEdit
        // needs a &mut buffer even when non-interactive; feed it a
        // throwaway clone the widget can't actually mutate.
        let mut text = preview.text.clone();
        render_code_with_gutter(ui, &preview.path, &mut text, false);
    }
}

/// Right-aligned line numbers `"1\n2\n…\nN"` for the gutter. Pure so it
/// can be unit-tested without a UI.
fn gutter_text(line_count: usize) -> String {
    use std::fmt::Write;
    let width = line_count.to_string().len();
    let mut s = String::with_capacity(line_count * (width + 1));
    for n in 1..=line_count {
        if n > 1 {
            s.push('\n');
        }
        let _ = write!(s, "{n:>width$}");
    }
    s
}

/// Render `text` as a no-wrap, syntect-highlighted code view with a
/// line-number gutter on the left. `interactive` toggles editability.
/// No-wrap (the layouter is called with `f32::INFINITY`) keeps every
/// logical line one visual row so the gutter stays aligned 1:1.
///
/// Known limitation: with `ScrollArea::both`, scrolling far right also
/// scrolls the gutter off — it isn't horizontally pinned. Pinning needs
/// linked scroll offsets; deferred.
fn render_code_with_gutter(ui: &mut egui::Ui, path: &Path, text: &mut String, interactive: bool) {
    let font_id = FontId::monospace(13.0);
    let path_for_layout = path.to_path_buf();
    let mut layouter = |ui: &egui::Ui, t: &str, _wrap_width: f32| {
        // Force no-wrap so visual rows == logical lines (gutter alignment).
        let job = highlighter::highlight(&path_for_layout, t, font_id.clone(), f32::INFINITY);
        ui.fonts(|f| f.layout_job(job))
    };
    // `'\n' count + 1` matches what the editor shows, including the empty
    // line a trailing newline produces.
    let line_count = text.bytes().filter(|&b| b == b'\n').count() + 1;

    egui::ScrollArea::both()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // Gutter: same monospace size as the code → equal row
                // height; `frame(false)` on the editor drops its inset so
                // the first number lines up with the first code row.
                ui.add(
                    egui::Label::new(
                        RichText::new(gutter_text(line_count))
                            .font(FontId::monospace(13.0))
                            .color(theme::faint_text()),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
                let mut output = egui::TextEdit::multiline(text)
                    .code_editor()
                    .interactive(interactive)
                    .frame(false)
                    .desired_rows(20)
                    .layouter(&mut layouter)
                    .show(ui);
                if interactive {
                    handle_goto(ui, path, &mut output, line_count);
                }
            });
        });
}

/// Transient state for the Ctrl+G "go to line" dialog. Stored in egui
/// temp memory keyed by the editor path — the editor panel is stateless.
#[derive(Clone, Default)]
struct GotoLine {
    input: String,
    error: Option<String>,
    focus_next_frame: bool,
}

fn goto_id(path: &Path) -> egui::Id {
    egui::Id::new(("goto-line", path))
}

/// Char offset of the start of `line` (1-based), counting each preceding
/// line plus its `\n`. Clamped so an out-of-range line lands at the last
/// line start (never past the end).
fn line_start_char(text: &str, line_1based: usize) -> usize {
    if line_1based <= 1 {
        return 0;
    }
    let target = line_1based - 1; // newlines to skip
    let mut seen = 0usize;
    let mut chars = 0usize;
    for ch in text.chars() {
        chars += 1;
        if ch == '\n' {
            seen += 1;
            if seen == target {
                return chars; // position just after this newline
            }
        }
    }
    chars // fewer lines than requested → clamp to end
}

/// Ctrl+G go-to-line: open on the shortcut, render the modal, and on OK
/// move the caret to the chosen line and scroll it into view. See
/// spec/components/gui/file-tabs.md "Go to line".
fn handle_goto(
    ui: &mut egui::Ui,
    path: &Path,
    output: &mut egui::text_edit::TextEditOutput,
    line_count: usize,
) {
    let id = goto_id(path);

    // Open on Ctrl+G (Cmd+G on macOS) while the editor owns focus.
    if output.response.has_focus()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::G))
    {
        ui.memory_mut(|m| {
            m.data.insert_temp(
                id,
                GotoLine {
                    focus_next_frame: true,
                    ..Default::default()
                },
            )
        });
    }

    let Some(mut goto) = ui.memory(|m| m.data.get_temp::<GotoLine>(id)) else {
        return;
    };

    let mut want_ok = false;
    let mut close = false;

    egui::Window::new("Go to line")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ui.ctx(), |ui| {
            ui.label(RichText::new(format!("line (1–{line_count})")).color(theme::muted_text()));
            let resp = ui.add(
                egui::TextEdit::singleline(&mut goto.input)
                    .hint_text("e.g. 42")
                    .desired_width(120.0),
            );
            if goto.focus_next_frame {
                resp.request_focus();
                goto.focus_next_frame = false;
            }
            // Keep digits only — the field is a line number.
            goto.input.retain(|c| c.is_ascii_digit());
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                want_ok = true;
            }
            if let Some(err) = &goto.error {
                ui.label(RichText::new(err).color(ui.visuals().error_fg_color));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Go").clicked() {
                        want_ok = true;
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        close = true;
                    }
                });
            });
        });

    if want_ok {
        match goto.input.trim().parse::<usize>() {
            Ok(n) if n >= 1 => {
                let line = n.min(line_count);
                let cidx = line_start_char(output.galley.text(), line);
                let ccursor = egui::text::CCursor::new(cidx);
                output
                    .state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(ccursor, ccursor)));
                output.state.clone().store(ui.ctx(), output.response.id);
                output.response.request_focus();
                // Scroll the chosen line into view.
                let rect = output
                    .galley
                    .pos_from_ccursor(ccursor)
                    .translate(output.galley_pos.to_vec2());
                ui.scroll_to_rect(rect, Some(egui::Align::Center));
                close = true;
            }
            _ => {
                goto.error = Some("enter a line number".to_string());
            }
        }
    }

    if close {
        ui.memory_mut(|m| m.data.remove::<GotoLine>(id));
    } else {
        ui.memory_mut(|m| m.data.insert_temp(id, goto));
    }
}

#[cfg(test)]
mod tests {
    use super::{gutter_text, is_markdown, line_start_char};
    use std::path::Path;

    #[test]
    fn line_start_char_maps_lines_to_offsets() {
        let t = "a\nbb\nccc";
        assert_eq!(line_start_char(t, 1), 0); // 'a'
        assert_eq!(line_start_char(t, 2), 2); // 'bb'  (after "a\n")
        assert_eq!(line_start_char(t, 3), 5); // 'ccc' (after "a\nbb\n")
    }

    #[test]
    fn line_start_char_clamps_and_handles_edges() {
        let t = "a\nbb\nccc";
        assert_eq!(line_start_char(t, 0), 0); // 0/1 → start
        assert_eq!(line_start_char(t, 99), t.chars().count()); // beyond → end
        assert_eq!(line_start_char("", 5), 0);
    }

    #[test]
    fn gutter_text_single_digit_unpadded() {
        assert_eq!(gutter_text(3), "1\n2\n3");
        assert_eq!(gutter_text(1), "1");
    }

    #[test]
    fn gutter_text_right_aligns_to_widest_number() {
        // 10 lines → width 2; single digits get a leading space.
        let g = gutter_text(10);
        assert!(g.starts_with(" 1\n 2\n"), "got: {g:?}");
        assert!(g.ends_with("\n10"), "got: {g:?}");
    }

    #[test]
    fn markdown_extensions_detected_case_insensitively() {
        assert!(is_markdown(Path::new("spec/overview.md")));
        assert!(is_markdown(Path::new("README.MD")));
        assert!(is_markdown(Path::new("notes.markdown")));
    }

    #[test]
    fn non_markdown_is_rejected() {
        assert!(!is_markdown(Path::new("src/main.rs")));
        assert!(!is_markdown(Path::new("Cargo.toml")));
        assert!(!is_markdown(Path::new("LICENSE")));
    }
}
