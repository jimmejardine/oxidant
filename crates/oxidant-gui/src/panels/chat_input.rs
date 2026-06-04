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

use futures::StreamExt;
use oxidant_core::{
    AgentLoopConfig, AgentLoopOutcome, AgentMode, ContentBlock, Conversation, Message, ToolContext,
    ToolRegistry, run,
};
use oxidant_providers::{ChatEvent, ChatRequest, ContentPart, Provider, RequestMessage, Role};

use oxidant_core::ExplorationId;

use crate::app::{AgentEvent, SharedState, TurnOutcome};
use crate::dock::DockTab;
use crate::theme;

/// Default `max_iterations` for a fresh prompt. The chat input passes
/// this into `spawn_agent_inner`. The default in
/// `AgentLoopConfig::new` is the same number — kept aligned here so
/// future tuning is a single-place edit.
pub(crate) const DEFAULT_TURN_MAX_ITERATIONS: usize = 30;
/// How much each "Continue iterating" click bumps `max_iterations` by.
/// See spec/components/gui/chat-input-panel.md "Continue iterating".
pub(crate) const CONTINUE_ITERATIONS_INCREMENT: usize = 20;

pub struct ChatInputPanel {
    draft: String,
    /// Plan vs Implement. Defaults to Plan per
    /// spec/components/core/agent-mode.md. Flipped by Shift+Tab while
    /// the text edit is focused, or by clicking the header chip.
    mode: AgentMode,
    /// Inline feedback for slash commands (e.g. "unknown command: /foo").
    /// Cleared on the next keystroke that mutates `draft`. See
    /// spec/components/gui/chat-input-panel.md "Slash commands".
    command_feedback: Option<String>,
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
            command_feedback: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        state: &Arc<StdMutex<SharedState>>,
        view_id: ExplorationId,
        event_tx: &UnboundedSender<AgentEvent>,
        tokio_handle: &Handle,
        workspace_root: &Path,
        provider: &Arc<dyn Provider>,
        model: &str,
        system_prompt: Option<&str>,
        enter_sends: bool,
        egui_ctx: &egui::Context,
    ) {
        // Drain pending_continue before drawing — clicking the
        // "Continue iterating" button in the transcript queued a
        // resume request via SharedState. Dispatch it immediately so
        // `streaming` (computed below) reflects the new in-flight
        // turn. No new user message is appended. See
        // spec/components/gui/chat-input-panel.md "Continue iterating".
        // Note: pending_continue is window-scoped — sub-windows have
        // their own `Continue iterating` affordances. The shared
        // SharedState.pending_continue is the MAIN window's; for now
        // sub-windows only read it when they're the active.
        let pending_continue = if state.lock().unwrap().active_id == view_id {
            state.lock().unwrap().pending_continue.take()
        } else {
            None
        };
        if let Some(new_max) = pending_continue {
            spawn_agent_inner(
                None,
                new_max,
                view_id,
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

        let streaming = state
            .lock()
            .unwrap()
            .window(view_id)
            .is_some_and(|w| w.live_turn.is_some());

        // Drain pending_chat_prompt before drawing — only the MAIN
        // window's chat input consumes this (the health-check panel
        // only signals to the main). Sub windows ignore it.
        let text_edit_id = ui.make_persistent_id("oxidant-chat-input");
        let pending = if state.lock().unwrap().active_id == view_id {
            state.lock().unwrap().pending_chat_prompt.take()
        } else {
            None
        };
        if let Some(p) = pending {
            self.draft = p.prompt;
            self.mode = p.mode;
            ui.memory_mut(|m| m.request_focus(text_edit_id));
        }

        // Whether the chat input held focus (last frame). Reused for both
        // the keyboard-send decision and the Shift+Tab mode toggle below.
        let chat_input_focused = ui.memory(|m| m.has_focus(text_edit_id));

        // Decide a keyboard "send" and CONSUME the key so the multiline
        // TextEdit (rendered below) doesn't also insert a newline — same
        // trick as the Shift+Tab handling. Ctrl/Cmd+Enter always sends;
        // when `enter_sends` is on, plain Enter sends too while Shift+Enter
        // (the SHIFT modifier is left untouched) still inserts a newline.
        // See spec/components/gui/chat-input-panel.md "Keybindings".
        let key_send = !streaming && chat_input_focused && {
            let mut s = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
            if enter_sends {
                s |= ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            }
            s
        };

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
                        if let Some(token) = s.window(view_id).and_then(|w| w.cancellation.as_ref())
                        {
                            token.cancel();
                        }
                    }
                } else {
                    let label = if enter_sends {
                        "Send ⏎"
                    } else {
                        "Send ⏎ (Ctrl+Enter)"
                    };
                    let pressed_send = ui.button(label).clicked() || key_send;
                    if pressed_send && !self.draft.trim().is_empty() {
                        let prompt = std::mem::take(&mut self.draft);
                        // Clear any prior command feedback as we re-submit.
                        self.command_feedback = None;
                        match crate::panels::slash_commands::parse(&prompt) {
                            crate::panels::slash_commands::ChatCommand::Clear => {
                                if let Ok(mut s) = state.lock() {
                                    s.exploration_mut(view_id).conversation.clear();
                                    let w = s.window_mut(view_id);
                                    w.last_outcome = None;
                                    w.live_turn = None;
                                }
                                egui_ctx.request_repaint();
                            }
                            crate::panels::slash_commands::ChatCommand::Compact => {
                                spawn_compact(
                                    view_id,
                                    state.clone(),
                                    event_tx.clone(),
                                    tokio_handle,
                                    provider.clone(),
                                    model.to_string(),
                                    egui_ctx.clone(),
                                );
                            }
                            crate::panels::slash_commands::ChatCommand::Unknown(name) => {
                                self.command_feedback = Some(format!("unknown command: /{name}"));
                                // Restore the draft so the user can fix the typo.
                                self.draft = prompt;
                            }
                            crate::panels::slash_commands::ChatCommand::Prompt(text) => {
                                spawn_agent_inner(
                                    Some(text.to_string()),
                                    DEFAULT_TURN_MAX_ITERATIONS,
                                    view_id,
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
                                // Bring the Transcript to the front so the
                                // streaming response is visible immediately
                                // (main window only — sub windows already
                                // show the transcript by default).
                                if let Ok(mut s) = state.lock()
                                    && s.active_id == view_id
                                    && !s.pending_centre_tabs.contains(&DockTab::Transcript)
                                {
                                    s.pending_centre_tabs.push(DockTab::Transcript);
                                }
                            }
                        }
                    }
                }
            });
        });

        // Multi-line input.
        ui.add_space(2.0);
        let id = ui.make_persistent_id("oxidant-chat-input");

        // Consume Shift+Tab BEFORE the TextEdit renders so we can flip
        // the mode without it reaching the widget's event loop. With
        // .lock_focus(true) below, the TextEdit would otherwise see
        // Shift+Tab and outdent (decrease_indentation); pulling the
        // event from the queue first means our consume_key fires and
        // the TextEdit's filtered_events returns nothing for Shift+Tab.
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
        // .lock_focus(true) is what actually suppresses egui's Tab
        // focus-traversal — it's the only way to set the EventFilter
        // at the right point in the frame (the TextEdit's interaction
        // phase). egui's Focus::begin_pass scans the raw input for Tab
        // BEFORE widgets render, so a manual set_focus_lock_filter
        // call earlier in render() is a no-op (precondition
        // `had_focus_last_frame && has_focus` fails for our id before
        // the widget has registered focus). Side-effect: plain Tab now
        // inserts a literal '\t' inside the chat input — useful for
        // pasted snippets, harmless otherwise.
        let draft_before = self.draft.clone();
        let _edit_response = ui.add_sized(
            [ui.available_width(), ui.available_height().max(60.0)],
            TextEdit::multiline(&mut self.draft)
                .id(id)
                .desired_rows(4)
                .lock_focus(true)
                .hint_text(hint),
        );
        // Clear the slash-command feedback as soon as the user edits
        // the draft so a stale "unknown command" hint doesn't linger.
        if self.command_feedback.is_some() && self.draft != draft_before {
            self.command_feedback = None;
        }
        if let Some(msg) = &self.command_feedback {
            ui.label(RichText::new(msg).color(theme::muted_text()));
        }
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

/// Spawn an agent loop turn on the conversation in `state`.
///
/// - `prompt = Some(text)` appends a user message before snapshotting —
///   the path for a fresh user prompt.
/// - `prompt = None` snapshots the conversation as-is — the path for a
///   "Continue iterating" resume, which picks up after the previous
///   turn's last `Message::ToolResult`. See
///   spec/components/gui/chat-input-panel.md "Continue iterating".
///
/// `max_iter` is the `AgentLoopConfig::max_iterations` to use for this
/// turn — `DEFAULT_TURN_MAX_ITERATIONS` for fresh prompts, the bumped
/// value for continuations.
#[allow(clippy::too_many_arguments)]
fn spawn_agent_inner(
    prompt: Option<String>,
    max_iter: usize,
    view_id: ExplorationId,
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
    let cancellation = CancellationToken::new();
    let (snapshot, registry, exploration_id) = {
        let mut s = state.lock().unwrap();
        if let Some(text) = prompt {
            s.exploration_mut(view_id).conversation.push_user_text(text);
        }
        let w = s.window_mut(view_id);
        w.live_turn = Some(crate::app::LiveTurn::default());
        w.last_outcome = None;
        w.cancellation = Some(cancellation.clone());
        (
            s.exploration(view_id).conversation.clone(),
            s.registry.clone(),
            s.exploration(view_id).id.to_string(),
        )
    };

    tokio_handle.spawn(async move {
        let outcome = drive_agent(
            snapshot,
            view_id,
            registry,
            workspace_root,
            provider,
            model,
            system_prompt,
            mode,
            max_iter,
            cancellation,
            exploration_id,
            event_tx.clone(),
            egui_ctx,
            state.clone(),
        )
        .await;
        let _ = event_tx.send(AgentEvent::Completed {
            viewport_id: view_id,
            outcome,
        });
    });
}

#[allow(clippy::too_many_arguments)]
async fn drive_agent(
    snapshot: Conversation,
    view_id: ExplorationId,
    registry: Arc<ToolRegistry>,
    workspace_root: PathBuf,
    provider: Arc<dyn Provider>,
    model: String,
    system_prompt: Option<String>,
    mode: AgentMode,
    max_iter: usize,
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
                error: Some(format!("non-UTF-8 workspace path: {}", canonical.display())),
                ..Default::default()
            };
        }
    };
    let ctx = ToolContext {
        workspace_root: workspace_camino,
        exploration_id,
        cancellation: cancellation.clone(),
        ui: Some(Arc::new(crate::ui_bridge::GuiBridge {
            state: state.clone(),
            view_id,
            egui_ctx: egui_ctx.clone(),
        })),
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
    config.max_iterations = max_iter;
    config.post_edit_check_tool = Some("spec_diff".to_string());

    let event_tx_for_loop = event_tx.clone();
    let egui_ctx_for_loop = egui_ctx.clone();
    let state_for_commit = state.clone();
    let egui_ctx_for_commit = egui_ctx.clone();

    let outcome = run(
        provider.as_ref(),
        registry,
        ctx,
        &mut conv,
        &config,
        |ev: &ChatEvent| {
            let _ = event_tx_for_loop.send(AgentEvent::Chat {
                viewport_id: view_id,
                event: ev.clone(),
            });
            egui_ctx_for_loop.request_repaint();
        },
        // Conversation commit observer: publish each push_assistant /
        // push_tool_result / post-edit push_user_text to SharedState
        // immediately so the transcript reflects in-flight tool results
        // without waiting for the whole loop to return. See
        // spec/components/core/agent-loop.md "Tool dispatch concurrency".
        //
        // We ALSO reset live_turn here. Whatever was streaming into it
        // has just been incorporated into the committed conversation;
        // leaving it populated makes the transcript render the same
        // content twice — once as Message::Assistant, once as the
        // live-turn preview. Reset to Some(LiveTurn::default()) (not
        // None) so the "{n} messages · streaming" header indicator
        // stays true across iterations; the transcript renderer guards
        // an empty placeholder so no ghost spinner appears.
        // See spec/components/gui/transcript-tab.md "Streaming".
        |conv: &Conversation| {
            if let Ok(mut s) = state_for_commit.lock() {
                s.exploration_mut(view_id).conversation = conv.clone();
                s.window_mut(view_id).live_turn = Some(crate::app::LiveTurn::default());
            }
            egui_ctx_for_commit.request_repaint();
        },
    )
    .await;

    // Whatever the result, the conversation must come back to the GUI so
    // the assistant's content (and any tool result messages) are visible
    // and the next user turn builds from the right history.
    {
        let mut s = state.lock().unwrap();
        s.exploration_mut(view_id).conversation = conv;
    }
    egui_ctx.request_repaint();

    match outcome {
        Ok(AgentLoopOutcome {
            iterations,
            stop_reason,
            total_usage,
            tool_calls_dispatched,
            cancelled,
            ..
        }) => TurnOutcome {
            stop_reason,
            usage: total_usage,
            iterations,
            tool_calls: tool_calls_dispatched,
            error: None,
            hit_max_iterations: false,
            cancelled,
        },
        Err(e) => outcome_from_loop_err(e, max_iter),
    }
}

/// Translate an `agent_loop::run` error into a `TurnOutcome`. Setting
/// `hit_max_iterations` here — based on the verbatim error prefix at
/// `oxidant-core/src/agent_loop.rs`'s "agent loop exceeded
/// max_iterations" — is what lets the transcript decide whether to
/// render the "Continue iterating" button. `max_iter` carries through
/// to `iterations` so the button's bump math (`iterations +
/// INCREMENT`) is correct.
fn outcome_from_loop_err(err: anyhow::Error, max_iter: usize) -> TurnOutcome {
    let msg = err.to_string();
    let hit_max = msg.contains("agent loop exceeded max_iterations");
    TurnOutcome {
        error: Some(msg),
        iterations: if hit_max { max_iter } else { 0 },
        hit_max_iterations: hit_max,
        ..Default::default()
    }
}

// ---------------------------------------------------------------- /compact

const COMPACTION_SYSTEM_PROMPT: &str = "You are summarising the conversation so far into a compact handover note for a future session. Preserve: the user's current goal, decisions made, key file paths discovered, open questions. Drop: verbose tool output, redundant reasoning. Output as plain prose, ~500 words max. Begin with a short heading.";

/// Dispatch `/compact`. One-shot provider call (no agent loop, no tools)
/// that streams a summary, then calls
/// `Conversation::install_compaction_summary` to advance the divider.
/// See spec/components/gui/chat-input-panel.md "Slash commands".
#[allow(clippy::too_many_arguments)]
fn spawn_compact(
    view_id: ExplorationId,
    state: Arc<StdMutex<SharedState>>,
    event_tx: UnboundedSender<AgentEvent>,
    tokio_handle: &Handle,
    provider: Arc<dyn Provider>,
    model: String,
    egui_ctx: egui::Context,
) {
    let cancellation = CancellationToken::new();
    let snapshot = {
        let mut s = state.lock().unwrap();
        let snap = s.exploration(view_id).conversation.clone();
        let w = s.window_mut(view_id);
        w.live_turn = Some(crate::app::LiveTurn::default());
        w.last_outcome = None;
        w.cancellation = Some(cancellation.clone());
        snap
    };

    let req = build_compaction_request(&snapshot, model);
    let event_tx_for_turn = event_tx.clone();
    let egui_ctx_for_turn = egui_ctx.clone();
    let state_for_turn = state.clone();

    tokio_handle.spawn(async move {
        let outcome = run_compaction(
            req,
            view_id,
            provider,
            event_tx_for_turn,
            egui_ctx_for_turn,
            state_for_turn,
            cancellation,
        )
        .await;
        let _ = event_tx.send(AgentEvent::Completed {
            viewport_id: view_id,
            outcome,
        });
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_compaction(
    req: ChatRequest,
    view_id: ExplorationId,
    provider: Arc<dyn Provider>,
    event_tx: UnboundedSender<AgentEvent>,
    egui_ctx: egui::Context,
    state: Arc<StdMutex<SharedState>>,
    cancellation: CancellationToken,
) -> TurnOutcome {
    let mut stream = match provider.chat(req).await {
        Ok(s) => s,
        Err(e) => {
            return TurnOutcome {
                error: Some(format!("compaction request failed: {e}")),
                ..Default::default()
            };
        }
    };
    let mut text = String::new();
    let mut usage = oxidant_providers::Usage::default();
    let mut stop_reason = None;
    loop {
        // Race the next chunk against cancellation so ESC interrupts a
        // long compaction stream too.
        let event = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return TurnOutcome { usage, cancelled: true, ..Default::default() };
            }
            ev = stream.next() => match ev {
                Some(e) => e,
                None => break,
            },
        };
        let _ = event_tx.send(AgentEvent::Chat {
            viewport_id: view_id,
            event: event.clone(),
        });
        egui_ctx.request_repaint();
        match event {
            ChatEvent::TextDelta(s) => text.push_str(&s),
            ChatEvent::Finish {
                stop_reason: sr,
                usage: u,
            } => {
                stop_reason = Some(sr);
                usage = u;
            }
            ChatEvent::Error(e) => {
                return TurnOutcome {
                    usage,
                    error: Some(e),
                    ..Default::default()
                };
            }
            _ => {}
        }
    }
    if text.trim().is_empty() {
        return TurnOutcome {
            stop_reason,
            usage,
            error: Some("compaction returned an empty summary".to_string()),
            ..Default::default()
        };
    }
    if let Ok(mut s) = state.lock() {
        s.exploration_mut(view_id)
            .conversation
            .install_compaction_summary(text);
        s.window_mut(view_id).live_turn = None;
    }
    egui_ctx.request_repaint();
    TurnOutcome {
        stop_reason,
        usage,
        ..Default::default()
    }
}

fn build_compaction_request(conv: &Conversation, model: String) -> ChatRequest {
    let mut messages = Vec::<RequestMessage>::new();
    for msg in conv.live_messages() {
        match msg {
            Message::User { content } => {
                let parts: Vec<ContentPart> = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(ContentPart::Text(t.clone())),
                        _ => None,
                    })
                    .collect();
                if !parts.is_empty() {
                    messages.push(RequestMessage {
                        role: Role::User,
                        content: parts,
                    });
                }
            }
            Message::Assistant { content, .. } => {
                let parts: Vec<ContentPart> = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(ContentPart::Text(t.clone())),
                        _ => None,
                    })
                    .collect();
                if !parts.is_empty() {
                    messages.push(RequestMessage {
                        role: Role::Assistant,
                        content: parts,
                    });
                }
            }
            // Tool results carry raw blobs; the summary doesn't need them.
            Message::ToolResult { .. } => {}
        }
    }
    messages.push(RequestMessage {
        role: Role::User,
        content: vec![ContentPart::Text(
            "Please summarise the conversation above into a compact handover note.".to_string(),
        )],
    });
    ChatRequest {
        model,
        system: Some(COMPACTION_SYSTEM_PROMPT.to_string()),
        messages,
        tools: Vec::new(),
        max_tokens: 1024,
        temperature: Some(0.3),
        thinking: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_from_loop_err_marks_max_iterations_hit() {
        let err = anyhow::anyhow!("agent loop exceeded max_iterations (30)");
        let out = outcome_from_loop_err(err, 30);
        assert!(out.hit_max_iterations);
        assert_eq!(out.iterations, 30);
        assert!(out.error.as_deref().unwrap().contains("max_iterations"));
    }

    #[test]
    fn outcome_from_loop_err_leaves_unrelated_errors_alone() {
        let err = anyhow::anyhow!("provider returned 503: rate limited");
        let out = outcome_from_loop_err(err, 30);
        assert!(!out.hit_max_iterations);
        // iterations stays 0 for non-max-iter failures so the
        // transcript doesn't mis-render a bogus "n iterations" count.
        assert_eq!(out.iterations, 0);
        assert_eq!(
            out.error.as_deref().unwrap(),
            "provider returned 503: rate limited"
        );
    }
}
