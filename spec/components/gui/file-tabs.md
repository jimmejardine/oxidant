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
  Render an opened code or spec file as a centre-area dock tab with syntax highlighting and live diagnostic markers.
---

Opened files dock as siblings of the [[components/gui/transcript-tab]] in the centre tab group.

## File sources

- **Code**: `crates/**/*.rs`, `Cargo.toml`, etc. Read from disk; live-reloaded on `notify` events. Read-only in MVP; edit happens via the agent (`apply_edits` / `edit_string`).
- **Spec**: `spec/**/*.md`. **Editable** — opened via double-click from the spec tree. Rendered as a raw multi-line text editor with a Save button; the markdown preview toggle lands later. Edits are flushed to disk only when the user presses Save. While unsaved, the tab title carries a `●` marker.

### Edit lifecycle for specs

1. Tab opens → contents are loaded from disk into `SharedState::editor_buffers[path]` (a `HashMap<PathBuf, EditorBuffer>` keyed by absolute path).
2. The text edit binds to the buffer's `text` field. Mutations flip `dirty = true` and update `mtime_at_load` is NOT touched.
3. Save button is enabled iff `dirty`. Click writes the buffer to disk; on success, clears `dirty`. On error, surfaces the error inline in red and leaves `dirty` set.
4. If the on-disk mtime advances while a tab is dirty (the agent edited the file out from under us), a banner offers Reload / Discard. Conflict resolution beyond that is out of scope for the MVP.

The buffer survives a tab close+reopen — the user can dock-close a spec tab without losing unsaved changes until they explicitly Discard. (This is opinionated; the alternative — prompting on close — is more ceremonial than it's worth for a single-user editor.)

## Render

- Code: `syntect`-tokenised via the `egui_syntax_highlight` (or in-house) helper. Line numbers in a gutter. Diagnostic markers (red squiggle / yellow underline) overlaid from [[components/gui/diagnostic-panel]] data.
- Spec markdown: rendered via `egui_commonmark`. A toggle switches to raw view for cases where the user wants to read the source.

## Navigation actions

- Click on a diagnostic squiggle → jumps to the matching diagnostic in the right panel.
- Click on a `[[ref]]` in spec markdown → opens the referenced spec as another tab.
- Click on a `code:` link in spec markdown → opens that code file.

## Tab title

`<filename>` with a unicode marker `●` when there's an unread diagnostic on this file. Path on hover.

## Why specs are editable but code isn't

Specs are the canonical source per [[decisions/0008-spec-is-canonical]], so the user editing one is the *normal* path — every spec change starts as a manual edit. Letting the user hand-edit code while the agent is also editing it, by contrast, creates contention with `expected_text` checks and obscures who changed what. v2 may add a "manual edit mode" for code with a soft lock on the agent.
