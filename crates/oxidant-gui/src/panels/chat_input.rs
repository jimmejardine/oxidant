// Realises spec/components/gui/chat-input-panel.md.
//
// Bottom-docked text input. Ctrl+Enter sends; Esc cancels. The send
// path spawns a tokio task running agent_loop::run, forwarding
// ChatEvents back to the GUI via the App's mpsc channel.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use egui::{Color32, Key, Modifiers, RichText, TextEdit};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use oxidant_core::{
    AgentLoopConfig, AgentLoopOutcome, AgentMode, Conversation, ToolContext, ToolRegistry, run,
};
use oxidant_providers::{ChatEvent, Provider};

use crate::app::{AgentEvent, SharedState, TurnOutcome};
use crate::theme;

pub struct ChatInputPanel {
    draft: String,
    /// Plan vs Implement. Defaults to Plan per
    /// spec/components/core/agent-mode.md. Flipped by Shift+Tab while
    /// the text edit is focused, or by clicking the header chip.
    mode: AgentMode,
}

impl Default for ChatInputPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatInputPanel {
    pub fn new() -> Self {
        Self {
            draft: String::new(),
            mode: AgentMode::default(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        state: &Arc<StdMutex<SharedState>>,
        event_tx: &UnboundedSender<AgentEvent>,
        tokio_handle: &Handle,
        workspace_root: &Path,
        provider: &Arc<dyn Provider>,
        model: &str,
        system_prompt: Option<&str>,
        egui_ctx: &egui::Context,
    ) {
        let streaming = state.lock().unwrap().live_turn.is_some();

        // Drain pending_chat_prompt before drawing — another panel
        // (the Health Check tree, for now) may have queued an
        // "address this" prompt for us. We replace the draft, force
        // the requested mode, and grab focus so the user can review
        // and press Ctrl+Enter immediately. We never auto-send.
        // See spec/components/gui/chat-input-panel.md "External prompt fill".
        let text_edit_id = ui.make_persistent_id("oxidant-chat-input");
        let pending = state.lock().unwrap().pending_chat_prompt.take();
        if let Some(p) = pending {
            self.draft = p.prompt;
            self.mode = p.mode;
            ui.memory_mut(|m| m.request_focus(text_edit_id));
        }

        // Header row: mode chip · model · send/cancel.
        ui.horizontal(|ui| {
            let chip_clicked = render_mode_chip(ui, self.mode, streaming);
            if chip_clicked && !streaming {
                self.mode = self.mode.flip();
            }
            ui.label(RichText::new(format!("model: {model}")).color(theme::muted_text()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if streaming {
                    if ui.button("Cancel (Esc)").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        let s = state.lock().unwrap();
                        if let Some(token) = &s.cancellation {
                            token.cancel();
                        }
                    }
                } else {
                    let send = ui.button("Send ⏎ (Ctrl+Enter)");
                    let pressed_send = send.clicked()
                        || ui.input(|i| {
                            i.modifiers.command_only() && i.key_pressed(egui::Key::Enter)
                        });
                    if pressed_send && !self.draft.trim().is_empty() {
                        let prompt = std::mem::take(&mut self.draft);
                        spawn_agent(
                            prompt,
                            state.clone(),
                            event_tx.clone(),
                            tokio_handle,
                            workspace_root.to_path_buf(),
                            provider.clone(),
                            model.to_string(),
                            system_prompt.map(String::from),
                            self.mode,
                            egui_ctx.clone(),
                        );
                    }
                }
            });
        });

        // Multi-line input.
        ui.add_space(2.0);
        let id = ui.make_persistent_id("oxidant-chat-input");

        // Shift+Tab flips the mode while the text edit owns focus. We
        // MUST consume the key BEFORE the TextEdit renders — otherwise
        // egui's focus-traversal runs first during the TextEdit's
        // interaction phase, moves focus to the previous widget, and by
        // the time we'd check `edit_response.has_focus()` it's already
        // false. We instead check egui memory (previous frame's focus
        // state) for our persistent id, then remove the Shift+Tab event
        // from the input queue so the TextEdit never sees it. Plain Tab
        // (no Shift) is left alone — TextEdit handles it as a literal.
        let chat_input_focused = ui.memory(|m| m.has_focus(id));
        if chat_input_focused
            && !streaming
            && ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab))
        {
            self.mode = self.mode.flip();
        }

        // Hoist the hint out before borrowing self.draft mutably.
        let hint: &str = if streaming {
            "streaming… cancel with Esc to type a new prompt"
        } else {
            self.mode_hint()
        };
        let _edit_response = ui.add_sized(
            [ui.available_width(), ui.available_height().max(60.0)],
            TextEdit::multiline(&mut self.draft)
                .id(id)
                .desired_rows(4)
                .hint_text(hint),
        );
    }

    fn mode_hint(&self) -> &'static str {
        match self.mode {
            AgentMode::Plan => "PLAN mode · type a prompt — Ctrl+Enter to send · Shift+Tab to flip",
            AgentMode::Implement => {
                "IMPLEMENT mode · type a prompt — Ctrl+Enter to send · Shift+Tab to flip"
            }
        }
    }
}

/// Draw the mode chip (`[PLAN]` yellow, `[IMPLEMENT]` green). Returns
/// true when the chip was clicked. Disabled visually while a turn is
/// streaming — the in-flight request was sent with the prior mode and
/// flipping would mislead.
fn render_mode_chip(ui: &mut egui::Ui, mode: AgentMode, streaming: bool) -> bool {
    let (label, colour) = match mode {
        AgentMode::Plan => ("PLAN", Color32::from_rgb(255, 200, 100)),
        AgentMode::Implement => ("IMPLEMENT", Color32::LIGHT_GREEN),
    };
    let mut text = RichText::new(format!("[{label}]")).color(colour).strong();
    if streaming {
        text = text.color(theme::muted_text());
    }
    let resp = ui.add_enabled(!streaming, egui::Button::new(text).frame(false));
    resp.on_hover_text("Shift+Tab to toggle mode (or click).")
        .clicked()
}

#[allow(clippy::too_many_arguments)]
fn spawn_agent(
    prompt: String,
    state: Arc<StdMutex<SharedState>>,
    event_tx: UnboundedSender<AgentEvent>,
    tokio_handle: &Handle,
    workspace_root: PathBuf,
    provider: Arc<dyn Provider>,
    model: String,
    system_prompt: Option<String>,
    mode: AgentMode,
    egui_ctx: egui::Context,
) {
    // Append the user message, snapshot the conversation, set up
    // cancellation, then move into the tokio task.
    let cancellation = CancellationToken::new();
    let (snapshot, registry, exploration_id) = {
        let mut s = state.lock().unwrap();
        s.exploration.conversation.push_user_text(prompt);
        s.live_turn = Some(crate::app::LiveTurn::default());
        s.last_outcome = None;
        s.cancellation = Some(cancellation.clone());
        (
            s.exploration.conversation.clone(),
            s.registry.clone(),
            s.exploration.id.to_string(),
        )
    };

    tokio_handle.spawn(async move {
        let outcome = drive_agent(
            snapshot,
            registry,
            workspace_root,
            provider,
            model,
            system_prompt,
            mode,
            cancellation,
            exploration_id,
            event_tx.clone(),
            egui_ctx,
            state.clone(),
        )
        .await;
        let _ = event_tx.send(AgentEvent::Completed(outcome));
    });
}

#[allow(clippy::too_many_arguments)]
async fn drive_agent(
    snapshot: Conversation,
    registry: Arc<ToolRegistry>,
    workspace_root: PathBuf,
    provider: Arc<dyn Provider>,
    model: String,
    system_prompt: Option<String>,
    mode: AgentMode,
    cancellation: CancellationToken,
    exploration_id: String,
    event_tx: UnboundedSender<AgentEvent>,
    egui_ctx: egui::Context,
    state: Arc<StdMutex<SharedState>>,
) -> TurnOutcome {
    let mut conv = snapshot;
    let canonical = dunce::canonicalize(&workspace_root).unwrap_or(workspace_root);
    let workspace_camino = match camino::Utf8PathBuf::from_path_buf(canonical.clone()) {
        Ok(p) => p,
        Err(_) => {
            return TurnOutcome {
                stop_reason: None,
                usage: Default::default(),
                iterations: 0,
                tool_calls: 0,
                error: Some(format!("non-UTF-8 workspace path: {}", canonical.display())),
            };
        }
    };
    let ctx = ToolContext {
        workspace_root: workspace_camino,
        exploration_id,
        cancellation: cancellation.clone(),
    };

    let mut config = AgentLoopConfig::new(model);
    config.system_prompt = system_prompt;
    config.mode = mode;
    // 2048 fits comfortably under textgen-webui's default truncation
    // budget (8192 - 2048 = 6144 tokens for prompt+history) while still
    // leaving plenty of headroom for a typical assistant turn. Bump on
    // commercial providers with bigger context via settings once the
    // settings panel lands; the local-server failure mode of running
    // up against an undersized truncation_length is harder to debug
    // than a too-low max_tokens cap.
    config.max_tokens = 2048;
    config.max_iterations = 12;
    config.post_edit_check_tool = Some("spec_diff".to_string());

    let event_tx_for_loop = event_tx.clone();
    let egui_ctx_for_loop = egui_ctx.clone();

    let outcome = run(
        provider.as_ref(),
        registry,
        ctx,
        &mut conv,
        &config,
        |ev: &ChatEvent| {
            let _ = event_tx_for_loop.send(AgentEvent::Chat(ev.clone()));
            egui_ctx_for_loop.request_repaint();
        },
    )
    .await;

    // Whatever the result, the conversation must come back to the GUI so
    // the assistant's content (and any tool result messages) are visible
    // and the next user turn builds from the right history.
    {
        let mut s = state.lock().unwrap();
        s.exploration.conversation = conv;
    }
    egui_ctx.request_repaint();

    match outcome {
        Ok(AgentLoopOutcome {
            iterations,
            stop_reason,
            total_usage,
            tool_calls_dispatched,
            ..
        }) => TurnOutcome {
            stop_reason,
            usage: total_usage,
            iterations,
            tool_calls: tool_calls_dispatched,
            error: None,
        },
        Err(e) => TurnOutcome {
            stop_reason: None,
            usage: Default::default(),
            iterations: 0,
            tool_calls: 0,
            error: Some(e.to_string()),
        },
    }
}
