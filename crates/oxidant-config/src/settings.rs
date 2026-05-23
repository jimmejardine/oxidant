// Realises spec/components/config/settings.md.
//
// Load + merge oxidant's configuration from per-repo + per-user
// TOML files, with a handful of env-var overrides on top. Hot reload
// via notify and the tokio::sync::watch::Sender<Settings> propagation
// channel are deferred — for MVP, callers load once at startup.
//
// Merge order, lowest to highest precedence:
//   built-in defaults  <  per-repo TOML  <  per-user TOML  <  env vars
//
// (Per the spec, per-user wins over per-repo: API keys live in the
// user file and should override anything checked into a project.)

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub provider: ProviderSettings,
    pub gui: GuiSettings,
    pub permissions: PermissionsSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: ProviderSettings::default(),
            gui: GuiSettings::default(),
            permissions: PermissionsSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSettings {
    /// Which provider to use by default: "anthropic" | "openai" | "ollama" | "textgen" | "lmstudio" | "llamacpp".
    pub default: String,
    /// Model id sent in chat requests when no per-call override is given.
    pub default_model: Option<String>,
    pub anthropic: AnthropicSettings,
    pub openai: OpenAISettings,
    pub ollama: LocalSettings,
    pub textgen: LocalSettings,
    pub lmstudio: LocalSettings,
    pub llamacpp: LocalSettings,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            default: "textgen".into(),
            default_model: None,
            anthropic: AnthropicSettings::default(),
            openai: OpenAISettings::default(),
            ollama: LocalSettings::with_base_url("http://localhost:11434/v1"),
            textgen: LocalSettings::with_base_url("http://localhost:5000/v1"),
            lmstudio: LocalSettings::with_base_url("http://localhost:1234/v1"),
            llamacpp: LocalSettings::with_base_url("http://localhost:8080/v1"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AnthropicSettings {
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
    pub extended_thinking_budget: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAISettings {
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalSettings {
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub api_key: Option<String>,
}

impl LocalSettings {
    fn with_base_url(url: &str) -> Self {
        Self {
            base_url: Some(url.into()),
            default_model: None,
            api_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiSettings {
    /// Active colour scheme. One of the slugs from
    /// `oxidant_gui::theme::Theme::ALL`:
    ///   "espresso" | "monokai" | "dracula" | "one_dark" | "classic_dark"
    /// Unknown values fall back to the default ("espresso").
    pub theme: String,
    /// If true, Enter sends and Shift+Enter inserts a newline.
    pub enter_sends: bool,
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            theme: "espresso".into(),
            enter_sends: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionsSettings {
    /// ReadOnly tools auto-approve unconditionally when this is true (default).
    pub auto_approve_readonly: bool,
    /// Patterns that pre-approve a tool call. Format varies:
    ///   "fs_write"       — exact tool name match
    ///   "bash:cargo *"   — bash glob (substring or globset syntax)
    ///   "bash:/^git /"   — bash regex
    pub allowlist: Vec<String>,
    /// Same shape as allowlist; matches force a Deny.
    pub denylist: Vec<String>,
}

impl Default for PermissionsSettings {
    fn default() -> Self {
        Self {
            auto_approve_readonly: true,
            allowlist: vec![
                // Sensible defaults for the agent loop's common reads.
                "bash:ls *".into(),
                "bash:pwd".into(),
                "bash:cat *".into(),
                "bash:cargo check*".into(),
                "bash:cargo test*".into(),
            ],
            denylist: vec![
                "bash:rm -rf*".into(),
                "bash:rm -fr*".into(),
            ],
        }
    }
}

// ---------------------------------------------------------------- loading

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("read {path}: {message}", path = path.display())]
    Io { path: PathBuf, message: String },
    #[error("parse {path}: {message}", path = path.display())]
    Parse { path: PathBuf, message: String },
}

/// Compute the per-user config path (Linux/macOS: ~/.config/oxidant/config.toml,
/// Windows: %APPDATA%\oxidant\config.toml). None if the platform has no
/// project dirs (CI, locked-down sandboxes).
pub fn user_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("ai", "oxidant", "oxidant")
        .map(|d| d.config_dir().join("config.toml"))
}

/// Per-repo config path for a worktree (always inside `.oxidant/`).
pub fn repo_config_path(worktree: &Path) -> PathBuf {
    worktree.join(".oxidant").join("oxidant.toml")
}

/// Load + merge settings for a given worktree. Missing files are
/// ignored (the defaults still apply). Parse errors fail loudly.
pub fn load(worktree: &Path) -> Result<Settings, SettingsError> {
    let mut settings = Settings::default();

    let repo = repo_config_path(worktree);
    if repo.exists() {
        let txt = std::fs::read_to_string(&repo).map_err(|e| SettingsError::Io {
            path: repo.clone(),
            message: e.to_string(),
        })?;
        let parsed: Settings = toml::from_str(&txt).map_err(|e| SettingsError::Parse {
            path: repo.clone(),
            message: e.to_string(),
        })?;
        settings = parsed;
    }

    if let Some(user) = user_config_path() {
        if user.exists() {
            let txt = std::fs::read_to_string(&user).map_err(|e| SettingsError::Io {
                path: user.clone(),
                message: e.to_string(),
            })?;
            let parsed: Settings = toml::from_str(&txt).map_err(|e| SettingsError::Parse {
                path: user.clone(),
                message: e.to_string(),
            })?;
            settings = parsed; // user wins over repo per the spec
        }
    }

    apply_env_overrides(&mut settings);
    Ok(settings)
}

fn apply_env_overrides(settings: &mut Settings) {
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        settings.provider.anthropic.api_key = Some(key);
    }
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        settings.provider.openai.api_key = Some(key);
    }
    if let Ok(url) = std::env::var("OXIDANT_TEXTGEN_URL") {
        settings.provider.textgen.base_url = Some(url);
    }
    if let Ok(url) = std::env::var("OXIDANT_OLLAMA_URL") {
        settings.provider.ollama.base_url = Some(url);
    }
    if let Ok(url) = std::env::var("OXIDANT_LMSTUDIO_URL") {
        settings.provider.lmstudio.base_url = Some(url);
    }
    if let Ok(provider) = std::env::var("OXIDANT_PROVIDER") {
        settings.provider.default = provider;
    }
    if let Ok(model) = std::env::var("OXIDANT_MODEL") {
        settings.provider.default_model = Some(model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn defaults_have_sensible_local_endpoints() {
        let s = Settings::default();
        assert_eq!(s.provider.default, "textgen");
        assert_eq!(
            s.provider.textgen.base_url.as_deref(),
            Some("http://localhost:5000/v1")
        );
        assert_eq!(
            s.provider.lmstudio.base_url.as_deref(),
            Some("http://localhost:1234/v1")
        );
        assert!(s.permissions.auto_approve_readonly);
        assert!(!s.permissions.allowlist.is_empty());
    }

    #[test]
    fn load_per_repo_overrides_defaults() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".oxidant")).unwrap();
        std::fs::write(
            dir.path().join(".oxidant").join("oxidant.toml"),
            r#"
                [provider]
                default = "lmstudio"
                default_model = "qwen2.5-coder-32b-instruct"

                [gui]
                enter_sends = true
            "#,
        )
        .unwrap();
        let s = load(dir.path()).unwrap();
        assert_eq!(s.provider.default, "lmstudio");
        assert_eq!(
            s.provider.default_model.as_deref(),
            Some("qwen2.5-coder-32b-instruct")
        );
        assert!(s.gui.enter_sends);
    }

    #[test]
    fn env_override_wins() {
        // SAFETY: tests run in a single process; toggle the env only for this scope.
        // We can't reliably scope environment variable changes in parallel tests, so
        // we just assert apply_env_overrides honours a present var.
        let mut s = Settings::default();
        unsafe { std::env::set_var("OXIDANT_PROVIDER", "ollama"); }
        unsafe { std::env::set_var("OXIDANT_MODEL", "llama3.1:8b"); }
        apply_env_overrides(&mut s);
        unsafe { std::env::remove_var("OXIDANT_PROVIDER"); }
        unsafe { std::env::remove_var("OXIDANT_MODEL"); }
        assert_eq!(s.provider.default, "ollama");
        assert_eq!(s.provider.default_model.as_deref(), Some("llama3.1:8b"));
    }

    #[test]
    fn missing_files_use_defaults() {
        let dir = TempDir::new().unwrap();
        let s = load(dir.path()).unwrap();
        assert_eq!(s.provider.default, "textgen");
    }

    #[test]
    fn parse_error_surfaces_the_path() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".oxidant")).unwrap();
        std::fs::write(
            dir.path().join(".oxidant").join("oxidant.toml"),
            "not valid =[ toml",
        )
        .unwrap();
        let err = load(dir.path()).unwrap_err();
        assert!(matches!(err, SettingsError::Parse { .. }));
    }
}
