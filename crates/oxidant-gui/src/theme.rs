// Dark palette used across every panel. Applied once at viewport startup
// via `cc.egui_ctx.set_visuals(theme::dark_palette())`. egui's default
// `Visuals::dark()` sits at a mid-grey background that washes out our
// muted secondary text into illegibility; this palette pushes the
// background deeper and the text contrast higher.
//
// Public colour constants are exposed so panels can opt into "muted"
// text without hard-coding `Color32::DARK_GRAY`, which is invisible
// against the deeper background here.

use egui::{Color32, Visuals, style::Selection};

// -- Backgrounds (darkest → lightest) ------------------------------------
const BG_DEEP: Color32 = Color32::from_rgb(14, 16, 20); // code / extreme bg
const BG_WINDOW: Color32 = Color32::from_rgb(20, 22, 27); // app window
const BG_PANEL: Color32 = Color32::from_rgb(24, 26, 31); // dock surface
const BG_WIDGET: Color32 = Color32::from_rgb(32, 35, 42); // buttons, inputs
const BG_WIDGET_HOVERED: Color32 = Color32::from_rgb(42, 46, 56);
const BG_WIDGET_ACTIVE: Color32 = Color32::from_rgb(54, 60, 72);
const STROKE_SUBTLE: Color32 = Color32::from_rgb(46, 50, 60);

// -- Foreground text -----------------------------------------------------
const TEXT_STRONG: Color32 = Color32::from_rgb(232, 234, 240);
const TEXT_BODY: Color32 = Color32::from_rgb(200, 205, 215);

/// Use for de-emphasised text that still needs to be legible (status
/// hints, "no items yet" placeholders, file paths in diagnostics).
/// Replaces panel-level uses of `Color32::DARK_GRAY`.
pub const MUTED_TEXT: Color32 = Color32::from_rgb(150, 156, 172);

/// Use for tertiary information that should fade into the background
/// without disappearing entirely (separator labels, post-turn summary).
pub const FAINT_TEXT: Color32 = Color32::from_rgb(120, 126, 142);

// -- Accents -------------------------------------------------------------
const ACCENT: Color32 = Color32::from_rgb(122, 162, 247);
const WARN: Color32 = Color32::from_rgb(255, 200, 100);
const ERROR: Color32 = Color32::from_rgb(247, 118, 142);

pub fn dark_palette() -> Visuals {
    let mut v = Visuals::dark();

    v.window_fill = BG_WINDOW;
    v.panel_fill = BG_PANEL;
    v.extreme_bg_color = BG_DEEP;
    v.faint_bg_color = BG_WIDGET;
    v.code_bg_color = BG_DEEP;

    v.override_text_color = Some(TEXT_BODY);

    v.widgets.noninteractive.bg_fill = BG_PANEL;
    v.widgets.noninteractive.weak_bg_fill = BG_PANEL;
    v.widgets.noninteractive.fg_stroke.color = TEXT_BODY;
    v.widgets.noninteractive.bg_stroke.color = STROKE_SUBTLE;

    v.widgets.inactive.bg_fill = BG_WIDGET;
    v.widgets.inactive.weak_bg_fill = BG_WIDGET;
    v.widgets.inactive.fg_stroke.color = TEXT_BODY;
    v.widgets.inactive.bg_stroke.color = STROKE_SUBTLE;

    v.widgets.hovered.bg_fill = BG_WIDGET_HOVERED;
    v.widgets.hovered.weak_bg_fill = BG_WIDGET_HOVERED;
    v.widgets.hovered.fg_stroke.color = TEXT_STRONG;
    v.widgets.hovered.bg_stroke.color = ACCENT;

    v.widgets.active.bg_fill = BG_WIDGET_ACTIVE;
    v.widgets.active.weak_bg_fill = BG_WIDGET_ACTIVE;
    v.widgets.active.fg_stroke.color = TEXT_STRONG;
    v.widgets.active.bg_stroke.color = ACCENT;

    v.widgets.open.bg_fill = BG_WIDGET_ACTIVE;
    v.widgets.open.weak_bg_fill = BG_WIDGET_ACTIVE;
    v.widgets.open.fg_stroke.color = TEXT_STRONG;
    v.widgets.open.bg_stroke.color = ACCENT;

    v.selection = Selection {
        bg_fill: Color32::from_rgba_unmultiplied(122, 162, 247, 100),
        stroke: egui::Stroke::new(1.0, ACCENT),
    };

    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.error_fg_color = ERROR;

    v
}
