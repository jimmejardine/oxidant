// Modal "New file / New directory" dialog shared by the spec tree and
// the file tree. The owner panel opens the dialog from a right-click
// context menu, then calls `render` every frame; `render` returns an
// outcome describing whether anything was created so the panel can
// refresh its tree and queue a centre-tab open.
//
// Realises the "New file / New directory" affordances documented in:
//   spec/components/gui/spec-tree-panel.md
//   spec/components/gui/file-tree-panel.md

use std::path::{Path, PathBuf};

use egui::{RichText, TextEdit};

use crate::theme;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum NewKind {
    File,
    Directory,
}

pub struct NewItemDialog {
    pending: Option<Pending>,
}

struct Pending {
    parent_dir: PathBuf,
    kind: NewKind,
    name: String,
    error: Option<String>,
    /// Set true the frame we want the TextEdit to grab focus. Cleared
    /// after the first frame so subsequent renders don't fight the
    /// user's caret.
    focus_next_frame: bool,
}

#[derive(Default)]
pub struct NewItemOutcome {
    /// Absolute path of a newly created **file**. The caller pushes
    /// this onto `pending_centre_tabs` so the editor opens.
    pub created_file: Option<PathBuf>,
    /// True if a directory was created. The caller uses this to
    /// invalidate its cached tree without opening a tab.
    pub created_directory: bool,
}

impl Default for NewItemDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl NewItemDialog {
    pub fn new() -> Self {
        Self { pending: None }
    }

    /// Begin a new-item flow under `parent_dir`. Replaces any in-flight
    /// dialog (the user clicked New again from a different directory's
    /// context menu).
    pub fn open(&mut self, parent_dir: PathBuf, kind: NewKind) {
        self.pending = Some(Pending {
            parent_dir,
            kind,
            name: String::new(),
            error: None,
            focus_next_frame: true,
        });
    }

    pub fn is_open(&self) -> bool {
        self.pending.is_some()
    }

    pub fn render(&mut self, ctx: &egui::Context) -> NewItemOutcome {
        let mut outcome = NewItemOutcome::default();
        let mut close = false;

        let pending = match self.pending.as_mut() {
            Some(p) => p,
            None => return outcome,
        };

        let title = match pending.kind {
            NewKind::File => "New file",
            NewKind::Directory => "New directory",
        };
        let label = match pending.kind {
            NewKind::File => "Filename",
            NewKind::Directory => "Directory name",
        };

        let mut want_create = false;
        let mut want_cancel = false;

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("under {}", display_dir(&pending.parent_dir)))
                        .color(theme::muted_text()),
                );
                ui.add_space(4.0);

                ui.label(label);
                let resp = ui.add(
                    TextEdit::singleline(&mut pending.name)
                        .hint_text(match pending.kind {
                            NewKind::File => "e.g. notes.md",
                            NewKind::Directory => "e.g. ideas",
                        })
                        .desired_width(280.0),
                );
                if pending.focus_next_frame {
                    resp.request_focus();
                    pending.focus_next_frame = false;
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    want_create = true;
                }

                if let Some(err) = &pending.error {
                    ui.label(RichText::new(err).color(ui.visuals().error_fg_color));
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Create").clicked() {
                            want_create = true;
                        }
                        if ui.button("Cancel").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            want_cancel = true;
                        }
                    });
                });
            });

        if want_cancel {
            close = true;
        } else if want_create {
            match validate(&pending.name, &pending.parent_dir) {
                Err(e) => {
                    pending.error = Some(e);
                }
                Ok(target) => match perform(pending.kind, &target) {
                    Ok(()) => {
                        match pending.kind {
                            NewKind::File => outcome.created_file = Some(target),
                            NewKind::Directory => outcome.created_directory = true,
                        }
                        close = true;
                    }
                    Err(e) => {
                        pending.error = Some(e);
                    }
                },
            }
        }

        if close {
            self.pending = None;
        }
        outcome
    }
}

fn display_dir(dir: &Path) -> String {
    // Show the trailing portion so the dialog doesn't get dominated by a
    // long absolute path. Three components is usually enough context.
    let comps: Vec<_> = dir
        .components()
        .rev()
        .take(3)
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    if comps.is_empty() {
        return dir.to_string_lossy().to_string();
    }
    let mut joined: Vec<String> = comps.into_iter().rev().collect();
    if joined.len() == 3 {
        joined.insert(0, "…".to_string());
    }
    joined.join("/")
}

fn validate(name: &str, parent_dir: &Path) -> Result<PathBuf, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name cannot be empty".to_string());
    }
    if trimmed == "." || trimmed == ".." {
        return Err(format!("\"{trimmed}\" is not a valid name"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("name cannot contain path separators".to_string());
    }
    let target = parent_dir.join(trimmed);
    if target.exists() {
        return Err(format!("{trimmed} already exists"));
    }
    Ok(target)
}

fn perform(kind: NewKind, target: &Path) -> Result<(), String> {
    match kind {
        NewKind::File => {
            // Make sure the parent exists (it should, but be defensive
            // if the user is creating into a freshly-made subdir).
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::File::create(target).map_err(|e| e.to_string())?;
        }
        NewKind::Directory => {
            std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn validate_rejects_empty() {
        let dir = TempDir::new().unwrap();
        assert!(validate("", dir.path()).is_err());
        assert!(validate("   ", dir.path()).is_err());
    }

    #[test]
    fn validate_rejects_dot_and_dotdot() {
        let dir = TempDir::new().unwrap();
        assert!(validate(".", dir.path()).is_err());
        assert!(validate("..", dir.path()).is_err());
    }

    #[test]
    fn validate_rejects_path_separators() {
        let dir = TempDir::new().unwrap();
        assert!(validate("a/b", dir.path()).is_err());
        assert!(validate("a\\b", dir.path()).is_err());
    }

    #[test]
    fn validate_rejects_existing() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("foo.md"), b"").unwrap();
        let err = validate("foo.md", dir.path()).unwrap_err();
        assert!(err.contains("already exists"));
    }

    #[test]
    fn perform_creates_file_and_directory() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("hello.md");
        perform(NewKind::File, &file).unwrap();
        assert!(file.exists() && file.is_file());

        let sub = dir.path().join("subdir");
        perform(NewKind::Directory, &sub).unwrap();
        assert!(sub.exists() && sub.is_dir());
    }
}
