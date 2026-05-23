// User-level settings and permission state.
//
// Specs:
//   spec/components/config/settings.md
//   spec/components/config/permissions.md
//   spec/decisions/0002-no-built-in-sandbox.md

pub mod permissions;
pub mod settings;

pub use permissions::{PermissionDecision, PermissionEngine, PermissionState};
pub use settings::{
    AnthropicSettings, GuiSettings, LocalSettings, OpenAISettings, PermissionsSettings,
    ProviderSettings, Settings, SettingsError, load, repo_config_path, save_user, user_config_path,
};
