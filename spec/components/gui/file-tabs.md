---
id: file-tabs
kind: component
parent: overview
order: 7
implements: []
depends_on:
  - components/gui/dock-layout
code:
  - crates/oxidant-gui/src/panels/file_tab.rs
status: active
responsibility: |
  Render an opened code or spec file as a centre-area dock tab — markdown files default to a rendered preview with a Preview/Source toggle, everything else as a syntax-highlighted editor — plus live diagnostic markers.
---

Opened files dock as siblings of the [[components/gui/transcript-tab]] in the centre tab group.

## File sources

- **Code**: `crates/**/*.rs`, `Cargo.toml`, and anything else opened from [[components/gui/file-tree-panel]]. **Editable** via the same flow as Spec. Letting the user hand-edit code while the agent is also editing it does risk contention with `expected_text` checks; the user accepts that trade-off in exchange for being able to make targeted manual fixes without round-tripping through the chat. v2 may add a soft lock against the agent while a code tab is dirty.
- **Spec**: `spec/**/*.md`. **Editable** — opened via double-click from the spec tree. Markdown files open in **rendered preview** by default (see "Render"); the Source view is a raw multi-line text editor with a Save button. Edits are flushed to disk only when the user presses Save. While unsaved, the tab title carries a `●` marker.

Both sources share the edit-lifecycle, the on-disk-changed banner, and the same `SharedState::editor_buffers` map. The only difference is which syntax definition the highlighter loads.

### Edit lifecycle for specs

1. Tab opens → contents are loaded from disk into `SharedState::editor_buffers[path]` (a `HashMap<PathBuf, EditorBuffer>` keyed by absolute path).
2. The text edit binds to the buffer's `text` field. Mutations flip `dirty = true` and update `mtime_at_load` is NOT touched.
3. Save button is enabled iff `dirty`. Click writes the buffer to disk; on success, clears `dirty`. On error, surfaces the error inline in red and leaves `dirty` set.
4. If the on-disk mtime advances while a tab is dirty (the agent edited the file out from under us), a banner offers Reload / Discard. Conflict resolution beyond that is out of scope for the MVP.

The buffer survives a tab close+reopen — the user can dock-close a spec tab without losing unsaved changes until they explicitly Discard. (This is opinionated; the alternative — prompting on close — is more ceremonial than it's worth for a single-user editor.)

## Render

### Markdown preview toggle

Any markdown file (`.md` / `.markdown`, regardless of Code or Spec source) carries a **Preview | Source** toggle in the tab header. It opens in **Preview** by default — the file rendered through [`egui_commonmark`](https://crates.io/crates/egui_commonmark) (headings, lists, code blocks, tables, links). Preview is **read-only**; to edit, the user flips to **Source**, which is the highlighted `egui::TextEdit` described below. The toggle's per-file state lives alongside the buffer in `SharedState::editor_buffers` (`EditorBuffer::view_mode`); the `egui_commonmark::CommonMarkCache` is held once on `App` and threaded into the tab so image/parse state survives across frames. Non-markdown files never show the toggle and always render the editor.

### Source / non-markdown editor

The Source view (and every non-markdown file) renders through a multi-line `egui::TextEdit` with a `layouter` callback that paints **syntect-driven syntax highlighting** in place. The highlighter lives in `crates/oxidant-gui/src/highlighter.rs` and:

- Picks the syntect `SyntaxReference` by file extension (`.rs` → Rust, `.md` → Markdown, `.toml` → TOML, `.json` → JSON, `.yml` / `.yaml` → YAML, falling back to plain text on unknown extensions).
- Dot-files (`.gitignore`, `.gitattributes`, `.dockerignore`, `.npmignore`) are routed by stripping the leading `.` and re-looking-up as an extension, so the bundled grammars match cleanly.
- **Hand-written grammars bundled** under `crates/oxidant-gui/assets/` and merged into the syntect default set at first use:
  - `toml.sublime-syntax` — comments, table headers (`[a.b]` / `[[a.b]]`), bare and quoted keys, all four string forms, hex/oct/bin/float numbers, ISO 8601 dates, booleans.
  - `gitignore.sublime-syntax` — `#` comments, leading `!` negation, glob metacharacters, trailing `/` directory marker. Also handles `.dockerignore` / `.npmignore`.
  - `gitattributes.sublime-syntax` — `#` comments, pathspec on column 0, the well-known attribute names (`text`, `binary`, `eol`, `diff`, `merge`, …), `!` / `=` operators.
  Each grammar is intentionally minimal — visual differentiation for editing, not a complete parser. JSON and YAML are already in syntect's default bundle and need no asset.
- Caches `SyntaxSet` and `ThemeSet` in a `OnceLock` so subsequent edits don't repay parsing cost.
- Maps the active oxidant theme ([[components/gui/theme]]) onto a syntect highlighting theme:
  - Espresso, Monokai → `Monokai`
  - Dracula → `Solarized (dark)` (closest dark contrast match in syntect's defaults)
  - One Dark → `base16-ocean.dark`
  - Classic Dark → `base16-eighties.dark`
- Returns an `egui::text::LayoutJob` per visible line so the TextEdit lays out coloured spans without us shipping our own glyph cache.

**Go to line**: in the editable Source view, **Ctrl+G** (while the editor has focus) opens a small modal dialog that accepts a line number; on OK the caret moves to the start of that line and the view scrolls it into focus. Out-of-range input is clamped to `[1, line count]`. The read-only Selected preview has no caret and ignores Ctrl+G.

**Line numbers** render in a left gutter alongside the highlighted text (muted, monospace, right-aligned), for both the editable Source view and the read-only Selected preview. To keep the gutter aligned 1:1 with logical lines, the code views are **no-wrap with horizontal scroll** (the layouter is invoked with `wrap_width = f32::INFINITY`). The markdown Preview has no gutter. Diagnostic markers (red squiggle / yellow underline overlaid from [[components/gui/diagnostic-panel]] data) land in a follow-up — the highlighter contract is already shaped for that.

## Navigation actions

- Click on a diagnostic squiggle → jumps to the matching diagnostic in the right panel.
- Click on a `[[ref]]` in spec markdown → opens the referenced spec as another tab.
- Click on a `code:` link in spec markdown → opens that code file.

## Tab title

`<filename>` with a unicode marker `●` when there's an unread diagnostic on this file. Path on hover.

## Editability across sources

Specs are the canonical source per [[decisions/0008-spec-is-canonical]], so the user editing one is the *normal* path — every spec change starts as a manual edit. Code is editable too as of the file-tree work, on the user's call: the contention with the agent's `expected_text` writes is acknowledged and explicit (see the "Code" bullet under File sources). The conservative position of v0 (code read-only) was reversed once the file tree shipped, because gating manual code fixes behind the agent forced an awkward round-trip for typo-level edits.
