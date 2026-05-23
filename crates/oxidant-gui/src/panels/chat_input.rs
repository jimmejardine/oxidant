// Realises spec/components/gui/chat-input-panel.md.
//
// Bottom-docked text input. Ctrl+Enter sends; Esc cancels. The send
// path spawns a tokio task running agent_loop::run, forwarding
// ChatEvents back to the GUI via the App's mpsc channel.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use egui::{RichText, TextEdit};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use oxidant_core::{
    AgentLoopConfig, AgentLoopOutcome, Conversation, ToolContext, ToolRegistry, run,
};
use oxidant_providers::{ChatEvent, Provider};

use crate::app::{AgentEvent, SharedState, TurnOutcome};
use crate::theme;

pub struct ChatInputPanel {
    draft: String,
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

        // Header row: model + send/cancel buttons.
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("model: {model}")).color(theme::muted_text()),
            );
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
                            egui_ctx.clone(),
                        );
                    }
                }
            });
        });

        // Multi-line input.
        ui.add_space(2.0);
        let id = ui.make_persistent_id("oxidant-chat-input");
        let _ = ui.add_sized(
            [ui.available_width(), ui.available_height().max(60.0)],
            TextEdit::multiline(&mut self.draft)
                .id(id)
                .desired_rows(4)
                .hint_text(if streaming {
                    "streaming… cancel with Esc to type a new prompt"
                } else {
                    "type a prompt — Ctrl+Enter to send"
                }),
        );
    }
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
    config.max_tokens = 4096;
    config.max_iterations = 12;
    config.post_edit_check_tool = Some("spec_diff".to_string());

    let event_tx_for_loop = event_tx.clone();
    let egui_ctx_for_loop = egui_ctx.clone();

    let outcome = run(
        provider.as_ref(),
        registry.as_ref(),
        &ctx,
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
