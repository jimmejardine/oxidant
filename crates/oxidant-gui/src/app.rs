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

use oxidant_core::{AgentMode, Exploration, ExplorationId, ToolRegistry};
use oxidant_providers::{ChatEvent, Provider, StopReason, Usage};

use crate::dock::{
    DockTab, default_layout, is_tab_open, open_in_centre, open_tab, reset_layout_preserving_files,
    singleton_tabs,
};
use crate::panels::{
    chat_input::ChatInputPanel, diff_history::DiffHistoryPanel,
    exploration_list::ExplorationListPanel, file_tab::FileTabPanel, file_tree::FileTreePanel,
    health_check::HealthCheckPanel, merge_conflicts::MergeConflictsPanel, settings::SettingsPanel,
    spec_graph::SpecGraphPanel, spec_tree::SpecTreePanel, transcript::TranscriptPanel,
    user_question::UserQuestionPanel,
};
use crate::theme::Theme;
use crate::viewport::ViewportConfig;

pub struct App {
    config: ViewportConfig,
    dock: DockState<DockTab>,
    state: Arc<StdMutex<SharedState>>,
    event_rx: UnboundedReceiver<AgentEvent>,
    event_tx: UnboundedSender<AgentEvent>,
    /// Sub-viewports the user has opened on a non-main exploration.
    /// One per exploration_id; double-click on a row pushes onto
    /// `SharedState.pending_open_windows`, App drains and inserts a
    /// `SubWindow` here. Closing the sub-viewport tells us to remove
    /// the entry. See spec/components/gui/viewport.md.
    sub_windows: HashMap<ExplorationId, Arc<StdMutex<SubWindow>>>,
    chat_panel: ChatInputPanel,
    spec_panel: SpecTreePanel,
    file_tree_panel: FileTreePanel,
    /// One SpecGraphPanel per seed value the user has opened. Lazily
    /// inserted when the tab first paints; kept alive across tab
    /// close so re-opening the same seed preserves expansion state —
    /// same shape as `diff_history_panels`.
    spec_graph_panels: HashMap<String, SpecGraphPanel>,
    health_panel: HealthCheckPanel,
    settings_panel: SettingsPanel,
    exploration_list_panel: ExplorationListPanel,
    merge_conflicts_panel: MergeConflictsPanel,
    /// Modal that fulfils a pending `ask_user` tool call in the main
    /// window. See spec/tools/ask-user.md.
    user_question_panel: UserQuestionPanel,
    /// One DiffHistory panel per open path. Lazily inserted when the tab
    /// first paints; entries leak across tab close in MVP (cheap state,
    /// at most a handful per session). See
    /// spec/components/gui/diff-history-panel.md.
    diff_history_panels: HashMap<PathBuf, DiffHistoryPanel>,
    /// Currently-active theme. Mirrors what `theme::apply` recorded;
    /// kept here so the View → Theme submenu can render the radio
    /// state without a global lock.
    active_theme: Theme,
    /// Persistent markdown render cache for file-tab Preview mode. Held
    /// once here and threaded into each File tab so parse/image state
    /// survives across frames. See spec/components/gui/file-tabs.md.
    markdown_cache: egui_commonmark::CommonMarkCache,
    /// GPU load readout for the top bar. Inert when no GPU backend is
    /// available. See spec/components/gui/viewport.md "GPU load readout".
    gpu: crate::gpu::GpuMonitor,
}

/// The mutable bits shared between the GUI thread and the agent task.
/// Locks are held briefly; long async work happens on cloned data and
/// streams results back via the AgentEvent channel.
pub struct SharedState {
    /// All explorations open across every window in this process,
    /// keyed by id. Insertion-ordered (IndexMap) so the exploration-
    /// list panel can render them in spawn order — Main first, then
    /// subs by creation time.
    pub explorations: indexmap::IndexMap<ExplorationId, Exploration>,
    /// Pointer into `explorations` for the exploration whose
    /// conversation, worktree, and LSP handle the **main window** is
    /// driving. Sub-window view ids live on the App's sub_windows
    /// table — `state.active_id` is specifically the main window's.
    /// Updated by the exploration-list panel on single-click.
    pub active_id: ExplorationId,
    pub registry: Arc<ToolRegistry>,
    /// Per-window runtime state — live turn, last outcome, cancellation
    /// — keyed by the window's view id. Streamed events from a turn
    /// route into the right entry via `AgentEvent.viewport_id`. The
    /// main window's entry lives at `active_id`; sub windows at their
    /// locked id. Auto-inserted on demand via `window_mut`.
    pub windows: HashMap<ExplorationId, PerWindowState>,
    /// CI-style health report. One entry per CheckKind. See
    /// spec/components/gui/health-check-panel.md.
    pub health: HealthReport,
    /// Centre-tab opens requested by a panel that doesn't own the dock
    /// (e.g. spec-tree double-click). Drained once per frame after
    /// `DockArea::show`; see spec/components/gui/spec-tree-panel.md.
    pub pending_centre_tabs: Vec<DockTab>,
    /// Auto-fill request from any panel into the chat input. Drained
    /// once per frame by `ChatInputPanel::render`: replace draft, set
    /// mode, request focus, clear the field. No auto-send.
    /// See spec/components/gui/chat-input-panel.md "External prompt fill".
    pub pending_chat_prompt: Option<PendingChatPrompt>,
    /// "Continue iterating" request from the transcript: the new
    /// `max_iterations` to use when re-entering the agent loop on the
    /// existing conversation. Drained once per frame by
    /// `ChatInputPanel::render` — mirror of `pending_chat_prompt`. See
    /// spec/components/gui/chat-input-panel.md "Continue iterating".
    pub pending_continue: Option<usize>,
    /// Per-path edit buffers for File tabs. Keyed by absolute path so
    /// the same file in two tabs (which can't actually happen — dock
    /// tabs are unique) would share state. See
    /// spec/components/gui/file-tabs.md "Edit lifecycle for specs".
    pub editor_buffers: HashMap<PathBuf, EditorBuffer>,
    /// Read-only contents for the `Selected` preview tab. Set by a
    /// single-click in the spec/file tree; rendered by the Selected tab.
    /// See spec/components/gui/dock-layout.md "Selected preview tab".
    pub selected_preview: Option<SelectedPreview>,
    /// Conflict-resolution state populated when a merge-back returns
    /// conflicts; consumed by the MergeConflicts panel. None means
    /// no merge is in progress. See spec/components/gui/merge-conflicts.md.
    pub merge_conflicts: Option<MergeConflictsState>,
    /// Double-click on an exploration-list row enqueues the id here;
    /// App::update drains and opens a sub-viewport for each. See
    /// spec/components/gui/viewport.md.
    pub pending_open_windows: Vec<ExplorationId>,
    /// Sub-window close requests — populated from inside the
    /// sub-window's viewport closure when egui reports
    /// `close_requested()`; drained by App::update which removes the
    /// matching `sub_windows` entry. Channel exists because the
    /// closure runs under a different `egui::Context` and can't
    /// directly mutate App's state.
    pub pending_close_windows: Vec<ExplorationId>,
}

/// Per-sub-window UI state. Lives in `App.sub_windows` behind an
/// `Arc<Mutex<>>` so the egui sub-viewport closure (which captures it
/// by clone) can lock-and-mutate across frames.
pub struct SubWindow {
    pub view_id: ExplorationId,
    pub chat_panel: ChatInputPanel,
    pub user_question_panel: UserQuestionPanel,
}

impl SubWindow {
    pub fn new(view_id: ExplorationId) -> Self {
        Self {
            view_id,
            chat_panel: ChatInputPanel::new(),
            user_question_panel: UserQuestionPanel::new(),
        }
    }
}

/// Per-window runtime state. The shape that used to live directly on
/// `SharedState` for the three fields the agent loop drives — moving
/// them per-window means two windows can stream independent turns
/// against different explorations without their state colliding.
/// Routed by `AgentEvent.viewport_id`.
#[derive(Default)]
pub struct PerWindowState {
    pub live_turn: Option<LiveTurn>,
    pub last_outcome: Option<TurnOutcome>,
    /// Per-turn cancellation token (Esc / Cancel button). Distinct
    /// from the exploration's own `cancellation`, which tears down
    /// the whole exploration.
    pub cancellation: Option<CancellationToken>,
    /// An in-flight `ask_user` tool call awaiting an answer from
    /// the user in this window. Some when a question is pending;
    /// `UserQuestionPanel::render` consumes it on Submit by `take()`
    /// + sending on the oneshot. See spec/tools/ask-user.md.
    pub pending_question: Option<PendingUserQuestion>,
}

/// An in-flight `ask_user` request: the question, the precanned
/// options, whether to render the free-form fallback, and the
/// oneshot sender that fulfills the tool's awaiting future.
///
/// Not `Clone` (the Sender isn't); lives by `Option::take()` only.
pub struct PendingUserQuestion {
    pub question: String,
    pub options: Vec<String>,
    pub allow_freeform: bool,
    /// Consumed when the user clicks Submit. If the window or turn
    /// is dropped before the user answers, the Sender drops and the
    /// tool's Receiver errors — surfaced as "user cancelled" in the
    /// conversation.
    pub answer_tx: tokio::sync::oneshot::Sender<String>,
}

#[derive(Debug, Clone)]
pub struct MergeConflictsState {
    pub sub_id: ExplorationId,
    pub parent_id: ExplorationId,
    pub target_branch: String,
    /// The parent's worktree path — where the in-progress merge lives.
    pub parent_worktree: PathBuf,
    pub sub_branch: String,
    pub sub_worktree: PathBuf,
    /// True if the merge was started with `--squash`. Drives the
    /// finalise call: squash needs `git commit -m <message>`; the
    /// non-squash path (`--no-ff` with conflicts) just runs
    /// `git commit` with git's prepared merge message.
    pub squash: bool,
    pub message: String,
    pub files: Vec<String>,
    pub resolved: std::collections::HashSet<String>,
}

impl SharedState {
    /// Borrow the main window's currently-viewed exploration.
    pub fn active(&self) -> &Exploration {
        self.exploration(self.active_id)
    }

    /// Mutably borrow the main window's currently-viewed exploration.
    pub fn active_mut(&mut self) -> &mut Exploration {
        self.exploration_mut(self.active_id)
    }

    /// Borrow the exploration identified by `id`. Panics if missing —
    /// every Window's `view_id` is required by invariant to point at
    /// a live exploration; closing a sub-window or discarding the
    /// exploration removes the corresponding entry first.
    pub fn exploration(&self, id: ExplorationId) -> &Exploration {
        self.explorations
            .get(&id)
            .expect("view_id must always point at an entry in `explorations`")
    }

    /// Mutably borrow the exploration identified by `id`.
    pub fn exploration_mut(&mut self, id: ExplorationId) -> &mut Exploration {
        self.explorations
            .get_mut(&id)
            .expect("view_id must always point at an entry in `explorations`")
    }

    /// Get-or-default the per-window runtime state for `view_id`.
    pub fn window_mut(&mut self, view_id: ExplorationId) -> &mut PerWindowState {
        self.windows.entry(view_id).or_default()
    }

    /// Read-only access; returns `None` if no turn has been driven
    /// for this view yet (the entry is lazily inserted).
    pub fn window(&self, view_id: ExplorationId) -> Option<&PerWindowState> {
        self.windows.get(&view_id)
    }
}

/// Snapshot of a file shown read-only in the `Selected` preview tab.
/// Loaded once at single-click time (no per-frame disk reads).
#[derive(Debug, Clone)]
pub struct SelectedPreview {
    /// Absolute path of the previewed file.
    pub path: PathBuf,
    /// File contents at click time.
    pub text: String,
    /// Read error, if the file couldn't be loaded.
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PendingChatPrompt {
    pub prompt: String,
    pub mode: AgentMode,
}

/// Which view a (markdown) file tab is showing. Non-markdown files are
/// always `Source`; markdown files default to `Preview`. See
/// spec/components/gui/file-tabs.md "Markdown preview toggle".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Preview,
    Source,
}

#[derive(Debug, Clone)]
pub struct EditorBuffer {
    pub text: String,
    pub dirty: bool,
    /// Preview (rendered markdown) vs Source (highlighted editor). Only
    /// togglable for markdown files; ignored for everything else.
    pub view_mode: ViewMode,
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

#[derive(Debug, Clone, Default)]
pub struct TurnOutcome {
    pub stop_reason: Option<StopReason>,
    pub usage: Usage,
    pub iterations: usize,
    pub tool_calls: usize,
    pub error: Option<String>,
    /// `true` only when the failure mode was the agent loop hitting its
    /// `max_iterations` cap. Drives the "Continue iterating" affordance
    /// in the transcript — see spec/components/gui/chat-input-panel.md
    /// "Continue iterating".
    pub hit_max_iterations: bool,
    /// `true` when the turn was cancelled by the user (ESC / Cancel). The
    /// transcript shows a muted "⊘ turn cancelled" line. See
    /// spec/components/gui/chat-input-panel.md "Cancel semantics".
    pub cancelled: bool,
}

// ---------------------------------------------------------------- Health report
//
// Per spec/components/gui/health-check-panel.md.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct HealthReport {
    pub checks: BTreeMap<CheckKind, CheckState>,
    pub last_run_at: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CheckKind {
    CargoCheck,
    Clippy,
    Tests,
    SpecValidate,
    SpecDiff,
    SpecCoverage,
}

impl CheckKind {
    pub fn display_name(self) -> &'static str {
        match self {
            CheckKind::CargoCheck => "cargo check",
            CheckKind::Clippy => "clippy",
            CheckKind::Tests => "tests",
            CheckKind::SpecValidate => "spec validate",
            CheckKind::SpecDiff => "spec diff",
            CheckKind::SpecCoverage => "spec coverage",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CheckKind::CargoCheck => "cargo_check",
            CheckKind::Clippy => "clippy",
            CheckKind::Tests => "tests",
            CheckKind::SpecValidate => "spec_validate",
            CheckKind::SpecDiff => "spec_diff",
            CheckKind::SpecCoverage => "spec_coverage",
        }
    }

    pub fn tool_name(self) -> &'static str {
        match self {
            CheckKind::CargoCheck => "cargo_check",
            CheckKind::Clippy => "cargo_clippy",
            CheckKind::Tests => "cargo_test",
            CheckKind::SpecValidate => "spec_validate",
            CheckKind::SpecDiff => "spec_diff",
            CheckKind::SpecCoverage => "spec_coverage",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CheckState {
    pub status: CheckStatus,
    pub issues: Vec<HealthIssue>,
    pub finished_in_ms: u64,
    /// True once the user has clicked the root header. Suppresses
    /// auto-expand on the next red transition so we don't fight them.
    pub user_toggled: bool,
}

#[derive(Debug, Clone, Default)]
pub enum CheckStatus {
    #[default]
    Idle,
    Running,
    Done,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct HealthIssue {
    pub check: CheckKind,
    pub severity: IssueSeverity,
    /// Subtree group: file path / WarningKind / drift kind / test target.
    pub group_key: String,
    pub message: String,
    pub file: Option<String>,
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    Error,
    Warning,
    Note,
}

/// Messages flowing from the agent task back to the GUI thread. Each
/// event carries the `viewport_id` (= the window's `view_id`) so the
/// dispatcher routes streaming output to the correct window's
/// `PerWindowState.live_turn` rather than blending two windows.
pub enum AgentEvent {
    /// A raw streaming event from the provider; the GUI accumulates
    /// these into the matching window's LiveTurn.
    Chat {
        viewport_id: ExplorationId,
        event: ChatEvent,
    },
    /// The agent loop finished for this window. Whether successful or
    /// not, the live turn is committed into the conversation by the
    /// time this fires (the agent task did that before sending).
    Completed {
        viewport_id: ExplorationId,
        outcome: TurnOutcome,
    },
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
        let active_id = exploration.id;
        let mut explorations = indexmap::IndexMap::new();
        explorations.insert(active_id, exploration);
        let state = Arc::new(StdMutex::new(SharedState {
            explorations,
            active_id,
            registry: Arc::new(registry),
            windows: HashMap::from([(active_id, PerWindowState::default())]),
            health: HealthReport::default(),
            pending_centre_tabs: Vec::new(),
            pending_chat_prompt: None,
            pending_continue: None,
            editor_buffers: HashMap::new(),
            selected_preview: None,
            merge_conflicts: None,
            pending_open_windows: Vec::new(),
            pending_close_windows: Vec::new(),
        }));
        let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();

        let spec_panel = SpecTreePanel::new(config.workspace_root.clone());
        let file_tree_panel = FileTreePanel::new(config.workspace_root.clone());
        let active_theme = config.theme;
        let settings_panel = SettingsPanel::new(&config.settings);
        Self {
            sub_windows: HashMap::new(),
            chat_panel: ChatInputPanel::new(),
            spec_panel,
            file_tree_panel,
            spec_graph_panels: HashMap::new(),
            health_panel: HealthCheckPanel::new(),
            settings_panel,
            exploration_list_panel: ExplorationListPanel::new(),
            merge_conflicts_panel: MergeConflictsPanel::new(),
            user_question_panel: UserQuestionPanel::new(),
            diff_history_panels: HashMap::new(),
            config,
            dock: default_layout(),
            state,
            event_rx,
            event_tx,
            active_theme,
            markdown_cache: egui_commonmark::CommonMarkCache::default(),
            gpu: crate::gpu::GpuMonitor::new(),
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
        } else if self
            .state
            .lock()
            .unwrap()
            .windows
            .values()
            .any(|w| w.live_turn.is_some())
        {
            // Keep redrawing while ANY window has a turn in flight so
            // the spinner stays animated. Multi-window note: any
            // window's stream warrants a paint tick — they all share
            // the main update loop.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }

        // Global UI zoom: Ctrl+scroll-wheel steps the zoom factor;
        // Ctrl+0 resets to 1.0. Consumed at this layer so ScrollAreas
        // don't ALSO scroll. Persisted to GuiSettings on every change.
        // See spec/components/gui/typography.md.
        let (zoom_delta, reset) = ctx.input_mut(|i| {
            let cmd = i.modifiers.command;
            let delta = if cmd && i.raw_scroll_delta.y.abs() > 0.5 {
                let dir = i.raw_scroll_delta.y.signum();
                i.raw_scroll_delta.y = 0.0;
                i.smooth_scroll_delta.y = 0.0;
                Some(dir * 0.1)
            } else {
                None
            };
            let reset = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0);
            (delta, reset)
        });
        let new_factor = if reset {
            Some(1.0)
        } else {
            zoom_delta.map(|d| (ctx.zoom_factor() + d).clamp(0.5, 3.0))
        };
        if let Some(z) = new_factor {
            ctx.set_zoom_factor(z);
            if let Ok(mut s) = self.config.settings.lock() {
                s.gui.zoom_factor = z;
                if let Err(e) = oxidant_config::save_user(&s) {
                    tracing::warn!(?e, "saving zoom_factor to user settings");
                }
            }
            ctx.request_repaint();
        }

        // Refresh the GPU readout (throttled internally to ~1 Hz) and
        // keep the frame ticking so the number updates while idle —
        // only when a GPU backend is actually present.
        self.gpu.sample();
        if self.gpu.is_active() {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }

        // Top menu bar: title + Window menu + status + model.
        egui::TopBottomPanel::top("oxidant-menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.label(egui::RichText::new("oxidant").strong());
                ui.separator();
                self.render_window_menu(ui);
                ui.separator();
                let state = self.state.lock().unwrap();
                let n = state.active().conversation.len();
                let live = if state
                    .window(state.active_id)
                    .is_some_and(|w| w.live_turn.is_some())
                {
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
                    if let Some(gpu) = self.gpu.label() {
                        ui.separator();
                        ui.label(egui::RichText::new(gpu).color(crate::theme::muted_text()));
                    }
                });
            });
        });

        // Dock area. The main viewport's view_id mirrors active_id.
        let main_view_id = self.state.lock().unwrap().active_id;
        let mut viewer = TabViewer {
            state: self.state.clone(),
            view_id: main_view_id,
            chat_panel: &mut self.chat_panel,
            spec_panel: &mut self.spec_panel,
            file_tree_panel: &mut self.file_tree_panel,
            spec_graph_panels: &mut self.spec_graph_panels,
            health_panel: &mut self.health_panel,
            settings_panel: &mut self.settings_panel,
            exploration_list_panel: &mut self.exploration_list_panel,
            merge_conflicts_panel: &mut self.merge_conflicts_panel,
            diff_history_panels: &mut self.diff_history_panels,
            settings: self.config.settings.clone(),
            active_theme: &mut self.active_theme,
            markdown_cache: &mut self.markdown_cache,
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

        // ask_user modal — renders on top of the dock when a tool has
        // posted a pending question for the main window's view. See
        // spec/tools/ask-user.md.
        if self
            .user_question_panel
            .render(ctx, &self.state, main_view_id)
        {
            ctx.request_repaint();
        }

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

        // Drain pending_open_windows + pending_close_windows into our
        // sub_windows table. Then re-register every live sub-window
        // with egui as a deferred viewport. See
        // spec/components/gui/viewport.md.
        let (to_open, to_close) = {
            let mut s = self.state.lock().unwrap();
            (
                std::mem::take(&mut s.pending_open_windows),
                std::mem::take(&mut s.pending_close_windows),
            )
        };
        for id in to_close {
            self.sub_windows.remove(&id);
        }
        for id in to_open {
            // Refuse to open a duplicate viewport on an exploration
            // that already has one — egui would dedupe by ViewportId
            // anyway, but tracking it here makes the intent explicit
            // and lets us focus the existing window in a follow-up.
            self.sub_windows
                .entry(id)
                .or_insert_with(|| Arc::new(StdMutex::new(SubWindow::new(id))));
        }

        for (id, sub) in &self.sub_windows {
            let id = *id;
            let sub = sub.clone();
            let state = self.state.clone();
            let event_tx = self.event_tx.clone();
            let tokio_handle = self.config.tokio_handle.clone();
            let workspace_root = self.config.workspace_root.clone();
            let provider = self.config.provider.clone();
            let model = self.config.model.clone();
            let system_prompt = self.config.system_prompt.clone();
            let title = format_sub_title(&self.state, id);
            ctx.show_viewport_deferred(
                egui::ViewportId::from_hash_of(id),
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([900.0, 700.0]),
                move |viewport_ctx, _class| {
                    render_sub_window(
                        viewport_ctx,
                        &sub,
                        &state,
                        &event_tx,
                        &tokio_handle,
                        &workspace_root,
                        &provider,
                        &model,
                        system_prompt.as_deref(),
                    );
                    if viewport_ctx.input(|i| i.viewport().close_requested())
                        && let Ok(mut s) = state.lock()
                        && !s.pending_close_windows.contains(&id)
                    {
                        s.pending_close_windows.push(id);
                    }
                },
            );
        }
    }
}

fn format_sub_title(state: &Arc<StdMutex<SharedState>>, id: ExplorationId) -> String {
    let branch = state
        .lock()
        .ok()
        .and_then(|s| s.explorations.get(&id).map(|e| e.branch.clone()))
        .unwrap_or_else(|| id.to_string());
    format!("oxidant — [sub: {branch}]")
}

#[allow(clippy::too_many_arguments)]
fn render_sub_window(
    ctx: &egui::Context,
    sub: &Arc<StdMutex<SubWindow>>,
    state: &Arc<StdMutex<SharedState>>,
    event_tx: &UnboundedSender<AgentEvent>,
    tokio_handle: &Handle,
    workspace_root: &std::path::Path,
    provider: &Arc<dyn Provider>,
    model: &str,
    system_prompt: Option<&str>,
) {
    let view_id = sub.lock().unwrap().view_id;
    // Top label: which exploration this window is locked to.
    egui::TopBottomPanel::top("oxidant-sub-top").show(ctx, |ui| {
        ui.horizontal(|ui| {
            let branch = state
                .lock()
                .ok()
                .and_then(|s| s.explorations.get(&view_id).map(|e| e.branch.clone()))
                .unwrap_or_else(|| "—".into());
            ui.label(egui::RichText::new(format!("[sub] {branch}")).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(model).color(crate::theme::muted_text()));
            });
        });
    });
    // Bottom: chat input.
    egui::TopBottomPanel::bottom("oxidant-sub-chat").show(ctx, |ui| {
        let mut sub_locked = sub.lock().unwrap();
        sub_locked.chat_panel.render(
            ui,
            state,
            view_id,
            event_tx,
            tokio_handle,
            workspace_root,
            provider,
            model,
            system_prompt,
            ctx,
        );
    });
    // Centre: transcript.
    egui::CentralPanel::default().show(ctx, |ui| {
        let mut s = state.lock().unwrap();
        let action = TranscriptPanel.render(ui, &s, view_id);
        if let crate::panels::transcript::TranscriptAction::ContinueIterating { new_max } = action {
            // For now sub-windows piggy-back on the shared
            // pending_continue side-channel — the main-window chat
            // panel will route correctly only when it's also the
            // active view. A future cleanup is to route per-window.
            s.pending_continue = Some(new_max);
        }
    });
    // ask_user modal for this sub-window. Rendered after the
    // central panel so it overlays the transcript when a question
    // is posted on this view.
    let mut sub_locked = sub.lock().unwrap();
    if sub_locked.user_question_panel.render(ctx, state, view_id) {
        ctx.request_repaint();
    }
}

fn apply_event(state: &mut SharedState, ev: AgentEvent) {
    match ev {
        AgentEvent::Chat { viewport_id, event } => {
            let window = state.window_mut(viewport_id);
            let turn = window.live_turn.get_or_insert_with(LiveTurn::default);
            match event {
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
        AgentEvent::Completed {
            viewport_id,
            outcome,
        } => {
            let window = state.window_mut(viewport_id);
            window.live_turn = None;
            window.last_outcome = Some(outcome);
            window.cancellation = None;
        }
    }
}

pub(crate) struct TabViewer<'a> {
    pub state: Arc<StdMutex<SharedState>>,
    /// The exploration this window is bound to. For the main viewport
    /// this mirrors `state.active_id`; for sub-viewports it's the
    /// locked id. Panels that drive turns (chat input, transcript)
    /// route through this rather than `state.active_id`.
    pub view_id: ExplorationId,
    pub chat_panel: &'a mut ChatInputPanel,
    pub spec_panel: &'a mut SpecTreePanel,
    pub file_tree_panel: &'a mut FileTreePanel,
    pub spec_graph_panels: &'a mut HashMap<String, SpecGraphPanel>,
    pub health_panel: &'a mut HealthCheckPanel,
    pub settings_panel: &'a mut SettingsPanel,
    pub exploration_list_panel: &'a mut ExplorationListPanel,
    pub merge_conflicts_panel: &'a mut MergeConflictsPanel,
    pub diff_history_panels: &'a mut HashMap<PathBuf, DiffHistoryPanel>,
    pub settings: Arc<StdMutex<oxidant_config::Settings>>,
    pub active_theme: &'a mut Theme,
    pub markdown_cache: &'a mut egui_commonmark::CommonMarkCache,
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
                let mut state = self.state.lock().unwrap();
                let action = TranscriptPanel.render(ui, &state, self.view_id);
                match action {
                    crate::panels::transcript::TranscriptAction::None => {}
                    crate::panels::transcript::TranscriptAction::ContinueIterating { new_max } => {
                        state.pending_continue = Some(new_max);
                    }
                }
            }
            DockTab::Selected => {
                let preview = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|s| s.selected_preview.clone());
                crate::panels::file_tab::render_selected(
                    ui,
                    preview.as_ref(),
                    &self.workspace_root,
                    self.markdown_cache,
                );
            }
            DockTab::SpecTree => {
                self.spec_panel.render(ui, &self.state);
            }
            DockTab::FileTree => {
                self.file_tree_panel.render(ui, &self.state);
            }
            DockTab::SpecGraph { seed } => {
                let workspace = self.workspace_root.clone();
                let seed_owned = seed.clone();
                let panel = self
                    .spec_graph_panels
                    .entry(seed_owned.clone())
                    .or_insert_with(|| SpecGraphPanel::new(workspace, seed_owned));
                panel.render(ui, &self.state);
            }
            DockTab::ExplorationList => {
                self.exploration_list_panel.render(
                    ui,
                    &self.state,
                    &self.workspace_root,
                    &self.tokio_handle,
                    &self.egui_ctx,
                );
            }
            DockTab::MergeConflicts => {
                self.merge_conflicts_panel.render(
                    ui,
                    &self.state,
                    &self.tokio_handle,
                    &self.egui_ctx,
                );
            }
            DockTab::HealthCheck => {
                self.health_panel.render(
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
                    self.view_id,
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
                FileTabPanel.render(
                    ui,
                    path,
                    *source,
                    &self.workspace_root,
                    &self.state,
                    self.markdown_cache,
                );
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
