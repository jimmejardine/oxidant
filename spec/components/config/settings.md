```yaml
id: settings
kind: component
parent: overview
order: 1
implements: []
depends_on:
  - components/gui/typography
code:
  - crates/oxidant-config/src/settings.rs
status: active
responsibility: |
  Load, merge, and serve oxidant's configuration from per-repo and per-user oxidant.toml files.
```

## File locations

| Scope | Path | Purpose |
|---|---|---|
| Per-repo | `<worktree>/.oxidant/oxidant.toml` | Project conventions, tool allowlists, model defaults. Tracked or gitignored at the user's discretion. |
| Per-user | `~/.config/oxidant/config.toml` (Linux/macOS), `%APPDATA%\oxidant\config.toml` (Windows) | API keys, provider preferences, theme. |
| Env vars | `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc. | Override any settings value. |

## Schema (sketch)

```toml
[provider]
default = "anthropic"
default_model = "claude-opus-4-7"

[provider.anthropic]
api_key_env = "ANTHROPIC_API_KEY"
extended_thinking_budget = 8000

[provider.ollama]
base_url = "http://localhost:11434/v1"
default_model = "llama3"

[gui]
theme = "system"            # light | dark | system
enter_sends = false         # if true, Enter sends and Shift+Enter inserts newline
zoom_factor = 1.0           # global UI scale (0.5..=3.0); see components/gui/typography

[permissions]
auto_approve_readonly = true
allowlist = ["cargo check", "cargo test", "ls", "pwd"]
denylist = []
```

## Merge order

env > per-user > per-repo > built-in defaults.

## Hot reload

The config file is watched (`notify`). On change, the settings struct is rebuilt and propagated to subscribers via `tokio::sync::watch::Sender<Settings>`.

## Validation

Schema validation on load with friendly error reporting; invalid settings fail loudly with the offending file + line.

## Test override

When `OXIDANT_CONFIG_PATH` is set, `user_config_path()` returns the env-var value verbatim and skips the `directories::ProjectDirs` lookup. Intended for tests that need to isolate from the host's real user config — `load()` will read (or fail to find) that file instead of the user's actual `~/.config/oxidant/config.toml`. Production code never sets this env var.

Safe under `cargo nextest` (process-per-test); racy under `cargo test`'s parallel single-process runner because env-var mutations leak across tests in the same process. The project standard is nextest (see CLAUDE.md), and the env-var mutations in tests are marked `unsafe` to flag the Rust 2024 contract that the runtime makes no thread-safety guarantee.
