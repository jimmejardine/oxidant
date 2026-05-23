// Switchable colour palettes for the GUI. Five themes ship out of the
// box (Espresso, Monokai, Dracula, One Dark, Classic Dark) — the
// active one is selected at viewport startup from settings and can
// be flipped at runtime via the View → Theme menu.
//
// Why this lives in a single module:
//   - The five Visuals tables differ only in colour constants; sharing
//     the same builder keeps the shapes (button corner radius, stroke
//     width, selection alpha) consistent across themes.
//   - Panels look up the active theme's muted/faint text via
//     `theme::muted_text()` / `theme::faint_text()` so a theme switch
//     reaches every label without re-plumbing args through render
//     calls.
//
// "Why not egui's `Visuals::dark()`?" — its background is a mid-grey
// that washes panel-level muted text (Color32::DARK_GRAY) into
// illegibility. Every theme here defines its own muted band that's
// actually readable against its own ground.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use egui::{Color32, Visuals, style::Selection};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- Theme enum

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    Espresso,
    Monokai,
    Dracula,
    OneDark,
    /// Classic dark terminal palette (formerly "Midnight" in the
    /// repo's working name) — near-black ground, white text, cyan
    /// accent. Highest contrast of the bunch.
    ClassicDark,
}

impl Theme {
    /// Order in the menu, sticky for the user.
    pub const ALL: &'static [Theme] = &[
        Theme::Espresso,
        Theme::Monokai,
        Theme::Dracula,
        Theme::OneDark,
        Theme::ClassicDark,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Theme::Espresso => "Espresso",
            Theme::Monokai => "Monokai",
            Theme::Dracula => "Dracula",
            Theme::OneDark => "One Dark",
            Theme::ClassicDark => "Classic Dark",
        }
    }

    /// Settings-file slug. Stable across renames.
    pub fn slug(self) -> &'static str {
        match self {
            Theme::Espresso => "espresso",
            Theme::Monokai => "monokai",
            Theme::Dracula => "dracula",
            Theme::OneDark => "one_dark",
            Theme::ClassicDark => "classic_dark",
        }
    }

    pub fn from_slug(s: &str) -> Option<Theme> {
        Theme::ALL
            .iter()
            .copied()
            .find(|t| t.slug().eq_ignore_ascii_case(s))
    }

    pub fn palette(self) -> Palette {
        match self {
            Theme::Espresso => espresso(),
            Theme::Monokai => monokai(),
            Theme::Dracula => dracula(),
            Theme::OneDark => one_dark(),
            Theme::ClassicDark => classic_dark(),
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            Theme::Espresso => 0,
            Theme::Monokai => 1,
            Theme::Dracula => 2,
            Theme::OneDark => 3,
            Theme::ClassicDark => 4,
        }
    }

    fn from_u8(n: u8) -> Theme {
        match n {
            1 => Theme::Monokai,
            2 => Theme::Dracula,
            3 => Theme::OneDark,
            4 => Theme::ClassicDark,
            _ => Theme::Espresso,
        }
    }
}

// ---------------------------------------------------------------- Palette

/// A theme's full colour set. `visuals` is what egui consumes; the
/// loose `muted` / `faint` colours are surfaced separately so panels
/// can paint secondary text without poking at the Visuals struct.
pub struct Palette {
    pub visuals: Visuals,
    pub muted: Color32,
    pub faint: Color32,
}

// ---------------------------------------------------------------- Globals

/// Active theme tag. Cheap atomic load on every `muted_text()` /
/// `faint_text()` call, avoiding a Mutex on the hot path.
static CURRENT_TAG: AtomicU8 = AtomicU8::new(0);
/// Cached muted/faint colours for the active theme so panels don't
/// rebuild a whole Visuals on every label. Updated alongside
/// `CURRENT_TAG` from `apply`.
static CURRENT_COLOURS: Mutex<(Color32, Color32)> = Mutex::new((
    // Initialised to Espresso's muted/faint values; `apply()` overwrites
    // before any panel renders.
    Color32::from_rgb(181, 162, 134),
    Color32::from_rgb(139, 123, 101),
));

/// Apply `theme` to the egui context and remember it as the active
/// theme so panel-level `muted_text()` / `faint_text()` agree.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    let p = theme.palette();
    ctx.set_visuals(p.visuals);
    CURRENT_TAG.store(theme.as_u8(), Ordering::Relaxed);
    if let Ok(mut g) = CURRENT_COLOURS.lock() {
        *g = (p.muted, p.faint);
    }
}

pub fn current() -> Theme {
    Theme::from_u8(CURRENT_TAG.load(Ordering::Relaxed))
}

/// Secondary text colour for the active theme. Replaces hard-coded
/// `Color32::DARK_GRAY` uses across the panels.
pub fn muted_text() -> Color32 {
    CURRENT_COLOURS.lock().map(|g| g.0).unwrap_or(Color32::GRAY)
}

/// Tertiary text colour for the active theme — softer than `muted_text`,
/// still legible.
pub fn faint_text() -> Color32 {
    CURRENT_COLOURS
        .lock()
        .map(|g| g.1)
        .unwrap_or(Color32::DARK_GRAY)
}

// ---------------------------------------------------------------- Builder

#[allow(clippy::too_many_arguments)] // palette-builder takes one Color32 per band by design
fn build_visuals(
    bg_deep: Color32,
    bg_window: Color32,
    bg_panel: Color32,
    bg_widget: Color32,
    bg_hover: Color32,
    bg_active: Color32,
    stroke: Color32,
    text_body: Color32,
    text_strong: Color32,
    accent: Color32,
    warn: Color32,
    error: Color32,
) -> Visuals {
    let mut v = Visuals::dark();

    v.window_fill = bg_window;
    v.panel_fill = bg_panel;
    v.extreme_bg_color = bg_deep;
    v.faint_bg_color = bg_widget;
    v.code_bg_color = bg_deep;

    v.override_text_color = Some(text_body);

    v.widgets.noninteractive.bg_fill = bg_panel;
    v.widgets.noninteractive.weak_bg_fill = bg_panel;
    v.widgets.noninteractive.fg_stroke.color = text_body;
    v.widgets.noninteractive.bg_stroke.color = stroke;

    v.widgets.inactive.bg_fill = bg_widget;
    v.widgets.inactive.weak_bg_fill = bg_widget;
    v.widgets.inactive.fg_stroke.color = text_body;
    v.widgets.inactive.bg_stroke.color = stroke;

    v.widgets.hovered.bg_fill = bg_hover;
    v.widgets.hovered.weak_bg_fill = bg_hover;
    v.widgets.hovered.fg_stroke.color = text_strong;
    v.widgets.hovered.bg_stroke.color = accent;

    v.widgets.active.bg_fill = bg_active;
    v.widgets.active.weak_bg_fill = bg_active;
    v.widgets.active.fg_stroke.color = text_strong;
    v.widgets.active.bg_stroke.color = accent;

    v.widgets.open.bg_fill = bg_active;
    v.widgets.open.weak_bg_fill = bg_active;
    v.widgets.open.fg_stroke.color = text_strong;
    v.widgets.open.bg_stroke.color = accent;

    let mut sel = accent;
    sel = Color32::from_rgba_unmultiplied(sel.r(), sel.g(), sel.b(), 100);
    v.selection = Selection {
        bg_fill: sel,
        stroke: egui::Stroke::new(1.0, accent),
    };

    v.hyperlink_color = accent;
    v.warn_fg_color = warn;
    v.error_fg_color = error;

    v
}

// ---------------------------------------------------------------- Espresso
// TextMate / iTerm classic — warm dark brown, cream text, amber accent.

fn espresso() -> Palette {
    let muted = Color32::from_rgb(181, 162, 134);
    let faint = Color32::from_rgb(139, 123, 101);
    let visuals = build_visuals(
        Color32::from_rgb(20, 17, 12),
        Color32::from_rgb(42, 36, 25),
        Color32::from_rgb(49, 42, 31),
        Color32::from_rgb(61, 53, 39),
        Color32::from_rgb(75, 64, 47),
        Color32::from_rgb(92, 79, 58),
        Color32::from_rgb(75, 64, 47),
        Color32::from_rgb(226, 212, 188),
        Color32::from_rgb(245, 233, 215),
        Color32::from_rgb(255, 198, 109), // amber
        Color32::from_rgb(255, 198, 109),
        Color32::from_rgb(210, 82, 82),
    );
    Palette {
        visuals,
        muted,
        faint,
    }
}

// ---------------------------------------------------------------- Monokai
// Sublime / TextMate classic. BG #272822, FG #F8F8F2, comments #75715E
// (which is exactly our "muted" band — perfect alignment).

fn monokai() -> Palette {
    let muted = Color32::from_rgb(117, 113, 94); // #75715E
    let faint = Color32::from_rgb(85, 82, 68);
    let visuals = build_visuals(
        Color32::from_rgb(28, 29, 24),
        Color32::from_rgb(39, 40, 34), // #272822
        Color32::from_rgb(46, 47, 39),
        Color32::from_rgb(58, 59, 50),
        Color32::from_rgb(72, 73, 62),
        Color32::from_rgb(89, 90, 76),
        Color32::from_rgb(72, 73, 62),
        Color32::from_rgb(248, 248, 242), // #F8F8F2
        Color32::from_rgb(253, 253, 248),
        Color32::from_rgb(166, 226, 46), // #A6E22E green
        Color32::from_rgb(253, 151, 31), // #FD971F orange
        Color32::from_rgb(249, 38, 114), // #F92672 pink/red
    );
    Palette {
        visuals,
        muted,
        faint,
    }
}

// ---------------------------------------------------------------- Dracula
// draculatheme.com. BG #282A36, current-line #44475A, FG #F8F8F2,
// comment #6272A4 (cool indigo).

fn dracula() -> Palette {
    let muted = Color32::from_rgb(98, 114, 164); // #6272A4
    let faint = Color32::from_rgb(80, 90, 130);
    let visuals = build_visuals(
        Color32::from_rgb(30, 32, 41),
        Color32::from_rgb(40, 42, 54), // #282A36
        Color32::from_rgb(48, 50, 64),
        Color32::from_rgb(68, 71, 90), // #44475A
        Color32::from_rgb(85, 89, 113),
        Color32::from_rgb(108, 113, 144),
        Color32::from_rgb(68, 71, 90),
        Color32::from_rgb(248, 248, 242), // #F8F8F2
        Color32::from_rgb(255, 255, 252),
        Color32::from_rgb(189, 147, 249), // #BD93F9 purple
        Color32::from_rgb(241, 250, 140), // #F1FA8C yellow
        Color32::from_rgb(255, 85, 85),   // #FF5555 red
    );
    Palette {
        visuals,
        muted,
        faint,
    }
}

// ---------------------------------------------------------------- One Dark
// Atom / VS Code "One Dark Pro". BG #282C34, FG #ABB2BF, comment #5C6370.

fn one_dark() -> Palette {
    let muted = Color32::from_rgb(92, 99, 112); // #5C6370
    let faint = Color32::from_rgb(75, 81, 92);
    let visuals = build_visuals(
        Color32::from_rgb(30, 33, 39),
        Color32::from_rgb(40, 44, 52), // #282C34
        Color32::from_rgb(48, 52, 62),
        Color32::from_rgb(58, 63, 75),
        Color32::from_rgb(73, 79, 93),
        Color32::from_rgb(92, 99, 112),
        Color32::from_rgb(58, 63, 75),
        Color32::from_rgb(171, 178, 191), // #ABB2BF body
        Color32::from_rgb(220, 223, 228), // #DCDFE4 strong
        Color32::from_rgb(97, 175, 239),  // #61AFEF blue
        Color32::from_rgb(229, 192, 123), // #E5C07B yellow
        Color32::from_rgb(224, 108, 117), // #E06C75 red
    );
    Palette {
        visuals,
        muted,
        faint,
    }
}

// ---------------------------------------------------------------- Classic Dark
// The "Midnight" working name. Near-black ground, white text, cyan
// accent — the high-contrast terminal aesthetic.

fn classic_dark() -> Palette {
    let muted = Color32::from_rgb(170, 170, 170);
    let faint = Color32::from_rgb(120, 120, 120);
    let visuals = build_visuals(
        Color32::from_rgb(0, 0, 0),
        Color32::from_rgb(10, 10, 10),
        Color32::from_rgb(16, 16, 16),
        Color32::from_rgb(28, 28, 28),
        Color32::from_rgb(44, 44, 44),
        Color32::from_rgb(60, 60, 60),
        Color32::from_rgb(60, 60, 60),
        Color32::from_rgb(232, 232, 232),
        Color32::from_rgb(255, 255, 255),
        Color32::from_rgb(0, 200, 220), // cyan
        Color32::from_rgb(255, 200, 100),
        Color32::from_rgb(255, 100, 100),
    );
    Palette {
        visuals,
        muted,
        faint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_roundtrip() {
        for t in Theme::ALL {
            assert_eq!(Theme::from_slug(t.slug()), Some(*t));
        }
    }

    #[test]
    fn from_slug_is_case_insensitive() {
        assert_eq!(Theme::from_slug("ONE_DARK"), Some(Theme::OneDark));
        assert_eq!(Theme::from_slug("Espresso"), Some(Theme::Espresso));
    }

    #[test]
    fn from_slug_unknown_returns_none() {
        assert!(Theme::from_slug("solarized").is_none());
    }

    #[test]
    fn every_theme_has_a_distinct_muted_band() {
        // Guards against an accidental copy-paste between palettes.
        let muteds: Vec<_> = Theme::ALL.iter().map(|t| t.palette().muted).collect();
        for (i, a) in muteds.iter().enumerate() {
            for (j, b) in muteds.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "theme {i} and {j} share a muted colour");
                }
            }
        }
    }

    #[test]
    fn apply_then_muted_text_returns_that_themes_muted() {
        let ctx = egui::Context::default();
        for t in Theme::ALL {
            apply(&ctx, *t);
            assert_eq!(muted_text(), t.palette().muted);
            assert_eq!(faint_text(), t.palette().faint);
            assert_eq!(current(), *t);
        }
    }

    #[test]
    fn default_is_espresso() {
        assert_eq!(Theme::default(), Theme::Espresso);
    }
}
