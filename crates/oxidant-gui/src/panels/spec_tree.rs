// Realises spec/components/gui/spec-tree-panel.md.
//
// Left-docked hierarchical view of spec/. MVP: walks the spec tree on
// init and caches the result. Live updates via notify watcher are
// deferred. Drag-and-drop, validate-warning badges, and recent-change
// dots will plug in once the SQLite index + watcher land in the GUI.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use egui::text::LayoutJob;
use egui::{Color32, CursorIcon, RichText, SelectableLabel, Stroke, TextFormat};

use oxidant_spec_tools::{SpecRecord, walk_specs};

use crate::app::SharedState;
use crate::dock::{DockTab, FileSource};
use crate::panels::new_item_dialog::{NewItemDialog, NewKind};
use crate::theme;

pub struct SpecTreePanel {
    workspace_root: PathBuf,
    tree: Option<DirNode>,
    new_item: NewItemDialog,
}

#[derive(Debug, Default)]
struct DirNode {
    files: Vec<SpecRecord>,
    dirs: BTreeMap<String, DirNode>,
}

impl SpecTreePanel {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            tree: None,
            new_item: NewItemDialog::new(),
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui, state: &Arc<StdMutex<SharedState>>) {
        if self.tree.is_none() {
            self.tree = Some(self.build_tree());
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("specs").strong());
            if ui
                .small_button("⟳")
                .on_hover_text("rebuild from disk")
                .clicked()
            {
                self.tree = Some(self.build_tree());
            }
        });
        ui.separator();

        let workspace_root = self.workspace_root.clone();
        let spec_root = workspace_root.join("spec");
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if let Some(tree) = &self.tree {
                    render_node(
                        ui,
                        tree,
                        "spec",
                        &spec_root,
                        &workspace_root,
                        state,
                        &mut self.new_item,
                    );
                }
            });

        // Modal dialog above everything; render every frame, no-op when
        // closed.
        let outcome = self.new_item.render(ui.ctx());
        if let Some(created) = outcome.created_file {
            self.tree = None;
            push_open(state, &created, &workspace_root);
        } else if outcome.created_directory {
            self.tree = None;
        }
    }

    fn build_tree(&self) -> DirNode {
        let mut root = DirNode::default();
        for rec in walk_specs(&self.workspace_root) {
            let mut cursor = &mut root;
            let segments: Vec<&str> = rec.canonical_id.split('/').collect();
            let last = segments.len() - 1;
            for (i, seg) in segments.iter().enumerate() {
                if i == last {
                    cursor.files.push(rec.clone());
                    break;
                } else {
                    cursor = cursor.dirs.entry((*seg).to_string()).or_default();
                }
            }
        }
        // Sort files within each node by frontmatter order then alphabetical.
        sort_dir(&mut root);
        root
    }
}

fn sort_dir(node: &mut DirNode) {
    node.files.sort_by(|a, b| {
        let ao = a.file.frontmatter.order;
        let bo = b.file.frontmatter.order;
        match (ao, bo) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.canonical_id.cmp(&b.canonical_id),
        }
    });
    for v in node.dirs.values_mut() {
        sort_dir(v);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_node(
    ui: &mut egui::Ui,
    node: &DirNode,
    label: &str,
    node_dir: &Path,
    workspace_root: &Path,
    state: &Arc<StdMutex<SharedState>>,
    new_item: &mut NewItemDialog,
) {
    let header = egui::CollapsingHeader::new(RichText::new(label).strong())
        .default_open(label == "spec")
        .show(ui, |ui| {
            for (name, child) in &node.dirs {
                render_node(
                    ui,
                    child,
                    name,
                    &node_dir.join(name),
                    workspace_root,
                    state,
                    new_item,
                );
            }
            for rec in &node.files {
                render_leaf(ui, rec, workspace_root, state);
            }
        });
    header.header_response.context_menu(|ui| {
        if ui.button("New spec").clicked() {
            new_item.open(node_dir.to_path_buf(), NewKind::File);
            ui.close_menu();
        }
        if ui.button("New folder").clicked() {
            new_item.open(node_dir.to_path_buf(), NewKind::Directory);
            ui.close_menu();
        }
    });
}

fn render_leaf(
    ui: &mut egui::Ui,
    rec: &SpecRecord,
    workspace_root: &Path,
    state: &Arc<StdMutex<SharedState>>,
) {
    let leaf_name = rec
        .canonical_id
        .rsplit('/')
        .next()
        .unwrap_or(&rec.canonical_id);
    let kind = rec.file.frontmatter.kind.as_str();
    let status = rec.file.frontmatter.status;
    let kind_color = match kind {
        "overview" | "glossary" => Color32::LIGHT_BLUE,
        "contract" => Color32::from_rgb(255, 160, 0),
        "component" => Color32::LIGHT_GREEN,
        "tool" => Color32::from_rgb(180, 220, 255),
        "flow" => Color32::from_rgb(220, 180, 255),
        "invariant" => Color32::from_rgb(255, 200, 200),
        "decision" => Color32::from_rgb(200, 200, 200),
        _ => theme::muted_text(),
    };
    let (leaf_color, strike) = match status {
        oxidant_spec_tools::SpecStatus::Deprecated => (theme::faint_text(), true),
        oxidant_spec_tools::SpecStatus::Draft => (Color32::from_rgb(255, 200, 100), false),
        _ => (ui.visuals().text_color(), false),
    };

    let mut job = LayoutJob::default();
    job.append(
        &format!("[{kind}] "),
        0.0,
        TextFormat {
            color: kind_color,
            ..Default::default()
        },
    );
    let mut leaf_fmt = TextFormat {
        color: leaf_color,
        ..Default::default()
    };
    if strike {
        leaf_fmt.strikethrough = Stroke::new(1.0, leaf_color);
    }
    job.append(leaf_name, 0.0, leaf_fmt);

    let resp = ui
        .add(SelectableLabel::new(false, job))
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{} — double-click to edit, right-click for history",
            rec.canonical_id
        ));
    if resp.double_clicked() {
        push_open(state, &rec.path, workspace_root);
    }
    let abs_path = rec.path.clone();
    let workspace_root_owned = workspace_root.to_path_buf();
    let state_for_menu = state.clone();
    resp.context_menu(move |ui| {
        if ui.button("View history").clicked() {
            push_open_history(&state_for_menu, &abs_path, &workspace_root_owned);
            ui.close_menu();
        }
    });
}

/// Push a `DockTab::File { source: Spec }` onto the shared state's
/// `pending_centre_tabs` queue. The App drains the queue after
/// `DockArea::show` and inserts the tab into the dock — we can't
/// touch the dock from here because we're inside the dock's render.
fn push_open(state: &Arc<StdMutex<SharedState>>, abs_path: &Path, workspace_root: &Path) {
    let path = abs_path
        .strip_prefix(workspace_root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| abs_path.to_path_buf());
    let tab = DockTab::File {
        path,
        source: FileSource::Spec,
    };
    if let Ok(mut s) = state.lock()
        && !s.pending_centre_tabs.contains(&tab)
    {
        s.pending_centre_tabs.push(tab);
    }
}

/// Like `push_open` but queues a read-only `DiffHistory` tab instead of
/// the editable `File` tab. See [[flows/view-spec-history]].
fn push_open_history(state: &Arc<StdMutex<SharedState>>, abs_path: &Path, workspace_root: &Path) {
    let path = abs_path
        .strip_prefix(workspace_root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| abs_path.to_path_buf());
    let tab = DockTab::DiffHistory {
        path,
        source: FileSource::Spec,
    };
    if let Ok(mut s) = state.lock()
        && !s.pending_centre_tabs.contains(&tab)
    {
        s.pending_centre_tabs.push(tab);
    }
}
