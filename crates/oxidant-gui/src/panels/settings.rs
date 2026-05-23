// Settings / Preferences panel.
//
// Shows everything in `oxidant_config::Settings` grouped into three
// sections (Providers, GUI, Permissions). Edits mutate a local draft;
// Save writes the user-level config TOML (`save_user`) and updates the
// shared Settings lock so other panels see the change. Revert restores
// the last-saved baseline.

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use egui::{Color32, RichText, TextEdit};

use oxidant_config::{Settings, save_user, user_config_path};

use crate::theme::{self, Theme};

pub struct SettingsPanel {
    /// Last-saved snapshot. Used to compute dirty state and to power Revert.
    baseline: Settings,
    /// Live edits. Rendered into widgets; copied into the shared lock on Save.
    draft: Settings,
    /// Multi-line text buffers backing the allowlist / denylist editors.
    /// Kept in sync with `draft.permissions.*` on every render.
    allowlist_text: String,
    denylist_text: String,
    /// API-key reveal toggles (eyeball icon). Default: masked.
    show_anthropic_key: bool,
    show_openai_key: bool,
    /// Most recent save attempt result, displayed inline. None = idle.
    last_save: Option<Result<PathBuf, String>>,
}

impl SettingsPanel {
    pub fn new(settings: &Arc<StdMutex<Settings>>) -> Self {
        let snapshot = settings.lock().map(|s| s.clone()).unwrap_or_default();
        let allowlist_text = snapshot.permissions.allowlist.join("\n");
        let denylist_text = snapshot.permissions.denylist.join("\n");
        Self {
            baseline: snapshot.clone(),
            draft: snapshot,
            allowlist_text,
            denylist_text,
            show_anthropic_key: false,
            show_openai_key: false,
            last_save: None,
        }
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        shared: &Arc<StdMutex<Settings>>,
        active_theme: &mut Theme,
    ) {
        // Sync list editors → draft before rendering, so anything below
        // sees consistent state.
        self.draft.permissions.allowlist = lines_to_list(&self.allowlist_text);
        self.draft.permissions.denylist = lines_to_list(&self.denylist_text);

        let dirty = self.draft != self.baseline;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Settings").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save_resp = ui.add_enabled(dirty, egui::Button::new("Save to user config"));
                if save_resp.clicked() {
                    match save_user(&self.draft) {
                        Ok(path) => {
                            self.baseline = self.draft.clone();
                            if let Ok(mut s) = shared.lock() {
                                *s = self.draft.clone();
                            }
                            self.last_save = Some(Ok(path));
                        }
                        Err(e) => self.last_save = Some(Err(e.to_string())),
                    }
                }
                if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
                    self.draft = self.baseline.clone();
                    self.allowlist_text = self.draft.permissions.allowlist.join("\n");
                    self.denylist_text = self.draft.permissions.denylist.join("\n");
                    self.last_save = None;
                }
            });
        });

        if dirty {
            ui.label(RichText::new("unsaved changes").color(theme::muted_text()));
        }
        if let Some(r) = &self.last_save {
            match r {
                Ok(path) => {
                    ui.label(
                        RichText::new(format!("saved → {}", path.display()))
                            .color(theme::muted_text()),
                    );
                }
                Err(msg) => {
                    ui.label(
                        RichText::new(format!("save failed: {msg}")).color(Color32::LIGHT_RED),
                    );
                }
            }
        }
        if let Some(path) = user_config_path() {
            ui.label(
                RichText::new(format!("config file: {}", path.display()))
                    .color(theme::faint_text())
                    .small(),
            );
        }
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.section_providers(ui);
                ui.add_space(8.0);
                self.section_gui(ui, active_theme);
                ui.add_space(8.0);
                self.section_permissions(ui);
            });
    }

    fn section_providers(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(RichText::new("Providers").strong())
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Active provider");
                    egui::ComboBox::from_id_salt("settings-active-provider")
                        .selected_text(&self.draft.provider.default)
                        .show_ui(ui, |ui| {
                            for p in &[
                                "anthropic",
                                "openai",
                                "ollama",
                                "textgen",
                                "lmstudio",
                                "llamacpp",
                            ] {
                                ui.selectable_value(
                                    &mut self.draft.provider.default,
                                    (*p).to_string(),
                                    *p,
                                );
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label("Default model");
                    let mut value = self
                        .draft
                        .provider
                        .default_model
                        .clone()
                        .unwrap_or_default();
                    let resp = ui.add(TextEdit::singleline(&mut value).desired_width(280.0));
                    if resp.changed() {
                        self.draft.provider.default_model = empty_to_none(value);
                    }
                });

                ui.add_space(6.0);
                ui.collapsing("Anthropic", |ui| {
                    api_key_row(
                        ui,
                        "API key",
                        &mut self.draft.provider.anthropic.api_key,
                        &mut self.show_anthropic_key,
                    );
                    optional_text_row(
                        ui,
                        "Read from env",
                        &mut self.draft.provider.anthropic.api_key_env,
                    );
                    ui.label(
                        RichText::new("Stored in plain text in the user config TOML.")
                            .color(theme::faint_text())
                            .small(),
                    );
                });

                ui.collapsing("OpenAI", |ui| {
                    api_key_row(
                        ui,
                        "API key",
                        &mut self.draft.provider.openai.api_key,
                        &mut self.show_openai_key,
                    );
                    optional_text_row(
                        ui,
                        "Read from env",
                        &mut self.draft.provider.openai.api_key_env,
                    );
                    optional_text_row(ui, "Base URL", &mut self.draft.provider.openai.base_url);
                    optional_text_row(
                        ui,
                        "Default model",
                        &mut self.draft.provider.openai.default_model,
                    );
                });

                ui.collapsing("Ollama (local)", |ui| {
                    local_provider_rows(ui, &mut self.draft.provider.ollama);
                });
                ui.collapsing("textgen-webui (local)", |ui| {
                    local_provider_rows(ui, &mut self.draft.provider.textgen);
                });
                ui.collapsing("LM Studio (local)", |ui| {
                    local_provider_rows(ui, &mut self.draft.provider.lmstudio);
                });
                ui.collapsing("llama.cpp server (local)", |ui| {
                    local_provider_rows(ui, &mut self.draft.provider.llamacpp);
                });
            });
    }

    fn section_gui(&mut self, ui: &mut egui::Ui, active_theme: &mut Theme) {
        egui::CollapsingHeader::new(RichText::new("GUI").strong())
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    let current = Theme::from_slug(&self.draft.gui.theme).unwrap_or_default();
                    let mut selected = current;
                    let prev_slug = self.draft.gui.theme.clone();
                    egui::ComboBox::from_id_salt("settings-theme")
                        .selected_text(selected.display_name())
                        .show_ui(ui, |ui| {
                            for t in Theme::ALL {
                                ui.selectable_value(&mut selected, *t, t.display_name());
                            }
                        });
                    if selected.slug() != prev_slug {
                        self.draft.gui.theme = selected.slug().to_string();
                        *active_theme = selected;
                        theme::apply(ui.ctx(), selected);
                    }
                });

                ui.checkbox(
                    &mut self.draft.gui.enter_sends,
                    "Enter sends (Shift+Enter inserts a newline)",
                );
            });
    }

    fn section_permissions(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(RichText::new("Permissions").strong())
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(
                    &mut self.draft.permissions.auto_approve_readonly,
                    "Auto-approve ReadOnly tools",
                );
                ui.label(
                    RichText::new(
                        "Mutating and Network tools always prompt unless they match an \
                         allowlist entry. Patterns: `fs_write` (exact name), \
                         `bash:cargo *` (bash glob), `bash:/^git /` (bash regex). One per line.",
                    )
                    .color(theme::faint_text())
                    .small(),
                );
                ui.add_space(4.0);
                ui.label(RichText::new("Allowlist").strong());
                ui.add(
                    TextEdit::multiline(&mut self.allowlist_text)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
                ui.add_space(4.0);
                ui.label(RichText::new("Denylist").strong());
                ui.add(
                    TextEdit::multiline(&mut self.denylist_text)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            });
    }
}

fn api_key_row(ui: &mut egui::Ui, label: &str, value: &mut Option<String>, show: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut buf = value.clone().unwrap_or_default();
        let edit = TextEdit::singleline(&mut buf)
            .password(!*show)
            .desired_width(280.0);
        let resp = ui.add(edit);
        if resp.changed() {
            *value = empty_to_none(buf);
        }
        let icon = if *show { "🔒" } else { "👁" };
        if ui.small_button(icon).clicked() {
            *show = !*show;
        }
    });
}

fn optional_text_row(ui: &mut egui::Ui, label: &str, value: &mut Option<String>) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut buf = value.clone().unwrap_or_default();
        let resp = ui.add(TextEdit::singleline(&mut buf).desired_width(280.0));
        if resp.changed() {
            *value = empty_to_none(buf);
        }
    });
}

fn local_provider_rows(ui: &mut egui::Ui, settings: &mut oxidant_config::LocalSettings) {
    optional_text_row(ui, "Base URL", &mut settings.base_url);
    optional_text_row(ui, "Default model", &mut settings.default_model);
    optional_text_row(ui, "API key (if required)", &mut settings.api_key);
}

fn empty_to_none(s: String) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn lines_to_list(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}
