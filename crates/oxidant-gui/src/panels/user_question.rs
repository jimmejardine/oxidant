// Modal that fulfils a pending `ask_user` tool call. Reads
// `PerWindowState.pending_question`, renders the question text +
// each option as a button + an optional free-form text field +
// Send. On submit, takes the pending question out of state and
// sends the answer through the held oneshot, unblocking the tool's
// awaiting future. See spec/tools/ask-user.md and
// spec/components/gui/user-question-modal.md.

use std::sync::{Arc, Mutex as StdMutex};

use egui::{Align2, RichText, TextEdit};

use oxidant_core::ExplorationId;

use crate::app::SharedState;
use crate::theme;

/// Per-window state for the modal. Carries the user's draft text
/// for the free-form field across frames, plus a "this is the id of
/// the question we last rendered" so a fresh question resets the
/// draft.
pub struct UserQuestionPanel {
    freeform_draft: String,
    /// Identity of the currently-rendered question, just by question
    /// text. Used to detect "a new question arrived, reset the draft"
    /// without exposing a real id type.
    last_question_key: Option<String>,
}

impl Default for UserQuestionPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl UserQuestionPanel {
    pub fn new() -> Self {
        Self {
            freeform_draft: String::new(),
            last_question_key: None,
        }
    }

    /// Render the modal if this window has a pending question.
    /// Returns `true` when the question was answered this frame
    /// (caller bumps repaint and clears any in-modal focus). No-op
    /// when there's nothing pending.
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        state: &Arc<StdMutex<SharedState>>,
        view_id: ExplorationId,
    ) -> bool {
        // Peek at the pending question to decide whether to render.
        // Reset our draft state on the transition into a *new*
        // question so a stale answer text doesn't leak between calls.
        let (question, options, allow_freeform) = {
            let s = state.lock().unwrap();
            let q = match s.window(view_id).and_then(|w| w.pending_question.as_ref()) {
                Some(q) => q,
                None => {
                    // Nothing pending — clear our remembered key so
                    // the next arrival is treated as fresh.
                    if self.last_question_key.is_some() {
                        self.last_question_key = None;
                        self.freeform_draft.clear();
                    }
                    return false;
                }
            };
            (q.question.clone(), q.options.clone(), q.allow_freeform)
        };

        // Detect a new question and reset the draft when it changed.
        if self.last_question_key.as_deref() != Some(question.as_str()) {
            self.freeform_draft.clear();
            self.last_question_key = Some(question.clone());
        }

        let mut chosen_answer: Option<String> = None;

        egui::Window::new("Question for you")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.label(RichText::new(&question).strong());
                ui.add_space(8.0);
                for opt in &options {
                    if ui
                        .add_sized(
                            [ui.available_width(), 0.0],
                            egui::Button::new(RichText::new(opt)),
                        )
                        .clicked()
                    {
                        chosen_answer = Some(opt.clone());
                    }
                }
                if allow_freeform {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("or type your own answer")
                            .color(theme::muted_text())
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        let resp = ui.add(
                            TextEdit::singleline(&mut self.freeform_draft)
                                .desired_width(ui.available_width() - 70.0)
                                .hint_text("…"),
                        );
                        let enter =
                            resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let clicked = ui.button("Send").clicked();
                        if (clicked || enter) && !self.freeform_draft.trim().is_empty() {
                            chosen_answer = Some(self.freeform_draft.trim().to_string());
                        }
                    });
                }
            });

        if let Some(answer) = chosen_answer {
            if let Ok(mut s) = state.lock()
                && let Some(q) = s.window_mut(view_id).pending_question.take()
            {
                // Best-effort send: if the receiver was dropped we
                // just discard. The tool call returned an error in
                // that case anyway.
                let _ = q.answer_tx.send(answer);
            }
            self.freeform_draft.clear();
            self.last_question_key = None;
            ctx.request_repaint();
            true
        } else {
            false
        }
    }
}
