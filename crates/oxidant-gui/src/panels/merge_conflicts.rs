// Realises spec/components/gui/merge-conflicts.md.
//
// Centre tab that surfaces when a squash-or-no-ff merge-back returns
// conflicts. Two resolution paths per file (in-editor + git mergetool),
// then a finalize button that runs git commit + cleanup. No Rust merge-
// conflict crate involved — git already produced the standard markers,
// this is purely UX over shell-outs.

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use egui::{Color32, RichText};
use tokio::runtime::Handle;

use oxidant_vcs::Git;

use crate::app::{MergeConflictsState, SharedState};
use crate::dock::{DockTab, FileSource};
use crate::theme;

pub struct MergeConflictsPanel {
    /// Last action's status line — set by the spawned tasks (mark-
    /// resolved, finalize, abort) so the user sees feedback without
    /// the panel needing its own AgentEvent channel.
    status: Arc<StdMutex<Option<String>>>,
}

impl Default for MergeConflictsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl MergeConflictsPanel {
    pub fn new() -> Self {
        Self {
            status: Arc::new(StdMutex::new(None)),
        }
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        state: &Arc<StdMutex<SharedState>>,
        tokio_handle: &Handle,
        egui_ctx: &egui::Context,
    ) {
        // Snapshot the conflict state under a quick lock so we don't
        // hold it across the render — button actions re-lock.
        let snapshot = state.lock().unwrap().merge_conflicts.clone();
        let Some(conflicts) = snapshot else {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("No merge in progress.")
                        .color(theme::muted_text()),
                );
                ui.label(
                    RichText::new(
                        "This tab opens automatically when a merge-back hits conflicts.",
                    )
                    .color(theme::faint_text()),
                );
            });
            return;
        };

        let resolved_count = conflicts.resolved.len();
        let total = conflicts.files.len();
        ui.label(
            RichText::new(format!(
                "Merge from `{}` into `{}` — {} conflicts, {} resolved",
                conflicts.sub_branch, conflicts.target_branch, total, resolved_count,
            ))
            .strong(),
        );
        ui.label(
            RichText::new(format!("parent worktree: {}", conflicts.parent_worktree.display()))
                .color(theme::muted_text()),
        );
        ui.separator();

        for file in &conflicts.files {
            let is_resolved = conflicts.resolved.contains(file);
            ui.horizontal(|ui| {
                let marker = if is_resolved { "[✓]" } else { "[ ]" };
                let colour = if is_resolved {
                    Color32::LIGHT_GREEN
                } else {
                    Color32::from_rgb(255, 180, 100)
                };
                ui.label(RichText::new(marker).color(colour).strong());
                ui.label(file);
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui
                            .add_enabled(!is_resolved, egui::Button::new("Mark resolved"))
                            .on_hover_text("git add <file> — confirms the conflict is fixed.")
                            .clicked()
                        {
                            mark_resolved(
                                state.clone(),
                                tokio_handle,
                                egui_ctx.clone(),
                                self.status.clone(),
                                conflicts.parent_worktree.clone(),
                                file.clone(),
                            );
                        }
                        if ui
                            .button("Open in mergetool")
                            .on_hover_text(
                                "Shells out to `git mergetool` — launches your configured \
                                 external merge tool (kdiff3, meld, VS Code, vimdiff…).",
                            )
                            .clicked()
                        {
                            open_in_mergetool(
                                tokio_handle,
                                egui_ctx.clone(),
                                self.status.clone(),
                                conflicts.parent_worktree.clone(),
                                file.clone(),
                            );
                        }
                        if ui
                            .button("Open in editor")
                            .on_hover_text(
                                "Open the file with its conflict markers in a centre tab. \
                                 Edit out the <<<<<<< / ======= / >>>>>>> regions, save, \
                                 then click 'Mark resolved'.",
                            )
                            .clicked()
                        {
                            open_in_editor(state, &conflicts, file);
                            egui_ctx.request_repaint();
                        }
                    },
                );
            });
        }

        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            let all_resolved = resolved_count == total && total > 0;
            if ui
                .add_enabled(all_resolved, egui::Button::new("Finalize merge commit"))
                .on_hover_text(if all_resolved {
                    "git commit -m <message>; then remove the sub worktree + branch."
                } else {
                    "Mark every conflict resolved first."
                })
                .clicked()
            {
                finalize(
                    state.clone(),
                    tokio_handle,
                    egui_ctx.clone(),
                    self.status.clone(),
                    conflicts.clone(),
                );
            }
            if ui
                .button("Abort merge")
                .on_hover_text(
                    "Reset the parent worktree to clean state. Leaves the sub-exploration \
                     intact so you can keep working on it.",
                )
                .clicked()
            {
                abort(
                    state.clone(),
                    tokio_handle,
                    egui_ctx.clone(),
                    self.status.clone(),
                    conflicts.clone(),
                );
            }
        });

        if let Some(msg) = self.status.lock().unwrap().clone() {
            ui.add_space(4.0);
            ui.label(RichText::new(msg).color(theme::muted_text()));
        }
    }
}

fn open_in_editor(
    state: &Arc<StdMutex<SharedState>>,
    conflicts: &MergeConflictsState,
    file: &str,
) {
    let abs = conflicts.parent_worktree.join(file);
    let tab = DockTab::File {
        path: abs,
        source: FileSource::Code,
    };
    if let Ok(mut s) = state.lock() {
        s.pending_centre_tabs.push(tab);
    }
}

fn open_in_mergetool(
    tokio_handle: &Handle,
    egui_ctx: egui::Context,
    status: Arc<StdMutex<Option<String>>>,
    parent_worktree: PathBuf,
    file: String,
) {
    tokio_handle.spawn(async move {
        let result = tokio::process::Command::new("git")
            .current_dir(&parent_worktree)
            .args(["mergetool", "--no-prompt", "--", &file])
            .status()
            .await;
        match result {
            Ok(s) if s.success() => {
                *status.lock().unwrap() =
                    Some(format!("mergetool exited cleanly for `{file}` — click Mark resolved to confirm"));
            }
            Ok(s) => {
                *status.lock().unwrap() = Some(format!(
                    "mergetool exited with code {:?} for `{file}`",
                    s.code()
                ));
            }
            Err(e) => {
                tracing::error!(?e, "spawning git mergetool failed");
                *status.lock().unwrap() = Some(format!("git mergetool failed: {e}"));
            }
        }
        egui_ctx.request_repaint();
    });
}

fn mark_resolved(
    state: Arc<StdMutex<SharedState>>,
    tokio_handle: &Handle,
    egui_ctx: egui::Context,
    status: Arc<StdMutex<Option<String>>>,
    parent_worktree: PathBuf,
    file: String,
) {
    tokio_handle.spawn(async move {
        let git = Git::at(&parent_worktree);
        if let Err(e) = git.add(&[&file]).await {
            tracing::error!(?e, "git add failed");
            *status.lock().unwrap() = Some(format!("git add `{file}` failed: {e}"));
            egui_ctx.request_repaint();
            return;
        }
        if let Ok(mut s) = state.lock()
            && let Some(c) = s.merge_conflicts.as_mut()
        {
            c.resolved.insert(file.clone());
        }
        *status.lock().unwrap() = Some(format!("staged `{file}`"));
        egui_ctx.request_repaint();
    });
}

fn finalize(
    state: Arc<StdMutex<SharedState>>,
    tokio_handle: &Handle,
    egui_ctx: egui::Context,
    status: Arc<StdMutex<Option<String>>>,
    conflicts: MergeConflictsState,
) {
    tokio_handle.spawn(async move {
        let parent_git = Git::at(&conflicts.parent_worktree);
        // For squash merges we need an explicit -m message; for
        // --no-ff conflict paths git has already prepared its merge
        // message and a plain `git commit` finishes the in-flight
        // merge. We only ship squash today, so always use the message
        // path. If we later carry through --no-ff conflicts the
        // squash flag will guard us into the right branch.
        let commit_result = if conflicts.squash {
            parent_git.commit_message_only(&conflicts.message).await
        } else {
            // --no-ff finalise: just `git commit` with no flags — git
            // uses the prepared MERGE_MSG. Fall through commit_message_only
            // with an empty message would create a "commit" with empty
            // body; we need the prepared one. For MVP we still pass the
            // explicit message — if/when --no-ff lands, swap this for
            // `git commit --no-edit` or similar.
            parent_git.commit_message_only(&conflicts.message).await
        };
        let sha = match commit_result {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(?e, "finalize commit failed");
                *status.lock().unwrap() = Some(format!("commit failed: {e}"));
                egui_ctx.request_repaint();
                return;
            }
        };
        // Clean up: remove the sub worktree, delete its branch,
        // switch active back to the parent, clear merge_conflicts.
        if let Err(e) = parent_git
            .worktree_remove(&conflicts.sub_worktree, false)
            .await
        {
            tracing::warn!(?e, "post-finalize worktree_remove failed");
            *status.lock().unwrap() = Some(format!(
                "merged ({}) but worktree cleanup failed: {e}",
                short_sha(&sha)
            ));
            egui_ctx.request_repaint();
            return;
        }
        if let Err(e) = parent_git.branch_delete(&conflicts.sub_branch, true).await {
            tracing::warn!(?e, "post-finalize branch_delete failed");
            // Non-fatal: worktree's gone, branch is just an orphan ref.
        }
        {
            let mut s = state.lock().unwrap();
            s.explorations.shift_remove(&conflicts.sub_id);
            s.active_id = conflicts.parent_id;
            s.merge_conflicts = None;
        }
        *status.lock().unwrap() = Some(format!(
            "merged to `{}` ({})",
            conflicts.target_branch,
            short_sha(&sha)
        ));
        egui_ctx.request_repaint();
    });
}

fn abort(
    state: Arc<StdMutex<SharedState>>,
    tokio_handle: &Handle,
    egui_ctx: egui::Context,
    status: Arc<StdMutex<Option<String>>>,
    conflicts: MergeConflictsState,
) {
    tokio_handle.spawn(async move {
        let parent_git = Git::at(&conflicts.parent_worktree);
        // --squash doesn't set MERGE_HEAD, so `git merge --abort`
        // errors out. Use reset --hard HEAD in that case.
        let result = if conflicts.squash {
            parent_git.reset_hard_head().await
        } else {
            parent_git.merge_abort().await
        };
        match result {
            Ok(()) => {
                if let Ok(mut s) = state.lock() {
                    s.merge_conflicts = None;
                }
                *status.lock().unwrap() =
                    Some("merge aborted; sub-exploration kept intact".into());
            }
            Err(e) => {
                tracing::error!(?e, "merge abort failed");
                *status.lock().unwrap() = Some(format!("abort failed: {e}"));
            }
        }
        egui_ctx.request_repaint();
    });
}

fn short_sha(s: &str) -> String {
    s.chars().take(7).collect()
}
