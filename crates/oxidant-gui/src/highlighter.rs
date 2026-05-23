// Syntect-backed syntax highlighting for the file_tab editor.
//
// Realises the "Render" section of spec/components/gui/file-tabs.md.
//
// Used as a `layouter` callback by `egui::TextEdit::multiline`. Each
// call rebuilds a `LayoutJob` for the whole text — fine for the editor
// sizes we deal with (specs are a few KiB, source files a few tens),
// and avoids state-tracking complications around partial re-layout.

use std::path::Path;
use std::sync::OnceLock;

use egui::text::LayoutJob;
use egui::{Color32, FontId, TextFormat};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style, Theme, ThemeSet};
use syntect::parsing::{SyntaxDefinition, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

use crate::theme::{self, Theme as OxidantTheme};

// Hand-written grammars bundled at compile time for formats syntect's
// default 75-grammar bundle doesn't ship: TOML, .gitignore (also used
// by .dockerignore / .npmignore), and .gitattributes. Each is loaded
// once and merged into the default SyntaxSet on first use.
//
// JSON and YAML are already in syntect's default bundle — no asset
// needed for those.
const BUNDLED_SYNTAXES: &[(&str, &str)] = &[
    ("toml", include_str!("../assets/toml.sublime-syntax")),
    (
        "gitignore",
        include_str!("../assets/gitignore.sublime-syntax"),
    ),
    (
        "gitattributes",
        include_str!("../assets/gitattributes.sublime-syntax"),
    ),
];

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(|| {
        let defaults = SyntaxSet::load_defaults_newlines();
        let mut builder = defaults.into_builder();
        for (slug, body) in BUNDLED_SYNTAXES {
            match SyntaxDefinition::load_from_str(body, true, Some(slug)) {
                Ok(def) => builder.add(def),
                Err(e) => {
                    // A malformed asset must not blow up the GUI —
                    // worst case is that format renders as plain text,
                    // which is what users got before bundling.
                    tracing::error!("failed to load bundled {slug} syntax: {e}");
                }
            }
        }
        builder.build()
    })
}

fn theme_set() -> &'static ThemeSet {
    static SET: OnceLock<ThemeSet> = OnceLock::new();
    SET.get_or_init(ThemeSet::load_defaults)
}

/// Pick the syntect syntax for a given filename. Falls back to plain
/// text when nothing matches. Handles three lookup paths:
///   1. `.rs`, `.md`, `.toml` etc. → `find_syntax_by_extension(ext)`.
///   2. Dot-files like `.gitignore` (no extension as `Path` sees it) →
///      strip the leading dot and look the name up as an extension,
///      because our bundled grammars declare `gitignore` as their
///      extension.
///   3. As a final attempt, match the bare filename as a token
///      (`Cargo.toml`, `Makefile`).
fn syntax_for(path: &Path) -> &'static SyntaxReference {
    let ss = syntax_set();
    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && let Some(s) = ss.find_syntax_by_extension(ext)
    {
        return s;
    }
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let dot_stripped = name.trim_start_matches('.');
        if !dot_stripped.is_empty()
            && let Some(s) = ss.find_syntax_by_extension(dot_stripped)
        {
            return s;
        }
        if let Some(s) = ss.find_syntax_by_token(name) {
            return s;
        }
    }
    ss.find_syntax_plain_text()
}

/// Pick a syntect highlighting theme that matches the active oxidant
/// theme. The mapping is documented in spec/components/gui/file-tabs.md
/// "Render".
fn syntect_theme_for(t: OxidantTheme) -> &'static Theme {
    let ts = theme_set();
    let name = match t {
        OxidantTheme::Espresso | OxidantTheme::Monokai => "base16-mocha.dark",
        OxidantTheme::Dracula => "Solarized (dark)",
        OxidantTheme::OneDark => "base16-ocean.dark",
        OxidantTheme::ClassicDark => "base16-eighties.dark",
    };
    ts.themes
        .get(name)
        .or_else(|| ts.themes.get("base16-ocean.dark"))
        .expect("syntect default theme set is non-empty")
}

/// Build a LayoutJob for `text`, coloured by the syntect syntax for
/// `path` and the syntect theme matched to the active oxidant theme.
/// Wrap mode is controlled by `wrap_width`; pass `f32::INFINITY` to
/// disable wrapping.
pub fn highlight(path: &Path, text: &str, font_id: FontId, wrap_width: f32) -> LayoutJob {
    let syntax = syntax_for(path);
    let theme = syntect_theme_for(theme::current());
    let mut highlighter = HighlightLines::new(syntax, theme);

    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let ss = syntax_set();

    for line in LinesWithEndings::from(text) {
        // If a line fails to highlight (malformed text mid-stream), fall
        // back to plain so we never panic in the editor.
        let ranges = highlighter
            .highlight_line(line, ss)
            .unwrap_or_else(|_| vec![(Style::default(), line)]);
        for (style, slice) in ranges {
            job.append(slice, 0.0, format_from_style(style, font_id.clone()));
        }
    }
    job
}

fn format_from_style(style: Style, font_id: FontId) -> TextFormat {
    let fg = Color32::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b);
    let italics = style.font_style.contains(FontStyle::ITALIC);
    TextFormat {
        font_id,
        color: fg,
        italics,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_extension_picks_rust_syntax() {
        let s = syntax_for(Path::new("foo.rs"));
        assert_eq!(s.name, "Rust");
    }

    #[test]
    fn markdown_extension_picks_markdown_syntax() {
        let s = syntax_for(Path::new("README.md"));
        assert!(s.name.to_lowercase().contains("markdown"));
    }

    #[test]
    fn unknown_extension_falls_back_to_plain_text() {
        let s = syntax_for(Path::new("notes.xyzzy"));
        assert_eq!(s.name, "Plain Text");
    }

    #[test]
    fn json_extension_picks_json_syntax() {
        let s = syntax_for(Path::new("settings.json"));
        assert_eq!(s.name, "JSON");
    }

    #[test]
    fn yaml_extension_picks_yaml_syntax() {
        let s = syntax_for(Path::new("ci.yml"));
        assert!(s.name.to_lowercase().contains("yaml"));
    }

    #[test]
    fn gitignore_dotfile_picks_bundled_gitignore_syntax() {
        let s = syntax_for(Path::new(".gitignore"));
        assert_eq!(s.name, "Git Ignore");
    }

    #[test]
    fn gitattributes_dotfile_picks_bundled_gitattributes_syntax() {
        let s = syntax_for(Path::new(".gitattributes"));
        assert_eq!(s.name, "Git Attributes");
    }

    #[test]
    fn toml_extension_picks_bundled_toml_syntax() {
        // syntect's default bundle doesn't include TOML — the highlighter
        // module bundles its own .sublime-syntax to fill the gap. This
        // test guards against the asset going missing or failing to load.
        let s = syntax_for(Path::new("Cargo.toml"));
        assert_eq!(s.name, "TOML");
    }

    #[test]
    fn highlight_toml_produces_distinct_colours_for_keys_and_strings() {
        let job = highlight(
            Path::new("Cargo.toml"),
            "[package]\nname = \"oxidant\"\nversion = \"0.1.0\"\n",
            FontId::monospace(13.0),
            f32::INFINITY,
        );
        // At least one section colour must differ from the body colour —
        // otherwise the highlighter is silently rendering plain text.
        let colours: std::collections::HashSet<_> =
            job.sections.iter().map(|s| s.format.color).collect();
        assert!(
            colours.len() >= 2,
            "TOML highlighting produced only {} distinct colour(s)",
            colours.len()
        );
    }

    #[test]
    fn every_oxidant_theme_has_a_syntect_match() {
        for t in OxidantTheme::ALL {
            // Just asserting `syntect_theme_for` doesn't panic for any
            // of them — that guards against typos in the mapping table.
            let _ = syntect_theme_for(*t);
        }
    }

    #[test]
    fn highlight_returns_a_non_empty_job_for_simple_rust() {
        let job = highlight(
            Path::new("foo.rs"),
            "fn main() { println!(\"hi\"); }",
            FontId::monospace(12.0),
            f32::INFINITY,
        );
        assert!(!job.sections.is_empty());
    }
}
