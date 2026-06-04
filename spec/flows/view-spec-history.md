```yaml
id: view-spec-history
kind: flow
parent: overview
order: 13
status: active
responsibility: |
  Right-click a spec (or any file) → "View history" → a read-only side-by-side diff opens in the centre dock leaf. The user picks two commits (or "Working tree") from dropdowns and compares.
depends_on:
  - components/gui/diff-history-panel
  - components/gui/spec-tree-panel
  - components/gui/file-tree-panel
  - components/gui/dock-layout
  - components/vcs/git-shellout
```

# View the git history of a spec or file

The "what did this look like before?" affordance. Cousin to [[flows/open-file-from-tree]] — that flow ends in an editable file tab; this one ends in a read-only diff viewer.

## Trigger

A right-click on a leaf node inside either tree panel — `[[components/gui/spec-tree-panel]]` or `[[components/gui/file-tree-panel]]` — and selecting "View history" from the context menu. The same menu also carries the existing New file / New folder items; "View history" appears below them.

## Steps

1. **Resolve the absolute path.** The tree's right-click handler joins the leaf's relative path with `workspace_root` and canonicalises (via `dunce::canonicalize`).

2. **Queue the dock tab.** Push `DockTab::DiffHistory { path, source }` onto `SharedState.pending_centre_tabs` — same mechanism the file-open flow uses. Source is `FileSource::Spec` for paths under `spec/`, otherwise `FileSource::Code`; the renderer uses it to pick the right syntect syntax even though the file is read-only.

3. **Drain after `DockArea::show`.** The host viewport's per-frame update calls `open_in_centre` on the queued tab — same code path as [[flows/open-file-from-tree]]. Multiple DiffHistory tabs may coexist (one per file), and reopening the same path focuses the existing tab rather than duplicating.

4. **Panel first paint.** [[components/gui/diff-history-panel]] takes over:
   - Queries `Git::log(LogOpts { path: Some(...), limit: Some(200), .. })` to populate the dropdown list.
   - Selects the parent of the newest commit on the left and `Working tree` on the right.
   - Loads both versions via `Git::show_file(sha, &path)` (or reads from disk for `Working tree`).
   - Computes the diff with `similar::TextDiff::from_lines`.
   - Renders two columns side-by-side.

5. **User picks commits.** Changing either dropdown invalidates that side's text cache, re-fetches via `Git::show_file`, and re-runs the diff. The swap button (`⇄`) exchanges left/right selections in one click — useful for flipping a "before vs after" view into "after vs before".

6. **Scrolling.** Both columns share one vertical scroll offset so corresponding diff lines stay aligned as the user scrolls.

7. **Refresh.** A refresh button re-queries `Git::log` for the file, picking up new commits made since the tab opened.

## Why centre placement is special

Same reason as [[flows/open-file-from-tree]] — the tree panels live in the left dock leaf, so the right-click handler runs inside that leaf's focus. `egui_dock::push_to_focused_leaf` would put the new DiffHistory tab next to the tree, which is the wrong place. The `open_in_centre` helper finds the centre leaf (the one holding `Transcript`) and inserts there.

## Edge cases

- **File didn't exist at the selected commit.** `Git::show_file` returns `FileNotAtRevision`; the affected column shows "file not present at this commit". The diff against the empty string still renders meaningfully (every line on the other side appears as an Insert).
- **File renamed mid-history.** `Git::log` is invoked without `--follow`, so commits where the file lived at a different path are not listed. Known limitation; the dropdown only shows commits that touched the *current* path.
- **Very large file.** No special handling beyond what `Git::show_file` returns; the syntect highlighter handles large files acceptably for read-only display. Tree panels still exclude files above 5 MiB from being right-clickable in the first place.
- **No commits touch this file.** Brand-new spec that's never been committed. Dropdown shows only `Working tree`; the other slot defaults to "empty" and the whole file renders as Inserts.

## See also

- [[components/gui/diff-history-panel]] — the panel implementation
- [[flows/open-file-from-tree]] — the editable-tab cousin
- [[components/vcs/git-shellout]] — `Git::log`, `Git::show_file`
