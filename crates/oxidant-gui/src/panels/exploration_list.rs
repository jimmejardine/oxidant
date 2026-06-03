// Realises spec/components/gui/exploration-list.md.
//
// Lists every exploration in this window: Main first, then Sub-
// explorations in spawn order. Lets the user spawn a fresh sub
// (worktree + branch), switch the active exploration (row click),
// and squash-merge a Sub back into its parent. Conflict UX is
// Phase 3; for MVP a merge that produces conflicts leaves the
// worktree in its half-merged state and surfaces a one-line warning.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use egui::{Color32, CursorIcon, RichText, Sense};
use tokio::runtime::Handle;

use oxidant_core::{Exploration, ExplorationId, ExplorationKind};
use oxidant_vcs::{MergeBackOpts, SpawnOpts, WorktreeHandle, worktree};

use crate::app::SharedState;
use crate::theme;

pub struct ExplorationListPanel {
    /// Form-field text for the spawn-new dialog. Drained on submit.
    new_name: String,
    /// One-shot status message — last spawn/merge/discard result.
    /// Set by spawned tasks via the shared Arc; rendered at the
    /// bottom of the panel. None hides the line.
    status: Arc<StdMutex<Option<String>>>,
}

impl Default for ExplorationListPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ExplorationListPanel {
    pub fn new() -> Self {
        Self {
            new_name: String::new(),
            status: Arc::new(StdMutex::new(None)),
        }
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        state: &Arc<StdMutex<SharedState>>,
        workspace_root: &Path,
        tokio_handle: &Handle,
        egui_ctx: &egui::Context,
    ) {
        ui.label(RichText::new("explorations").strong());
        ui.separator();

        // Spawn form: name input + button. Submitting kicks off the
        // worktree creation on a tokio task; the new row appears
        // when the task locks SharedState back.
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.new_name)
                    .hint_text("new exploration name")
                    .desired_width(ui.available_width() - 90.0),
            );
            let spawn_clicked = ui.button("+ Spawn").clicked();
            let submit = spawn_clicked
                || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if submit && !self.new_name.trim().is_empty() {
                let name = std::mem::take(&mut self.new_name);
                spawn_sub(
                    state.clone(),
                    workspace_root.to_path_buf(),
                    tokio_handle,
                    egui_ctx.clone(),
                    self.status.clone(),
                    name,
                );
            }
        });
        ui.add_space(4.0);

        // Render one row per exploration. Snapshot the list so we
        // release the lock before iterating; row actions re-lock as
        // needed.
        let (rows, active_id, parents_by_id) = {
            let s = state.lock().unwrap();
            let active = s.active_id;
            let parents: std::collections::HashMap<ExplorationId, (String, ExplorationId)> = s
                .explorations
                .iter()
                .filter_map(|(id, e)| match e.kind {
                    ExplorationKind::Sub { parent_id } => {
                        let parent_branch = s
                            .explorations
                            .get(&parent_id)
                            .map(|p| p.branch.clone())
                            .unwrap_or_else(|| "main".into());
                        Some((*id, (parent_branch, parent_id)))
                    }
                    ExplorationKind::Main => None,
                })
                .collect();
            let rows: Vec<RowSnapshot> = s
                .explorations
                .iter()
                .map(|(id, e)| RowSnapshot {
                    id: *id,
                    kind: match e.kind {
                        ExplorationKind::Main => RowKind::Main,
                        ExplorationKind::Sub { .. } => RowKind::Sub,
                    },
                    branch: e.branch.clone(),
                    worktree_path: e.worktree_path.clone(),
                    created_at_rfc: e.created_at.to_rfc3339(),
                })
                .collect();
            (rows, active, parents)
        };

        for row in rows {
            let is_active = row.id == active_id;
            let (badge, badge_colour) = match row.kind {
                RowKind::Main => ("[main]", Color32::LIGHT_GREEN),
                RowKind::Sub => ("[sub]", Color32::from_rgb(255, 200, 100)),
            };
            let active_dot = if is_active { "● " } else { "  " };

            ui.horizontal(|ui| {
                let row_resp = ui.add(
                    egui::SelectableLabel::new(
                        is_active,
                        RichText::new(format!("{active_dot}{badge}")).color(badge_colour),
                    ),
                );
                let row_resp = row_resp.on_hover_cursor(CursorIcon::PointingHand);
                let branch_label = ui.add(
                    egui::Label::new(RichText::new(&row.branch).strong())
                        .sense(Sense::click()),
                );
                let branch_label = branch_label.on_hover_cursor(CursorIcon::PointingHand);
                if (row_resp.clicked() || branch_label.clicked()) && !is_active {
                    if let Ok(mut s) = state.lock() {
                        s.active_id = row.id;
                    }
                    egui_ctx.request_repaint();
                }

                // Per-Sub actions: Merge (squash), Discard.
                if matches!(row.kind, RowKind::Sub) {
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui
                                .button("Discard")
                                .on_hover_text(
                                    "git worktree remove + delete branch. \
                                     Refuses if the worktree has uncommitted changes.",
                                )
                                .clicked()
                            {
                                discard_sub(
                                    state.clone(),
                                    workspace_root.to_path_buf(),
                                    tokio_handle,
                                    egui_ctx.clone(),
                                    self.status.clone(),
                                    row.id,
                                    row.worktree_path.clone(),
                                );
                            }
                            if let Some((parent_branch, parent_id)) = parents_by_id.get(&row.id)
                                && ui
                                    .button("Merge ↩ squash")
                                    .on_hover_text(
                                        "git merge --squash + commit, then \
                                         remove the worktree. Conflicts (Phase 3) \
                                         will surface in a resolution panel.",
                                    )
                                    .clicked()
                            {
                                merge_back_squash(
                                    state.clone(),
                                    workspace_root.to_path_buf(),
                                    tokio_handle,
                                    egui_ctx.clone(),
                                    self.status.clone(),
                                    row.id,
                                    *parent_id,
                                    parent_branch.clone(),
                                    WorktreeHandle {
                                        path: row.worktree_path.clone(),
                                        branch: row.branch.clone(),
                                        // worktree::merge_back only reads `path`
                                        // and `branch` from this — created_at is
                                        // irrelevant. Reconstruct from row data.
                                        created_at: row.created_at_rfc.parse().unwrap_or_else(
                                            |_| chrono::Utc::now(),
                                        ),
                                    },
                                );
                            }
                        },
                    );
                }
            });
            ui.label(
                RichText::new(row.worktree_path.to_string_lossy())
                    .color(theme::muted_text())
                    .small(),
            );
            ui.add_space(2.0);
        }

        // Status line. Tasks write here; we render whatever's there.
        if let Some(msg) = self.status.lock().unwrap().clone() {
            ui.add_space(4.0);
            ui.separator();
            ui.label(RichText::new(msg).color(theme::muted_text()).small());
        }
    }
}

#[derive(Clone)]
struct RowSnapshot {
    id: ExplorationId,
    kind: RowKind,
    branch: String,
    worktree_path: PathBuf,
    created_at_rfc: String,
}

#[derive(Clone, Copy)]
enum RowKind {
    Main,
    Sub,
}

fn spawn_sub(
    state: Arc<StdMutex<SharedState>>,
    workspace_root: PathBuf,
    tokio_handle: &Handle,
    egui_ctx: egui::Context,
    status: Arc<StdMutex<Option<String>>>,
    name: String,
) {
    // Spawn from the currently-active exploration's worktree so the new
    // branch starts from that exploration's HEAD (matches spec).
    let (parent_id, parent_worktree) = {
        let s = state.lock().unwrap();
        (s.active_id, s.active().worktree_path.clone())
    };
    tokio_handle.spawn(async move {
        let opts = SpawnOpts {
            name: Some(name.clone()),
            base: None,
            branch_prefix: None,
        };
        match worktree::spawn(&parent_worktree, opts).await {
            Ok(handle) => {
                let new = Exploration::from_sub_worktree(parent_id, &handle.path, &handle.branch);
                let new_id = new.id;
                {
                    let mut s = state.lock().unwrap();
                    s.explorations.insert(new_id, new);
                    s.active_id = new_id;
                }
                *status.lock().unwrap() =
                    Some(format!("spawned `{}` on branch `{}`", name, handle.branch));
                // Workspace_root is only used for the "discard from main"
                // operation later; ignored here.
                let _ = &workspace_root;
                egui_ctx.request_repaint();
            }
            Err(e) => {
                tracing::error!(?e, "worktree::spawn failed");
                *status.lock().unwrap() = Some(format!("spawn `{name}` failed: {e}"));
                egui_ctx.request_repaint();
            }
        }
    });
}

fn merge_back_squash(
    state: Arc<StdMutex<SharedState>>,
    _workspace_root: PathBuf,
    tokio_handle: &Handle,
    egui_ctx: egui::Context,
    status: Arc<StdMutex<Option<String>>>,
    sub_id: ExplorationId,
    parent_id: ExplorationId,
    parent_branch: String,
    sub_handle: WorktreeHandle,
) {
    // The merge runs in the parent's worktree, not the sub. Look it up.
    let parent_worktree = {
        let s = state.lock().unwrap();
        s.explorations
            .get(&parent_id)
            .map(|e| e.worktree_path.clone())
    };
    let Some(parent_worktree) = parent_worktree else {
        *status.lock().unwrap() = Some("merge failed: parent exploration missing".into());
        egui_ctx.request_repaint();
        return;
    };
    tokio_handle.spawn(async move {
        let outcome = match worktree::merge_back(
            &parent_worktree,
            &sub_handle,
            &parent_branch,
            MergeBackOpts {
                squash: true,
                message: None,
            },
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                tracing::error!(?e, "worktree::merge_back failed");
                *status.lock().unwrap() = Some(format!("merge failed: {e}"));
                egui_ctx.request_repaint();
                return;
            }
        };
        if !outcome.conflicts.is_empty() {
            // Phase 3 will turn this into a resolution panel. For
            // MVP, just surface the count + names; the worktree is
            // left in its half-merged state for the user to either
            // resolve by hand or run `git reset --hard HEAD` in.
            let n = outcome.conflicts.len();
            let listed = outcome.conflicts.join(", ");
            *status.lock().unwrap() = Some(format!(
                "merge produced {n} conflict(s): {listed} — resolve in the parent worktree, \
                 then `git commit` to finalise. (Conflict UX lands in Phase 3.)"
            ));
            egui_ctx.request_repaint();
            return;
        }
        // Clean up the sub worktree + branch.
        if let Err(e) = worktree::discard(&parent_worktree, &sub_handle.path, false).await {
            tracing::warn!(?e, "post-merge discard failed (leaving worktree on disk)");
            *status.lock().unwrap() = Some(format!(
                "merged to `{}` ({}), but worktree cleanup failed: {e}",
                parent_branch,
                outcome.commit_sha.as_deref().unwrap_or("<sha?>")
            ));
            egui_ctx.request_repaint();
            return;
        }
        {
            let mut s = state.lock().unwrap();
            s.explorations.shift_remove(&sub_id);
            s.active_id = parent_id;
        }
        *status.lock().unwrap() = Some(format!(
            "merged to `{}` ({})",
            parent_branch,
            outcome.commit_sha.as_deref().unwrap_or("<sha?>")
        ));
        egui_ctx.request_repaint();
    });
}

fn discard_sub(
    state: Arc<StdMutex<SharedState>>,
    _workspace_root: PathBuf,
    tokio_handle: &Handle,
    egui_ctx: egui::Context,
    status: Arc<StdMutex<Option<String>>>,
    sub_id: ExplorationId,
    sub_path: PathBuf,
) {
    // Discard happens via the parent (Main) — it's the manager of the
    // worktree list. Look up the parent's path through the sub's
    // parent_id chain.
    let (parent_worktree, parent_id) = {
        let s = state.lock().unwrap();
        let sub = match s.explorations.get(&sub_id) {
            Some(e) => e,
            None => {
                *status.lock().unwrap() =
                    Some("discard failed: exploration not found".into());
                egui_ctx.request_repaint();
                return;
            }
        };
        let parent_id = match sub.kind {
            ExplorationKind::Sub { parent_id } => parent_id,
            ExplorationKind::Main => {
                *status.lock().unwrap() = Some("cannot discard the Main exploration".into());
                egui_ctx.request_repaint();
                return;
            }
        };
        let parent_worktree = s
            .explorations
            .get(&parent_id)
            .map(|e| e.worktree_path.clone());
        (parent_worktree, parent_id)
    };
    let Some(parent_worktree) = parent_worktree else {
        *status.lock().unwrap() = Some("discard failed: parent missing".into());
        egui_ctx.request_repaint();
        return;
    };
    tokio_handle.spawn(async move {
        if let Err(e) = worktree::discard(&parent_worktree, &sub_path, false).await {
            tracing::error!(?e, "worktree::discard failed");
            *status.lock().unwrap() = Some(format!("discard failed: {e}"));
            egui_ctx.request_repaint();
            return;
        }
        {
            let mut s = state.lock().unwrap();
            s.explorations.shift_remove(&sub_id);
            if s.active_id == sub_id {
                s.active_id = parent_id;
            }
        }
        *status.lock().unwrap() = Some(format!("discarded `{}`", sub_path.display()));
        egui_ctx.request_repaint();
    });
}
