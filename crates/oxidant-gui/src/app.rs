// Top-level eframe::App. Owns shared state, runs the per-frame update,
// drains agent events from the tokio runtime, and dispatches the dock
// tabs to their respective panel renderers.

use std::sync::{Arc, Mutex as StdMutex};

use egui_dock::{DockArea, DockState, Style};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use oxidant_core::{Conversation, ToolRegistry};
use oxidant_providers::{ChatEvent, Provider, StopReason, Usage};

use crate::dock::{DockTab, default_layout};
use crate::panels::{
    chat_input::ChatInputPanel,
    diagnostic::DiagnosticPanel,
    exploration_list::ExplorationListPanel,
    file_tab::FileTabPanel,
    spec_tree::SpecTreePanel,
    transcript::TranscriptPanel,
};
use crate::viewport::ViewportConfig;

pub struct App {
    config: ViewportConfig,
    dock: DockState<DockTab>,
    state: Arc<StdMutex<SharedState>>,
    event_rx: UnboundedReceiver<AgentEvent>,
    event_tx: UnboundedSender<AgentEvent>,
    chat_panel: ChatInputPanel,
    spec_panel: SpecTreePanel,
}

/// The mutable bits shared between the GUI thread and the agent task.
/// Locks are held briefly; long async work happens on cloned data and
/// streams results back via the AgentEvent channel.
pub struct SharedState {
    pub conv: Conversation,
    pub registry: Arc<ToolRegistry>,
    pub live_turn: Option<LiveTurn>,
    pub last_outcome: Option<TurnOutcome>,
    pub cancellation: Option<CancellationToken>,
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct LiveTurn {
    pub text: String,
    pub thinking: String,
    pub tool_calls: Vec<LiveToolCall>,
}

#[derive(Debug, Clone)]
pub struct LiveToolCall {
    pub id: String,
    pub name: String,
    pub input_buffer: String,
    pub finished: bool,
}

#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub stop_reason: Option<StopReason>,
    pub usage: Usage,
    pub iterations: usize,
    pub tool_calls: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiagnosticEntry {
    pub file: String,
    pub line: u32,
    pub character: u32,
    pub message: String,
    pub severity: String,
}

/// Messages flowing from the agent task back to the GUI thread.
pub enum AgentEvent {
    /// A raw streaming event from the provider; the GUI accumulates these
    /// into LiveTurn.
    Chat(ChatEvent),
    /// The agent loop finished. Whether successful or not, the live turn
    /// is committed into the conversation by the time this fires (the
    /// agent task did that before sending).
    Completed(TurnOutcome),
}

impl App {
    pub fn new(config: ViewportConfig) -> Self {
        let mut registry = ToolRegistry::new();
        oxidant_tools::register_standard_tools(&mut registry);
        oxidant_rust_tools::register_standard_tools(&mut registry);
        oxidant_spec_tools::register_standard_tools(&mut registry);
        oxidant_vcs::register_standard_tools(&mut registry);

        let state = Arc::new(StdMutex::new(SharedState {
            conv: Conversation::new(),
            registry: Arc::new(registry),
            live_turn: None,
            last_outcome: None,
            cancellation: None,
            diagnostics: Vec::new(),
        }));
        let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();

        let spec_panel = SpecTreePanel::new(config.workspace_root.clone());
        Self {
            chat_panel: ChatInputPanel::new(),
            spec_panel,
            config,
            dock: default_layout(),
            state,
            event_rx,
            event_tx,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain agent events into shared state. ctx.request_repaint() to
        // keep streaming smooth.
        let mut any_event = false;
        while let Ok(ev) = self.event_rx.try_recv() {
            any_event = true;
            let mut state = self.state.lock().unwrap();
            apply_event(&mut state, ev);
        }
        if any_event {
            ctx.request_repaint();
        } else if self.state.lock().unwrap().live_turn.is_some() {
            // Keep redrawing while a turn is in flight even between events,
            // so the spinner stays animated.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }

        // Top menu bar (minimal: just the title).
        egui::TopBottomPanel::top("oxidant-menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("oxidant").strong());
                ui.separator();
                let state = self.state.lock().unwrap();
                let n = state.conv.len();
                let live = if state.live_turn.is_some() {
                    " · streaming"
                } else {
                    ""
                };
                ui.label(format!("{n} messages{live}"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "{} · {}",
                        self.config.model,
                        self.config.workspace_root.display()
                    ));
                });
            });
        });

        // Dock area.
        let mut viewer = TabViewer {
            state: self.state.clone(),
            chat_panel: &mut self.chat_panel,
            spec_panel: &mut self.spec_panel,
            event_tx: self.event_tx.clone(),
            tokio_handle: self.config.tokio_handle.clone(),
            workspace_root: self.config.workspace_root.clone(),
            provider: self.config.provider.clone(),
            model: self.config.model.clone(),
            system_prompt: self.config.system_prompt.clone(),
            egui_ctx: ctx.clone(),
        };
        DockArea::new(&mut self.dock)
            .style(Style::from_egui(ctx.style().as_ref()))
            .show(ctx, &mut viewer);
    }
}

fn apply_event(state: &mut SharedState, ev: AgentEvent) {
    match ev {
        AgentEvent::Chat(c) => {
            let turn = state.live_turn.get_or_insert_with(LiveTurn::default);
            match c {
                ChatEvent::TextDelta(s) => turn.text.push_str(&s),
                ChatEvent::ThinkingDelta(s) => turn.thinking.push_str(&s),
                ChatEvent::ToolUseStart { id, name } => {
                    turn.tool_calls.push(LiveToolCall {
                        id,
                        name,
                        input_buffer: String::new(),
                        finished: false,
                    });
                }
                ChatEvent::ToolUseInputDelta { id, json_delta } => {
                    if let Some(tc) = turn.tool_calls.iter_mut().find(|t| t.id == id) {
                        tc.input_buffer.push_str(&json_delta);
                    }
                }
                ChatEvent::ToolUseEnd { id } => {
                    if let Some(tc) = turn.tool_calls.iter_mut().find(|t| t.id == id) {
                        tc.finished = true;
                    }
                }
                ChatEvent::Finish { .. } | ChatEvent::Error(_) => {
                    // Recorded via Completed; nothing to do here.
                }
            }
        }
        AgentEvent::Completed(outcome) => {
            state.live_turn = None;
            state.last_outcome = Some(outcome);
            state.cancellation = None;
        }
    }
}

pub(crate) struct TabViewer<'a> {
    pub state: Arc<StdMutex<SharedState>>,
    pub chat_panel: &'a mut ChatInputPanel,
    pub spec_panel: &'a mut SpecTreePanel,
    pub event_tx: UnboundedSender<AgentEvent>,
    pub tokio_handle: Handle,
    pub workspace_root: std::path::PathBuf,
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub system_prompt: Option<String>,
    pub egui_ctx: egui::Context,
}

impl<'a> egui_dock::TabViewer for TabViewer<'a> {
    type Tab = DockTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            DockTab::Transcript => {
                let state = self.state.lock().unwrap();
                TranscriptPanel.render(ui, &state);
            }
            DockTab::SpecTree => {
                self.spec_panel.render(ui);
            }
            DockTab::ExplorationList => {
                ExplorationListPanel.render(ui, &self.workspace_root);
            }
            DockTab::DiagnosticPreview => {
                let state = self.state.lock().unwrap();
                DiagnosticPanel.render(ui, &state);
            }
            DockTab::ChatInput => {
                self.chat_panel.render(
                    ui,
                    &self.state,
                    &self.event_tx,
                    &self.tokio_handle,
                    &self.workspace_root,
                    &self.provider,
                    &self.model,
                    self.system_prompt.as_deref(),
                    &self.egui_ctx,
                );
            }
            DockTab::File { path, source } => {
                FileTabPanel.render(ui, path, *source, &self.workspace_root);
            }
        }
    }
}
