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
    let [_centre, _right] = surface.split_right(
        NodeIndex::root(),
        0.78,
        vec![DockTab::DiagnosticPreview],
    );
    let [_centre, _bottom] = surface.split_below(
        NodeIndex::root(),
        0.75,
        vec![DockTab::ChatInput],
    );
    let _ = left;
    tree
}
