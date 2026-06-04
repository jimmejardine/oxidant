```yaml
id: edit-settings
kind: flow
parent: overview
order: 12
status: active
responsibility: |
  User opens the Settings dock tab, mutates a draft of Settings, hits Save — the user-level TOML is rewritten and the in-process Settings is updated so other panels (theme, chat input, permissions) see the change on the next frame.
depends_on:
  - components/config/settings
  - components/config/permissions
  - components/gui/dock-layout
  - components/gui/theme
  - tools/spec/spec-validate
```

# Edit and persist user settings

The runtime configuration loop. Replaces "edit the TOML file in your text editor and restart" with an in-app panel.

## Trigger

- **First-time setup.** A new user opens oxidant and the Settings tab is visible by default in the right dock area (grouped with Diagnostics). They go straight to Providers and paste an API key.
- **Live change.** User opens Window → Settings (or selects the Settings tab in the right dock leaf) to flip a permission, switch a theme, point at a different LLM endpoint, etc.

## Steps

1. **Snapshot.** On panel construction, the [[components/gui/dock-layout]]-hosted SettingsPanel reads the shared `Arc<Mutex<Settings>>` and stores two copies:
   - `baseline` — the last-saved state, used to compute the dirty flag and power Revert.
   - `draft` — the live edit buffer, mutated by widgets every frame.

   Multi-line lists (`permissions.allowlist`, `permissions.denylist`) are mirrored into `allowlist_text` / `denylist_text` newline-joined strings — egui's multi-line `TextEdit` works in strings, not in `Vec<String>`. These strings are re-parsed back into `Vec<String>` on every frame before the dirty check.

2. **Render the sections.** The panel groups settings into three collapsing headers:
   - **Providers.** Active provider combo + default model + per-provider sub-headers (Anthropic, OpenAI, Ollama, textgen-webui, LM Studio, llama.cpp). API key fields are masked by default with a 👁 reveal toggle.
   - **GUI.** Theme combo (live-applies via [[components/gui/theme]]::apply on selection) + enter_sends checkbox.
   - **Permissions.** auto_approve_readonly checkbox + multi-line allowlist + multi-line denylist, with pattern-syntax help text alongside.

3. **Track dirty state.** Each frame, after mirroring the list editors back into the draft, compare `draft != baseline`. If different, enable the Save and Revert buttons; show an "unsaved changes" hint.

4. **Save.** Save button invokes [[components/config/settings]]::save_user(&draft):
   - Serialises Settings to TOML.
   - Creates the parent directory if missing (`~/.config/oxidant/` on Linux/macOS, `%APPDATA%\oxidant\` on Windows).
   - Writes atomically (`std::fs::write` truncates + writes; future hardening could do temp-file + rename).

   On success: `baseline = draft.clone()`; update the shared `Arc<Mutex<Settings>>` so other panels observe the change; show "saved → <path>" inline. On failure (`NoUserConfigDir`, `Serialize`, `Io`): show the error in red and leave the baseline untouched so the user can retry.

5. **Revert.** Drops all unsaved changes: `draft = baseline.clone()`, refresh the list-editor strings.

6. **Theme live-apply.** Theme is special-cased: changing the combo triggers [[components/gui/theme]]::apply on the current ctx *immediately*, before Save. This gives the user a real preview ("does this palette work for me?") instead of forcing a Save-and-see cycle. The View → Theme menu's radio state is also updated via the shared `active_theme` so the two surfaces stay in sync.

## What Save affects on next launch

Subsequent `oxidant` invocations call [[components/config/settings]]::load, which merges (lowest precedence to highest):

```
built-in defaults  <  per-repo TOML  <  per-user TOML  <  env vars
```

Anything saved here lands in the per-user TOML and therefore beats per-repo settings on subsequent launches. Environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OXIDANT_PROVIDER`, etc.) still win at startup — explicit env intent overrides persisted preferences.

## What it doesn't (yet) affect mid-session

- **Active provider switch.** Today the App holds a single `Provider` trait object constructed at launch from the *initial* settings. Changing the active provider in Settings updates the shared lock and the next launch will pick it up, but the running session still uses the original. Hot-swap is future work — likely a small refactor where the chat path looks up the active provider from the shared lock per turn rather than caching it on App construction.
- **Permission engine.** Same shape: the agent loop's per-turn ToolContext doesn't currently consult the live PermissionsSettings; the engine is instantiated once. Acceptable for MVP because the engine is only consulted from the agent loop and the user can simply re-launch after a denylist change.

These deferrals are visible in the [[components/gui/dock-layout]] / [[components/config/settings]] specs and are not a flow-correctness problem — Save still persists, just doesn't propagate live to every consumer.

## See also

- [[components/config/settings]] — the load/save implementation
- [[components/config/permissions]] — what the saved patterns drive
- [[flows/tool-permission-check]] — how the saved allowlist/denylist are consulted at run time
