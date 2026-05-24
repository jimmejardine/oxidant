// Top-level eframe::App. Owns shared state, runs the per-frame update,
// drains agent events from the tokio runtime, and dispatches the dock
// tabs to their respective panel renderers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::SystemTime;

use egui_dock::{DockArea, DockState, Style};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use oxidant_core::{Exploration, ToolRegistry};
use oxidant_providers::{ChatEvent, Provider, StopReason, Usage};

use crate::dock::{
    DockTab, default_layout, is_tab_open, open_in_centre, open_tab, reset_layout_preserving_files,
    singleton_tabs,
};
use crate::panels::{
    chat_input::ChatInputPanel, diagnostic::DiagnosticPanel, diff_history::DiffHistoryPanel,
    exploration_list::ExplorationListPanel, file_tab::FileTabPanel, file_tree::FileTreePanel,
    settings::SettingsPanel, spec_tree::SpecTreePanel, transcript::TranscriptPanel,
};
use crate::theme::Theme;
use crate::viewport::ViewportConfig;

pub struct App {
    config: ViewportConfig,
    dock: DockState<DockTab>,
    state: Arc<StdMutex<SharedState>>,
    event_rx: UnboundedReceiver<AgentEvent>,
    event_tx: UnboundedSender<AgentEvent>,
    chat_panel: ChatInputPanel,
    spec_panel: SpecTreePanel,
    file_tree_panel: FileTreePanel,
    diag_panel: DiagnosticPanel,
    settings_panel: SettingsPanel,
    /// One DiffHistory panel per open path. Lazily inserted when the tab
    /// first paints; entries leak across tab close in MVP (cheap state,
    /// at most a handful per session). See
    /// spec/components/gui/diff-history-panel.md.
    diff_history_panels: HashMap<PathBuf, DiffHistoryPanel>,
    /// Currently-active theme. Mirrors what `theme::apply` recorded;
    /// kept here so the View → Theme submenu can render the radio
    /// state without a global lock.
    active_theme: Theme,
}

/// The mutable bits shared between the GUI thread and the agent task.
/// Locks are held briefly; long async work happens on cloned data and
/// streams results back via the AgentEvent channel.
pub struct SharedState {
    /// The Main exploration this window is bound to. Conversation, branch,
    /// worktree, and (lazily) the LSP handle live here. Sub-exploration
    /// windows will each own their own `Exploration`.
    pub exploration: Exploration,
    pub registry: Arc<ToolRegistry>,
    pub live_turn: Option<LiveTurn>,
    pub last_outcome: Option<TurnOutcome>,
    /// Per-turn cancellation token (Esc / Cancel button). Distinct from
    /// `exploration.cancellation`, which tears down the whole window.
    pub cancellation: Option<CancellationToken>,
    pub diagnostics: Vec<DiagnosticEntry>,
    /// Centre-tab opens requested by a panel that doesn't own the dock
    /// (e.g. spec-tree double-click). Drained once per frame after
    /// `DockArea::show`; see spec/components/gui/spec-tree-panel.md.
    pub pending_centre_tabs: Vec<DockTab>,
    /// Per-path edit buffers for File tabs. Keyed by absolute path so
    /// the same file in two tabs (which can't actually happen — dock
    /// tabs are unique) would share state. See
    /// spec/components/gui/file-tabs.md "Edit lifecycle for specs".
    pub editor_buffers: HashMap<PathBuf, EditorBuffer>,
}

#[derive(Debug, Clone)]
pub struct EditorBuffer {
    pub text: String,
    pub dirty: bool,
    /// Filesystem mtime at the moment we loaded `text`. Used to detect
    /// "the agent edited the file underneath this tab" — when the
    /// current mtime differs from this, a reload banner shows.
    pub mtime_at_load: Option<SystemTime>,
    /// Most recent save error, if any. Cleared on next successful save.
    pub last_save_error: Option<String>,
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
    /// Build the "Window" menu — one entry per singleton dock tab, plus
    /// a Reset layout action. See spec/components/gui/dock-layout.md.
    fn render_window_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Window", |ui| {
            for tab in singleton_tabs() {
                let open = is_tab_open(&self.dock, &tab);
                let label = tab.title();
                let resp = ui.add_enabled(
                    !open,
                    egui::Button::new(if open {
                        format!("✔ {label}")
                    } else {
                        format!("    {label}")
                    }),
                );
                if resp.clicked() {
                    open_tab(&mut self.dock, tab);
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui.button("Reset layout").clicked() {
                self.dock = reset_layout_preserving_files(&self.dock);
                ui.close_menu();
            }
        });
    }

    pub fn new(config: ViewportConfig) -> Self {
        let mut registry = ToolRegistry::new();
        oxidant_tools::register_standard_tools(&mut registry);
        oxidant_rust_tools::register_standard_tools(&mut registry);
        oxidant_spec_tools::register_standard_tools(&mut registry);
        oxidant_vcs::register_standard_tools(&mut registry);

        let exploration = Exploration::new_main(config.workspace_root.clone(), "main");
        let state = Arc::new(StdMutex::new(SharedState {
            exploration,
            registry: Arc::new(registry),
            live_turn: None,
            last_outcome: None,
            cancellation: None,
            diagnostics: Vec::new(),
            pending_centre_tabs: Vec::new(),
            editor_buffers: HashMap::new(),
        }));
        let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();

        let spec_panel = SpecTreePanel::new(config.workspace_root.clone());
        let file_tree_panel = FileTreePanel::new(config.workspace_root.clone());
        let active_theme = config.theme;
        let settings_panel = SettingsPanel::new(&config.settings);
        Self {
            chat_panel: ChatInputPanel::new(),
            spec_panel,
            file_tree_panel,
            diag_panel: DiagnosticPanel::new(),
            settings_panel,
            diff_history_panels: HashMap::new(),
            config,
            dock: default_layout(),
            state,
            event_rx,
            event_tx,
            active_theme,
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

        // Top menu bar: title + Window menu + status + model.
        egui::TopBottomPanel::top("oxidant-menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.label(egui::RichText::new("oxidant").strong());
                ui.separator();
                self.render_window_menu(ui);
                ui.separator();
                let state = self.state.lock().unwrap();
                let n = state.exploration.conversation.len();
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
            file_tree_panel: &mut self.file_tree_panel,
            diag_panel: &mut self.diag_panel,
            settings_panel: &mut self.settings_panel,
            diff_history_panels: &mut self.diff_history_panels,
            settings: self.config.settings.clone(),
            active_theme: &mut self.active_theme,
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

        // Drain any tab-open requests pushed by panels during render
        // (the spec tree's double-click handler is the main caller).
        // We do this AFTER DockArea::show because that's where panels
        // ran their handlers; pushing now applies on the next frame.
        let pending: Vec<DockTab> = {
            let mut s = self.state.lock().unwrap();
            std::mem::take(&mut s.pending_centre_tabs)
        };
        if !pending.is_empty() {
            for tab in pending {
                // Pending tabs are always centre-area opens (the only
                // current caller is the spec-tree double-click, which
                // wants the new tab next to Transcript, not on the
                // left next to the spec tree itself).
                open_in_centre(&mut self.dock, tab);
            }
            ctx.request_repaint();
        }
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
    pub file_tree_panel: &'a mut FileTreePanel,
    pub diag_panel: &'a mut DiagnosticPanel,
    pub settings_panel: &'a mut SettingsPanel,
    pub diff_history_panels: &'a mut HashMap<PathBuf, DiffHistoryPanel>,
    pub settings: Arc<StdMutex<oxidant_config::Settings>>,
    pub active_theme: &'a mut Theme,
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
                self.spec_panel.render(ui, &self.state);
            }
            DockTab::FileTree => {
                self.file_tree_panel.render(ui, &self.state);
            }
            DockTab::ExplorationList => {
                ExplorationListPanel.render(ui, &self.workspace_root);
            }
            DockTab::DiagnosticPreview => {
                self.diag_panel.render(
                    ui,
                    &self.state,
                    &self.tokio_handle,
                    &self.workspace_root,
                    &self.egui_ctx,
                );
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
            DockTab::Settings => {
                self.settings_panel
                    .render(ui, &self.settings, self.active_theme);
            }
            DockTab::File { path, source } => {
                FileTabPanel.render(ui, path, *source, &self.workspace_root, &self.state);
            }
            DockTab::DiffHistory { path, .. } => {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    self.workspace_root.join(path)
                };
                let panel = self
                    .diff_history_panels
                    .entry(absolute.clone())
                    .or_insert_with(|| {
                        DiffHistoryPanel::new(absolute.clone(), self.workspace_root.clone())
                    });
                panel.render(ui, &self.tokio_handle);
            }
        }
    }
}
