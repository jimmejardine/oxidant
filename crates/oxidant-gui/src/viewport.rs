// Realises spec/components/gui/viewport.md.
//
// MVP: one window for one exploration (the current workspace). Multi-
// viewport (one OS window per sub-exploration via
// ctx.show_viewport_deferred) is deferred to the next phase.
//
// This module owns the eframe::run_native entry point and the window
// title formatting. The persistent App state lives in app.rs.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::NativeOptions;
use tokio::runtime::Handle;

use crate::app::App;

#[derive(Clone)]
pub struct ViewportConfig {
    pub workspace_root: PathBuf,
    pub provider: Arc<dyn oxidant_providers::Provider>,
    pub model: String,
    pub system_prompt: Option<String>,
    pub tokio_handle: Handle,
}

pub fn run_viewport(config: ViewportConfig) -> Result<(), eframe::Error> {
    let repo_name = config
        .workspace_root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "oxidant".to_string());
    let title = format!("oxidant — {repo_name} (main)");

    let native_options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        &title,
        native_options,
        Box::new(move |_cc| Ok(Box::new(App::new(config)))),
    )
}
