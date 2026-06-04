```yaml
id: theme
kind: component
parent: overview
order: 8
implements: []
depends_on:
  - components/gui/viewport
  - components/config/settings
code:
  - crates/oxidant-gui/src/theme.rs
status: active
responsibility: |
  Provide a small set of switchable colour schemes for the viewport, expose the active theme's secondary text colours to every panel, and persist the user's choice in settings.
```

## Themes shipped

Five palettes, each chosen for being immediately recognisable to anyone who has used a code editor:

| Slug          | Name          | Origin                          |
|---------------|---------------|---------------------------------|
| `espresso`    | Espresso      | TextMate / iTerm classic        |
| `monokai`     | Monokai       | Sublime / TextMate classic      |
| `dracula`     | Dracula       | draculatheme.com                |
| `one_dark`    | One Dark      | Atom / VS Code "One Dark Pro"   |
| `classic_dark`| Classic Dark  | High-contrast terminal aesthetic|

Default: **Espresso**.

Adding a sixth (e.g. Solarized, Nord, Gruvbox) means adding a `palette()` arm plus an `ALL` entry — no other code changes.

## Active theme + secondary text

`theme::apply(&ctx, Theme::X)` does two things:
1. `ctx.set_visuals(...)` — egui consumes the palette's `Visuals`.
2. Records the muted/faint colours in a process-global slot.

Panels then call `theme::muted_text()` and `theme::faint_text()` for de-emphasised labels (status hints, file paths in diagnostics, deprecated-spec strikethrough). They MUST NOT hard-code `Color32::DARK_GRAY` / `LIGHT_GRAY`: those colours are illegible on at least one shipped theme.

## Persistence

The active theme slug lives in `[gui] theme = "..."` under settings (see [[components/config/settings]]). Unknown slugs fall back to the default.

Changes flow:
- Startup: `oxidant_config::load(...)` → `Theme::from_slug(&s.gui.theme)` → `theme::apply` in the viewport creation closure.
- Runtime switch: the Settings tab ([[components/gui/settings-panel]]) owns the theme picker. It updates `App::active_theme`, calls `theme::apply(ui.ctx(), …)`, and writes the new slug back to settings. There is no separate top-bar menu for theme — earlier revisions carried a View → Theme submenu, but it was removed once Settings shipped, so users have one obvious place to tune everything.

## Light themes

Out of scope for the MVP. Every shipped theme is dark. A `Theme::Light` family would extend the enum and require auditing every hard-coded colour in the panels (the kind-tag rainbow in the spec tree, the YELLOW/RED in diagnostics) for legibility on a light background.
