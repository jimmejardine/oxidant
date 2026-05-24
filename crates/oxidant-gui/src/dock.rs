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
    FileTree,
    ExplorationList,
    DiagnosticPreview,
    ChatInput,
    Settings,
    File {
        path: PathBuf,
        source: FileSource,
    },
    DiffHistory {
        path: PathBuf,
        source: FileSource,
    },
    /// Per-seed force-directed graph. The `seed` is a universe node
    /// id (a spec `canonical_id` or `"code:{rel_path}"`). Each
    /// distinct seed is its own tab.
    SpecGraph {
        seed: String,
    },
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
            DockTab::FileTree => "Files".into(),
            DockTab::ExplorationList => "Explorations".into(),
            DockTab::DiagnosticPreview => "Diagnostics".into(),
            DockTab::ChatInput => "Chat".into(),
            DockTab::Settings => "Settings".into(),
            DockTab::File { path, .. } => path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string()),
            DockTab::DiffHistory { path, .. } => {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string());
                format!("⌖ {name}")
            }
            DockTab::SpecGraph { seed } => {
                // Strip the "code:" prefix for code-file seeds; keep
                // only the trailing segment so the tab strip stays
                // readable.
                let trimmed = seed.strip_prefix("code:").unwrap_or(seed);
                let short = trimmed.rsplit('/').next().unwrap_or(trimmed);
                format!("⊕ {short}")
            }
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
        vec![
            DockTab::SpecTree,
            DockTab::FileTree,
            DockTab::ExplorationList,
        ],
    );
    let [_centre, _right] = surface.split_right(
        NodeIndex::root(),
        0.78,
        vec![DockTab::DiagnosticPreview, DockTab::Settings],
    );
    let [_centre, _bottom] = surface.split_below(NodeIndex::root(), 0.75, vec![DockTab::ChatInput]);
    let _ = left;
    tree
}

/// The singletons offered in the Window menu. Order matches the spec
/// (transcript, specs, explorations, diagnostics, chat, settings). File
/// tabs are excluded — they have their own discovery flow.
pub fn singleton_tabs() -> [DockTab; 7] {
    [
        DockTab::Transcript,
        DockTab::SpecTree,
        DockTab::FileTree,
        DockTab::ExplorationList,
        DockTab::DiagnosticPreview,
        DockTab::ChatInput,
        DockTab::Settings,
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

/// Insert `tab` into the **centre** dock area, regardless of which leaf
/// currently has focus. Used for File tabs requested by panels that
/// don't own the dock (the spec-tree double-click flow): without this,
/// the new tab would land next to the spec tree on the left, since
/// double-clicking inside the spec tree leaves that leaf focused.
///
/// "Centre" = the leaf currently holding the `Transcript` tab. If
/// Transcript has been closed, falls back to any leaf already holding
/// a `File` tab, then to the focused leaf as a last resort.
///
/// If `tab` is already open, focuses it instead of duplicating.
pub fn open_in_centre(state: &mut DockState<DockTab>, tab: DockTab) {
    if let Some(loc) = state.find_tab(&tab) {
        state.set_active_tab(loc);
        state.set_focused_node_and_surface((loc.0, loc.1));
        return;
    }
    let centre = centre_node(state);
    if let Some((surface, node)) = centre {
        state.set_focused_node_and_surface((surface, node));
    }
    state.push_to_focused_leaf(tab);
}

fn centre_node(
    state: &DockState<DockTab>,
) -> Option<(egui_dock::SurfaceIndex, egui_dock::NodeIndex)> {
    if let Some((s, n, _)) = state.find_tab(&DockTab::Transcript) {
        return Some((s, n));
    }
    for ((s, n), t) in state.iter_all_tabs() {
        if matches!(t, DockTab::File { .. }) {
            return Some((s, n));
        }
    }
    None
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
    fn open_in_centre_lands_in_transcript_leaf_not_focused_leaf() {
        let mut state = default_layout();
        // Simulate the bug condition: focus the spec-tree leaf, the way
        // a click inside the spec-tree panel would.
        let (s, n, _) = state
            .find_tab(&DockTab::SpecTree)
            .expect("default layout has SpecTree");
        state.set_focused_node_and_surface((s, n));

        let file = DockTab::File {
            path: PathBuf::from("spec/overview.md"),
            source: FileSource::Spec,
        };
        open_in_centre(&mut state, file.clone());

        let (transcript_s, transcript_n, _) = state
            .find_tab(&DockTab::Transcript)
            .expect("transcript still present");
        let (file_s, file_n, _) = state.find_tab(&file).expect("file tab inserted");
        assert_eq!(
            (transcript_s, transcript_n),
            (file_s, file_n),
            "open_in_centre put the file tab in a different leaf from Transcript"
        );
    }

    #[test]
    fn open_in_centre_focuses_existing_tab_instead_of_duplicating() {
        let mut state = default_layout();
        let file = DockTab::File {
            path: PathBuf::from("spec/overview.md"),
            source: FileSource::Spec,
        };
        open_in_centre(&mut state, file.clone());
        let count_before: usize = state.iter_all_tabs().filter(|(_, t)| **t == file).count();
        assert_eq!(count_before, 1);
        open_in_centre(&mut state, file.clone());
        let count_after: usize = state.iter_all_tabs().filter(|(_, t)| **t == file).count();
        assert_eq!(count_after, 1, "open_in_centre duplicated an open tab");
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
        for tab in singleton_tabs() {
            assert!(is_tab_open(&fresh, &tab));
        }
    }

    #[test]
    fn spec_graph_tabs_with_different_seeds_are_distinct() {
        let a = DockTab::SpecGraph {
            seed: "overview".into(),
        };
        let b = DockTab::SpecGraph {
            seed: "components/gui/file-tabs".into(),
        };
        assert_ne!(a, b);
        assert_ne!(a.title(), b.title());
    }

    #[test]
    fn spec_graph_title_strips_code_prefix() {
        let t = DockTab::SpecGraph {
            seed: "code:crates/oxidant-gui/src/app.rs".into(),
        };
        assert_eq!(t.title(), "⊕ app.rs");
    }
}
