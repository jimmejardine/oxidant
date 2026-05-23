// Realises spec/components/gui/dock-layout.md.
//
// DockTab enum + default layout. The host viewport owns the
// DockState<DockTab>; render delegates to the per-panel modules.
// Persistence to <worktree>/.oxidant/dock-layout.json is deferred —
// the file path is reserved per spec, but reading/writing it lands
// when settings + notify are wired through.

use std::path::PathBuf;

use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockTab {
    Transcript,
    SpecTree,
    ExplorationList,
    DiagnosticPreview,
    ChatInput,
    File { path: PathBuf, source: FileSource },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileSource {
    Code,
    Spec,
}

impl DockTab {
    pub fn title(&self) -> String {
        match self {
            DockTab::Transcript => "Transcript".into(),
            DockTab::SpecTree => "Specs".into(),
            DockTab::ExplorationList => "Explorations".into(),
            DockTab::DiagnosticPreview => "Diagnostics".into(),
            DockTab::ChatInput => "Chat".into(),
            DockTab::File { path, .. } => path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string()),
        }
    }
}

/// Build the default dock layout per the spec:
///   LEFT:   spec_tree, exploration_list (tab group)
///   CENTRE: transcript + future opened files
///   RIGHT:  diagnostic_preview
///   BOTTOM: chat_input
pub fn default_layout() -> DockState<DockTab> {
    let mut tree: DockState<DockTab> = DockState::new(vec![DockTab::Transcript]);
    let root = NodeIndex::root();
    let surface = tree.main_surface_mut();
    let [_centre, left] = surface.split_left(
        root,
        0.22,
        vec![DockTab::SpecTree, DockTab::ExplorationList],
    );
    let [_centre, _right] =
        surface.split_right(NodeIndex::root(), 0.78, vec![DockTab::DiagnosticPreview]);
    let [_centre, _bottom] = surface.split_below(NodeIndex::root(), 0.75, vec![DockTab::ChatInput]);
    let _ = left;
    tree
}

/// The singletons offered in the Window menu. Order matches the spec
/// (transcript, specs, explorations, diagnostics, chat). File tabs are
/// excluded — they have their own discovery flow.
pub fn singleton_tabs() -> [DockTab; 5] {
    [
        DockTab::Transcript,
        DockTab::SpecTree,
        DockTab::ExplorationList,
        DockTab::DiagnosticPreview,
        DockTab::ChatInput,
    ]
}

/// True when the given singleton tab is currently present somewhere in
/// `state`. Backed by egui_dock's `find_tab`, which walks every surface.
pub fn is_tab_open(state: &DockState<DockTab>, tab: &DockTab) -> bool {
    state.find_tab(tab).is_some()
}

/// Insert `tab` into the focused leaf (or the first leaf if none has
/// focus). No-op if the tab is already open — the menu drives this and
/// also disables the entry in that case, but defending against a
/// keyboard-shortcut double-fire is cheap.
pub fn open_tab(state: &mut DockState<DockTab>, tab: DockTab) {
    if is_tab_open(state, &tab) {
        return;
    }
    state.push_to_focused_leaf(tab);
}

/// Rebuild the default layout, but carry over any opened file tabs as
/// centre-tab children of the new tree so the user doesn't lose them.
pub fn reset_layout_preserving_files(state: &DockState<DockTab>) -> DockState<DockTab> {
    let files: Vec<DockTab> = state
        .iter_all_tabs()
        .filter_map(|(_, tab)| match tab {
            DockTab::File { .. } => Some(tab.clone()),
            _ => None,
        })
        .collect();
    let mut fresh = default_layout();
    for f in files {
        fresh.push_to_focused_leaf(f);
    }
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_has_every_singleton() {
        let state = default_layout();
        for tab in singleton_tabs() {
            assert!(
                is_tab_open(&state, &tab),
                "default layout missing singleton {:?}",
                tab
            );
        }
    }

    #[test]
    fn open_tab_inserts_a_closed_singleton() {
        let mut state = default_layout();
        // Remove the chat tab to simulate the user closing it.
        if let Some((surface, node, index)) = state.find_tab(&DockTab::ChatInput) {
            state.remove_tab((surface, node, index));
        }
        assert!(!is_tab_open(&state, &DockTab::ChatInput));
        open_tab(&mut state, DockTab::ChatInput);
        assert!(is_tab_open(&state, &DockTab::ChatInput));
    }

    #[test]
    fn open_tab_is_noop_when_already_present() {
        let mut state = default_layout();
        let before: usize = state.iter_all_tabs().count();
        open_tab(&mut state, DockTab::Transcript);
        let after: usize = state.iter_all_tabs().count();
        assert_eq!(before, after, "open_tab duplicated an already-open tab");
    }

    #[test]
    fn reset_preserves_file_tabs() {
        let mut state = default_layout();
        let file = DockTab::File {
            path: PathBuf::from("README.md"),
            source: FileSource::Code,
        };
        state.push_to_focused_leaf(file.clone());
        assert!(is_tab_open(&state, &file));

        let fresh = reset_layout_preserving_files(&state);
        assert!(
            is_tab_open(&fresh, &file),
            "file tab dropped on reset_layout"
        );
        // Singletons are back too.
        for tab in singleton_tabs() {
            assert!(is_tab_open(&fresh, &tab));
        }
    }
}
