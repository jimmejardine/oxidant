```yaml
id: typography
kind: component
parent: overview
order: 10
implements: []
depends_on:
  - components/config/settings
code:
  - crates/oxidant-gui/src/theme.rs
  - crates/oxidant-gui/src/viewport.rs
  - crates/oxidant-gui/src/app.rs
status: active
responsibility: "Single-size uniform typography across every panel, with broad-Unicode bundled fonts. Describes the user-facing zoom factor behavior (scroll-wheel hotkey, Ctrl+0 reset, settings slider); settings persistence is owned by [[components/config/settings]]."
```

## Bundled fonts

Oxidant ships **Noto Sans Regular** (proportional) and **Noto Sans Mono Regular** (monospace) under `crates/oxidant-gui/assets/fonts/`. Noto's reason for existing is broad Unicode coverage — the "no tofu" project — so the symbols we use across the GUI (✗ ✓ ⟳ ↩ ⊕ ⌖ ⚠ ⏎) render rather than dropping to missing-glyph boxes.

Both files are SIL Open Font License 1.1 (compatible with our MIT/Apache dual licence). The OFL text is preserved at `crates/oxidant-gui/assets/fonts/LICENSE-OFL.txt`.

`theme::install_fonts(ctx)` installs them as **the primary** entry in egui's Proportional and Monospace families, prepended ahead of egui's existing fallback chain so emoji and rare glyphs egui ships fall through to their established sources. Called once at app startup from `viewport.rs::run_viewport`, **not** from `theme::apply` (which runs every theme switch and shouldn't re-upload the font atlas to the GPU).

## Single font size

`theme.rs` defines `pub const BASE_FONT_PT: f32 = 15.0`. Every `egui::TextStyle` — Heading, Body, Button, Small, Monospace — uses this same point size. Installed inside `theme::apply` via `style.text_styles.insert(...)` so it re-applies on theme switches (egui clones the existing style and `set_style` replaces it wholesale).

**No per-label size overrides.** `RichText::new(...).size(...)` and `.small()` chains are forbidden in panel code — the unified TextStyle is the only knob. Weight / colour / family adjustments (`.strong()`, `.color()`, `.monospace()`, `.italics()`) remain orthogonal and are fine.

If a panel feels visually undifferentiated without size variation, lean on weight (`.strong()`), colour (`theme::muted_text()`, `theme::faint_text()`), and spacing (`ui.add_space(...)`) before reaching for size. The user can scale the whole UI up via the zoom factor; there's no reason to make individual labels small.

## Global zoom factor

User-controllable, persisted, three input paths — all writing the same `GuiSettings.zoom_factor` field:

- **Ctrl+scroll-wheel** over the GUI: ±0.1 per tick, clamped to `0.5..=3.0`. The handler in `App::update` consumes `raw_scroll_delta.y` so `ScrollArea`s underneath don't double-handle the wheel event.
- **Ctrl+0**: resets to `1.0`. Handled in the same `input_mut` block via `consume_key`.
- **Settings panel slider**: a `0.5..=3.0` `egui::Slider` bound to `self.draft.gui.zoom_factor`, with a "Reset (1.0)" button. Live-previews via `ctx.set_zoom_factor(...)` on change so the user sees the scale immediately. The existing Save button persists to disk; Revert restores both the file value and the live factor.

The slider draft mirrors the live `ctx.zoom_factor()` on every render, so opening Settings after Ctrl+scroll shows the current value rather than a stale draft.

Persistence: every change writes through `oxidant_config::save_user`. The TOML is a few hundred bytes — disk writes are cheap; no debouncing in MVP. At app startup, `viewport.rs::run_viewport` reads the persisted value, clamps it to `0.5..=3.0` (guards against a hand-edited out-of-range TOML), and calls `ctx.set_zoom_factor(...)` before the first paint.

## Settings field

```toml
[gui]
theme = "espresso"
enter_sends = false
zoom_factor = 1.0
```

`zoom_factor: f32` lives on `oxidant_config::settings::GuiSettings`. Default `1.0`. Missing field deserialises to default via `#[serde(default = "default_zoom_factor")]`, so existing user TOMLs keep working without migration.

## Out of scope

- Per-text-style font-size overrides. The uniform rule is the whole point.
- A font-family picker in Settings. Themes carry colours; fonts are bundled. Adding a picker is a future feature, not a current concern.
- Light-themed icon swap. Glyphs render as the current theme's text colour; no per-theme icon adjustment needed.
- macOS explicit Cmd+- / Cmd++ for stepped zoom — Ctrl+scroll covers it. Explicit keys are a follow-up if asked.
