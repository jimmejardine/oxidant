// Realises spec/components/gui/diff-history-panel.md.
//
// Read-only side-by-side diff viewer for one file. Two columns, each
// with a commit-picker dropdown that lists every commit that touched
// the file plus a virtual "Working tree" entry. Line-level diff
// computed with `similar`; syntect-highlighted text in both columns
// with red/green overlay bands for Delete/Insert.

use std::path::PathBuf;
use std::time::SystemTime;

use egui::{Color32, FontId, RichText, ScrollArea, TextFormat};
use similar::{ChangeTag, TextDiff};
use tokio::runtime::Handle;

use oxidant_vcs::{Commit, Git, GitError, LogOpts};

use crate::highlighter;
use crate::theme;

/// One entry in either of the two dropdowns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitChoice {
    /// File as it currently exists on disk (includes unsaved edits — read
    /// fresh from disk on every refresh, not from any open editor buffer).
    WorkingTree,
    Sha(String),
    /// "Before the first commit that touched this file." Renders as an
    /// empty string in the diff so the first commit's full contents
    /// surface as Inserts.
    EmptyTree,
}

pub struct DiffHistoryPanel {
    /// Absolute, canonicalised path of the file under review.
    pub path: PathBuf,
    /// Repo root — the parent worktree of `path`. `Git` invocations run
    /// here; `path` is converted to a repo-relative form before being
    /// passed to `git show`.
    repo_root: PathBuf,
    /// Lazily-loaded list of commits that touched the file, newest-first.
    /// `None` until first paint; `Some(vec)` (possibly empty) after.
    commits: Option<Result<Vec<Commit>, String>>,
    left: CommitChoice,
    right: CommitChoice,
    /// Cached contents per side, keyed by the choice that produced them.
    left_loaded: Option<(CommitChoice, Result<String, String>)>,
    right_loaded: Option<(CommitChoice, Result<String, String>)>,
    /// Last-seen worktree mtime, so `WorkingTree` is re-read when the
    /// file changes on disk.
    working_mtime: Option<SystemTime>,
}

impl DiffHistoryPanel {
    pub fn new(absolute_path: PathBuf, repo_root: PathBuf) -> Self {
        Self {
            path: absolute_path,
            repo_root,
            commits: None,
            // Sensible defaults set on first paint once we know the
            // commit list.
            left: CommitChoice::EmptyTree,
            right: CommitChoice::WorkingTree,
            left_loaded: None,
            right_loaded: None,
            working_mtime: None,
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui, tokio_handle: &Handle) {
        // First-paint: populate `commits` and pick sensible defaults.
        if self.commits.is_none() {
            self.refresh_commits(tokio_handle);
            if let Some(Ok(cs)) = &self.commits
                && let Some(newest) = cs.first()
            {
                // Default left side: parent of newest commit. If the
                // newest commit has no parent (root commit), use the
                // EmptyTree sentinel so we still show a diff.
                self.left = CommitChoice::Sha(format!("{}^", newest.sha));
            }
        }

        // Top bar: dropdowns + swap + refresh.
        let mut changed = false;
        ui.horizontal(|ui| {
            let commits_clone = self.commits.clone();
            if Self::dropdown(ui, "diff-history-left", &mut self.left, &commits_clone) {
                changed = true;
            }
            if ui.small_button("⇄").on_hover_text("swap sides").clicked() {
                std::mem::swap(&mut self.left, &mut self.right);
                changed = true;
            }
            if Self::dropdown(ui, "diff-history-right", &mut self.right, &commits_clone) {
                changed = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("⟳")
                    .on_hover_text("re-query git log")
                    .clicked()
                {
                    self.commits = None;
                    self.left_loaded = None;
                    self.right_loaded = None;
                }
            });
        });

        // Show the commit-list error inline if log() failed.
        if let Some(Err(msg)) = &self.commits {
            ui.label(RichText::new(format!("git log failed: {msg}")).color(Color32::LIGHT_RED));
            return;
        }

        // Detect WorkingTree mtime change → invalidate the affected side.
        if self.left == CommitChoice::WorkingTree || self.right == CommitChoice::WorkingTree {
            let now_mtime = std::fs::metadata(&self.path)
                .and_then(|m| m.modified())
                .ok();
            if now_mtime != self.working_mtime {
                self.working_mtime = now_mtime;
                if self.left == CommitChoice::WorkingTree {
                    self.left_loaded = None;
                }
                if self.right == CommitChoice::WorkingTree {
                    self.right_loaded = None;
                }
            }
        }

        // Load both sides if their cache misses (or selection changed).
        if changed
            || self
                .left_loaded
                .as_ref()
                .map(|(c, _)| c != &self.left)
                .unwrap_or(true)
        {
            let text = self.load_choice(&self.left.clone(), tokio_handle);
            self.left_loaded = Some((self.left.clone(), text));
        }
        if changed
            || self
                .right_loaded
                .as_ref()
                .map(|(c, _)| c != &self.right)
                .unwrap_or(true)
        {
            let text = self.load_choice(&self.right.clone(), tokio_handle);
            self.right_loaded = Some((self.right.clone(), text));
        }

        let left_text = match &self.left_loaded {
            Some((_, Ok(t))) => t.clone(),
            Some((_, Err(_))) | None => String::new(),
        };
        let right_text = match &self.right_loaded {
            Some((_, Ok(t))) => t.clone(),
            Some((_, Err(_))) | None => String::new(),
        };

        ui.separator();

        // Classify lines via similar; render two columns side-by-side.
        let (left_lines, right_lines) = classify(&left_text, &right_text);
        let path_clone = self.path.clone();
        let font_id = FontId::monospace(13.0);
        let scroll_id = egui::Id::new(("diff-history-scroll", &self.path));
        ScrollArea::vertical()
            .id_salt(scroll_id)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.columns(2, |cols| {
                    render_column(
                        &mut cols[0],
                        &left_lines,
                        &path_clone,
                        &font_id,
                        Side::Left,
                        self.left_loaded
                            .as_ref()
                            .and_then(|(_, r)| r.as_ref().err()),
                    );
                    render_column(
                        &mut cols[1],
                        &right_lines,
                        &path_clone,
                        &font_id,
                        Side::Right,
                        self.right_loaded
                            .as_ref()
                            .and_then(|(_, r)| r.as_ref().err()),
                    );
                });
            });
    }

    fn dropdown(
        ui: &mut egui::Ui,
        salt: &str,
        choice: &mut CommitChoice,
        commits: &Option<Result<Vec<Commit>, String>>,
    ) -> bool {
        let selected_label = label_for(choice, commits);
        let mut changed = false;
        egui::ComboBox::from_id_salt(salt)
            .selected_text(selected_label)
            .width(360.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(*choice == CommitChoice::WorkingTree, "Working tree")
                    .clicked()
                {
                    *choice = CommitChoice::WorkingTree;
                    changed = true;
                }
                if ui
                    .selectable_label(*choice == CommitChoice::EmptyTree, "(empty)")
                    .clicked()
                {
                    *choice = CommitChoice::EmptyTree;
                    changed = true;
                }
                ui.separator();
                if let Some(Ok(cs)) = commits {
                    for c in cs {
                        let label = commit_label(c);
                        let is_selected = matches!(choice, CommitChoice::Sha(s) if s == &c.sha);
                        if ui.selectable_label(is_selected, label).clicked() {
                            *choice = CommitChoice::Sha(c.sha.clone());
                            changed = true;
                        }
                    }
                }
            });
        changed
    }

    fn refresh_commits(&mut self, tokio_handle: &Handle) {
        let git = Git::at(self.repo_root.clone());
        let path = self.repo_relative();
        let result = tokio_handle.block_on(async move {
            git.log(LogOpts {
                limit: Some(200),
                path: Some(path),
                ..Default::default()
            })
            .await
        });
        self.commits = Some(result.map_err(|e| e.to_string()));
    }

    fn load_choice(&self, choice: &CommitChoice, tokio_handle: &Handle) -> Result<String, String> {
        match choice {
            CommitChoice::EmptyTree => Ok(String::new()),
            CommitChoice::WorkingTree => {
                std::fs::read_to_string(&self.path).map_err(|e| e.to_string())
            }
            CommitChoice::Sha(sha) => {
                let git = Git::at(self.repo_root.clone());
                let path = self.repo_relative();
                let sha = sha.clone();
                let res = tokio_handle.block_on(async move { git.show_file(&sha, &path).await });
                match res {
                    Ok(s) => Ok(s),
                    Err(GitError::FileNotAtRevision { .. }) => {
                        Err("file not present at this commit".to_string())
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
        }
    }

    fn repo_relative(&self) -> PathBuf {
        self.path
            .strip_prefix(&self.repo_root)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| self.path.clone())
    }
}

fn label_for(choice: &CommitChoice, commits: &Option<Result<Vec<Commit>, String>>) -> String {
    match choice {
        CommitChoice::WorkingTree => "Working tree".into(),
        CommitChoice::EmptyTree => "(empty)".into(),
        CommitChoice::Sha(s) => {
            // If the SHA is a synthesised revspec like "<full>^" (parent),
            // surface it as-is. Otherwise look up the friendly label.
            if s.ends_with('^') {
                let parent_of = &s[..s.len() - 1];
                if let Some(Ok(cs)) = commits
                    && let Some(c) = cs.iter().find(|c| c.sha == parent_of)
                {
                    return format!("{}^ (parent of \"{}\")", short_sha(parent_of), c.subject);
                }
                return format!("{}^", short_sha(parent_of));
            }
            if let Some(Ok(cs)) = commits
                && let Some(c) = cs.iter().find(|c| c.sha == *s)
            {
                return commit_label(c);
            }
            short_sha(s)
        }
    }
}

fn commit_label(c: &Commit) -> String {
    let date = c.iso_date.get(..10).unwrap_or(&c.iso_date);
    format!(
        "{} · {} · {}",
        short_sha(&c.sha),
        date,
        truncate(&c.subject, 60)
    )
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let trimmed: String = s.chars().take(max - 1).collect();
        format!("{trimmed}…")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
enum Side {
    Left,
    Right,
}

/// One line in the rendered column. `text` is None when the diff aligns
/// this column's row with empty space (i.e. the other column has an
/// Insert/Delete here and this side is blank).
struct DiffLine {
    text: Option<String>,
    tag: ChangeTag,
}

fn classify(left: &str, right: &str) -> (Vec<DiffLine>, Vec<DiffLine>) {
    let diff = TextDiff::from_lines(left, right);
    let mut l = Vec::<DiffLine>::new();
    let mut r = Vec::<DiffLine>::new();
    for change in diff.iter_all_changes() {
        let tag = change.tag();
        let value = change.value().trim_end_matches('\n').to_string();
        match tag {
            ChangeTag::Equal => {
                l.push(DiffLine {
                    text: Some(value.clone()),
                    tag,
                });
                r.push(DiffLine {
                    text: Some(value),
                    tag,
                });
            }
            ChangeTag::Delete => {
                l.push(DiffLine {
                    text: Some(value),
                    tag,
                });
                r.push(DiffLine { text: None, tag });
            }
            ChangeTag::Insert => {
                l.push(DiffLine { text: None, tag });
                r.push(DiffLine {
                    text: Some(value),
                    tag,
                });
            }
        }
    }
    (l, r)
}

fn render_column(
    ui: &mut egui::Ui,
    lines: &[DiffLine],
    path: &std::path::Path,
    font_id: &FontId,
    side: Side,
    load_error: Option<&String>,
) {
    if let Some(msg) = load_error {
        ui.label(RichText::new(msg).italics().color(theme::muted_text()));
        return;
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for line in lines {
            let bg = bg_for(line.tag, side);
            let text = line.text.clone().unwrap_or_default();
            let highlighted = highlighter::highlight(path, &text, font_id.clone(), f32::INFINITY);
            let mut frame = egui::Frame::none().fill(bg);
            if bg == Color32::TRANSPARENT {
                frame = egui::Frame::none();
            }
            frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                if line.text.is_none() {
                    // Blank slot opposite an Insert/Delete on the other
                    // side. Render an empty line of the right height so
                    // rows align between columns.
                    ui.label(
                        egui::RichText::new(" ")
                            .font(font_id.clone())
                            .color(Color32::TRANSPARENT),
                    );
                } else {
                    let mut job = highlighted;
                    if job.sections.is_empty() {
                        // Empty line that should still take a row.
                        job.append(
                            " ",
                            0.0,
                            TextFormat {
                                font_id: font_id.clone(),
                                color: Color32::TRANSPARENT,
                                ..Default::default()
                            },
                        );
                    }
                    ui.label(job);
                }
            });
        }
    });
}

fn bg_for(tag: ChangeTag, side: Side) -> Color32 {
    match (tag, side) {
        (ChangeTag::Delete, Side::Left) => Color32::from_rgb(70, 30, 30),
        (ChangeTag::Insert, Side::Right) => Color32::from_rgb(30, 60, 35),
        _ => Color32::TRANSPARENT,
    }
}
