// egui+eframe desktop GUI: one OS window per exploration, dockable panels.
//
// Specs:
//   spec/components/gui/viewport.md
//   spec/components/gui/dock-layout.md
//   spec/components/gui/transcript-tab.md
//   spec/components/gui/chat-input-panel.md
//   spec/components/gui/spec-tree-panel.md
//   spec/components/gui/file-tree-panel.md
//   spec/components/gui/diagnostic-panel.md
//   spec/components/gui/file-tabs.md
//   spec/components/gui/exploration-list.md
//   spec/components/gui/theme.md
//   spec/components/gui/typography.md
//   spec/decisions/0003-egui-gui-over-tui.md

pub mod app;
pub mod dock;
pub mod highlighter;
pub mod panels;
pub mod theme;
pub mod viewport;

pub use app::App;
pub use dock::{DockTab, FileSource, default_layout};
pub use viewport::{ViewportConfig, run_viewport};

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

use oxidant_providers::{OllamaConfig, OllamaProvider, Provider};
use tokio::runtime::Handle;

/// Entry point used by the `oxidant` binary. Spawns a single viewport
/// for `workspace_root` with `provider` driving the agent loop. The
/// initial theme is read from `[gui] theme = "..."` in
/// `<worktree>/.oxidant/oxidant.toml` (or the per-user file); unknown
/// slugs fall back to the default.
pub fn launch_gui(
    workspace_root: &Path,
    provider: Arc<dyn Provider>,
    model: String,
    system_prompt: Option<String>,
    tokio_handle: Handle,
) -> Result<(), eframe::Error> {
    let canonical =
        dunce::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let settings = oxidant_config::load(&canonical).unwrap_or_default();
    let theme = theme::Theme::from_slug(&settings.gui.theme).unwrap_or_default();
    let config = ViewportConfig {
        workspace_root: canonical,
        provider,
        model,
        system_prompt,
        tokio_handle,
        theme,
        settings: Arc::new(StdMutex::new(settings)),
    };
    run_viewport(config)
}

/// Convenience: build a textgen-webui-backed provider for the default
/// `oxidant` GUI launch (per the existing local_chat example).
pub fn default_local_provider() -> (Arc<dyn Provider>, String) {
    let config = OllamaConfig::textgen();
    let model = "loaded-model".to_string(); // overridden by /v1/models lookup later
    let provider = Arc::new(OllamaProvider::new(config)) as Arc<dyn Provider>;
    (provider, model)
}
