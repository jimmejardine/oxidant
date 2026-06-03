// Realises spec/components/gui/viewport.md.
//
// MVP: one window for one exploration (the current workspace). Multi-
// viewport (one OS window per sub-exploration via
// ctx.show_viewport_deferred) is deferred to the next phase.
//
// This module owns the eframe::run_native entry point and the window
// title formatting. The persistent App state lives in app.rs.

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use eframe::NativeOptions;
use oxidant_config::Settings;
use tokio::runtime::Handle;

use crate::app::App;
use crate::theme;

#[derive(Clone)]
pub struct ViewportConfig {
    pub workspace_root: PathBuf,
    pub provider: Arc<dyn oxidant_providers::Provider>,
    pub model: String,
    pub system_prompt: Option<String>,
    pub tokio_handle: Handle,
    /// Initial theme. Loaded from `[gui] theme = "..."` in settings;
    /// flipped at runtime via the View → Theme menu.
    pub theme: theme::Theme,
    /// Live, shared settings. The Settings panel mutates this and writes
    /// to disk; other panels (theme, model, chat-input) read from it.
    pub settings: Arc<StdMutex<Settings>>,
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
        Box::new(move |cc| {
            // install_fonts swaps egui's default proportional and mono
            // families for Noto so symbols like ✗ ↩ ⊕ ⌖ ⚠ ⏎ render
            // cleanly. Called once at startup; apply() handles the rest
            // (visuals + uniform text_styles).
            theme::install_fonts(&cc.egui_ctx);
            theme::apply(&cc.egui_ctx, config.theme);
            // Apply the persisted UI zoom factor before the first paint.
            // Clamped on load — guards against a hand-edited TOML out of
            // sane range. See spec/components/gui/typography.md.
            let z = config
                .settings
                .lock()
                .map(|s| s.gui.zoom_factor)
                .unwrap_or(1.0)
                .clamp(0.5, 3.0);
            cc.egui_ctx.set_zoom_factor(z);
            Ok(Box::new(App::new(config)))
        }),
    )
}
