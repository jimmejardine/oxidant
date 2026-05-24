---
id: diff-history-panel
kind: component
parent: overview
order: 11
implements: []
depends_on:
  - components/gui/dock-layout
  - components/gui/spec-tree-panel
  - components/gui/file-tree-panel
  - components/vcs/git-shellout
code:
  - crates/oxidant-gui/src/panels/diff_history.rs
status: active
responsibility: |
  Read-only side-by-side diff viewer for one file. Two columns, each with a commit-picker dropdown that lists every commit that touched the file plus a virtual "Working tree" entry. Body of each column shows the file at the selected version with line-level diff overlay.
---

The history-viewing surface. Opened from the spec tree or file tree right-click menu via "View history"; lives as a `DockTab::DiffHistory { path, source }` in the centre dock leaf. Distinct from [[components/gui/file-tabs]] — that one is the *editable* view of a file's current contents; this one is the *read-only* view of how the file changed over time.

## Layout

```
┌────────────────────────────────────────────────────────────────┐
│  [HEAD~3 · 2026-05-22 · "edit cli"  ▼] ⇄  [Working tree   ▼] │
├──────────────────────────────────┬─────────────────────────────┤
│  (file at HEAD~3, syntect-       │  (file at Working tree,     │
│  highlighted; removed lines      │  syntect-highlighted; added │
│  shown with a red background     │  lines shown with a green   │
│  band the width of the column)   │  background band)           │
│                                  │                             │
│  (vertically scrollable;         │  (scroll synced with the    │
│  left+right scroll together)     │  left column)               │
└──────────────────────────────────┴─────────────────────────────┘
```

Both columns share one vertical scroll offset so corresponding lines stay aligned. A `⇄` button between the two dropdowns swaps left/right.

## Dropdown shape

Each entry rendered as `<short-sha> · <iso-date> · <subject>` with the short SHA truncated to 7 chars. The first item in every dropdown is always the virtual entry `Working tree` (the file's current on-disk contents, including unsaved edits if the user has the file open in a [[components/gui/file-tabs]] tab — read freshly from disk each refresh, **not** from the editor buffer, to keep the model simple).

Defaults on first open:
- **Left**: the *parent* of the most recent commit that touched the file (i.e. "what the file looked like before the last change"). If only one commit exists, the parent is the empty tree and the column renders "file not present at this commit".
- **Right**: `Working tree`. This makes the most useful "what's changed lately" view the default.

## Diff overlay

`similar::TextDiff::from_lines(&left_text, &right_text)` produces a sequence of `ChangeTag::{Equal, Delete, Insert}`. Map to per-line backgrounds:

- `Delete` lines paint a band at `theme::muted_text()`-aware red; appear on the **left** only (the right's slot for those lines is blank).
- `Insert` lines paint a green band; appear on the **right** only.
- `Equal` lines paint no band on either side and align on the same screen row.

Foreground text uses `crate::highlighter::highlight(&self.path, &full_text)` for both columns so syntax highlighting matches what the file-tabs view shows. The diff colours layer underneath the highlighted glyphs.

## Caching

Three layers, all keyed inside the `DiffHistoryPanel` struct:

1. `commits: Vec<Commit>` — populated once on first paint via `Git::log(LogOpts { path: Some(self.path.clone()), limit: Some(200), .. })`. Refresh button re-queries.
2. `(left_text, right_text): (String, String)` — keyed by `(CommitChoice, mtime_when_loaded)`; invalidated when either dropdown changes or (for `WorkingTree`) when the file's mtime changes.
3. `diff_lines: Vec<DiffLine>` — recomputed whenever either side's text changes.

## Read-only by design

No edit buffer, no save, no Ctrl+S. The user goes back to a [[components/gui/file-tabs]] tab to make changes; the diff view is a research tool. Out of scope: word-level intra-line highlighting, three-way merge, rename following (`git log --follow`), and persisting open DiffHistory tabs across launches.

## Failure modes

- **File didn't exist at the selected commit.** `Git::show_file` returns `FileNotAtRevision`; column renders a centered "file not present at this commit" placeholder. Diff is computed against an empty string on that side.
- **Path renamed in history.** `Git::log` without `--follow` won't surface commits where the file lived under a different name. Documented limit; revisit when a real spec moves.
- **Very long history.** Capped at 200 commits per the `LogOpts.limit`; older history requires a CLI escape.

## See also

- [[flows/view-spec-history]] — end-to-end narrative from right-click to rendered diff
- [[components/vcs/git-shellout]] — `Git::log`, `Git::show_file`, `Commit`
- [[components/gui/file-tabs]] — the editable counterpart
