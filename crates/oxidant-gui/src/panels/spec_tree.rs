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

use oxidant_spec_tools::{EdgeKind, GraphInput, Node, SpecGraph, SpecRecord, walk_specs};

use crate::app::{SelectedPreview, SharedState};
use crate::dock::{DockTab, FileSource};
use crate::panels::new_item_dialog::{NewItemDialog, NewKind};
use crate::theme;

pub struct SpecTreePanel {
    workspace_root: PathBuf,
    tree: Option<DirNode>,
    /// Spec graph for the leaf "Refs out" / "Refs in" subtrees. Built
    /// from the same `walk_specs` pass as `tree`; rebuilt together.
    graph: Option<SpecGraph>,
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
            graph: None,
            new_item: NewItemDialog::new(),
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui, state: &Arc<StdMutex<SharedState>>) {
        if self.tree.is_none() {
            self.rebuild();
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("specs").strong());
            if ui
                .small_button("⟳")
                .on_hover_text("rebuild from disk")
                .clicked()
            {
                self.rebuild();
            }
        });
        ui.separator();

        let workspace_root = self.workspace_root.clone();
        let spec_root = workspace_root.join("spec");
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if let (Some(tree), Some(graph)) = (&self.tree, &self.graph) {
                    render_node(
                        ui,
                        tree,
                        "spec",
                        &spec_root,
                        &workspace_root,
                        state,
                        &mut self.new_item,
                        graph,
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

    /// Walk `spec/` once and rebuild both the directory `tree` and the
    /// `graph` (used for the leaf Refs out / Refs in subtrees).
    fn rebuild(&mut self) {
        let records = walk_specs(&self.workspace_root);

        let mut root = DirNode::default();
        for rec in &records {
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

        let inputs: Vec<GraphInput> = records
            .into_iter()
            .map(|r| GraphInput {
                canonical_id: r.canonical_id,
                file: r.file,
                path: r.path,
            })
            .collect();

        self.tree = Some(root);
        self.graph = Some(SpecGraph::build(&inputs));
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
    graph: &SpecGraph,
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
                    graph,
                );
            }
            for rec in &node.files {
                render_leaf(ui, rec, workspace_root, state, graph);
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

/// Colour for a spec kind. Shared by the leaf name and the ref rows.
fn kind_color(kind: &str) -> Color32 {
    match kind {
        "overview" | "glossary" => Color32::LIGHT_BLUE,
        "contract" => Color32::from_rgb(255, 160, 0),
        "component" => Color32::LIGHT_GREEN,
        "tool" => Color32::from_rgb(180, 220, 255),
        "flow" => Color32::from_rgb(220, 180, 255),
        "invariant" => Color32::from_rgb(255, 200, 200),
        "decision" => Color32::from_rgb(200, 200, 200),
        _ => theme::muted_text(),
    }
}

fn render_leaf(
    ui: &mut egui::Ui,
    rec: &SpecRecord,
    workspace_root: &Path,
    state: &Arc<StdMutex<SharedState>>,
    graph: &SpecGraph,
) {
    let leaf_name = rec
        .canonical_id
        .rsplit('/')
        .next()
        .unwrap_or(&rec.canonical_id);
    let kind = rec.file.frontmatter.kind.as_str();
    let status = rec.file.frontmatter.status;
    let (leaf_color, strike) = match status {
        oxidant_spec_tools::SpecStatus::Deprecated => (theme::faint_text(), true),
        oxidant_spec_tools::SpecStatus::Draft => (Color32::from_rgb(255, 200, 100), false),
        _ => (kind_color(kind), false),
    };

    let mut job = LayoutJob::default();
    let mut leaf_fmt = TextFormat {
        color: leaf_color,
        ..Default::default()
    };
    if strike {
        leaf_fmt.strikethrough = Stroke::new(1.0, leaf_color);
    }
    job.append(leaf_name, 0.0, leaf_fmt);

    let outbound = graph.outbound(&rec.canonical_id);
    let inbound = graph.inbound(&rec.canonical_id);
    let code_files = &rec.file.frontmatter.code;
    let has_refs = !outbound.is_empty() || !inbound.is_empty() || !code_files.is_empty();

    if !has_refs {
        // No associations — render a plain leaf, no expander.
        let resp = ui.add(SelectableLabel::new(false, job));
        wire_leaf_actions(resp, rec, workspace_root, state);
        return;
    }

    let header = egui::CollapsingHeader::new(job)
        .id_salt(&rec.canonical_id)
        .show(ui, |ui| {
            let out_count = outbound.len() + code_files.len();
            egui::CollapsingHeader::new(
                RichText::new(format!("Refs out ({out_count})"))
                    .italics()
                    .color(theme::muted_text()),
            )
            .show(ui, |ui| {
                for (node, ek) in &outbound {
                    ref_row_spec(ui, node, *ek, workspace_root, state);
                }
                for code in code_files {
                    ref_row_code(ui, code, workspace_root, state);
                }
            });

            egui::CollapsingHeader::new(
                RichText::new(format!("Refs in ({})", inbound.len()))
                    .italics()
                    .color(theme::muted_text()),
            )
            .show(ui, |ui| {
                for (node, ek) in &inbound {
                    ref_row_spec(ui, node, *ek, workspace_root, state);
                }
            });
        });
    wire_leaf_actions(header.header_response, rec, workspace_root, state);
}

/// Wire the shared leaf interactions (hover, double-click to open, the
/// right-click context menu) onto a leaf's response — works for both the
/// plain `SelectableLabel` and the `CollapsingHeader` header response.
fn wire_leaf_actions(
    resp: egui::Response,
    rec: &SpecRecord,
    workspace_root: &Path,
    state: &Arc<StdMutex<SharedState>>,
) {
    let resp = resp
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{} — double-click to edit, right-click for history",
            rec.canonical_id
        ));
    if resp.clicked() {
        set_selected_preview(state, &rec.path);
    }
    if resp.double_clicked() {
        push_open(state, &rec.path, workspace_root);
    }
    let abs_path = rec.path.clone();
    let workspace_root_owned = workspace_root.to_path_buf();
    let state_for_menu = state.clone();
    let canonical_id = rec.canonical_id.clone();
    resp.context_menu(move |ui| {
        if ui.button("Open").clicked() {
            push_open(&state_for_menu, &abs_path, &workspace_root_owned);
            ui.close_menu();
        }
        if ui.button("View history").clicked() {
            push_open_history(&state_for_menu, &abs_path, &workspace_root_owned);
            ui.close_menu();
        }
        if ui.button("Open in spec graph").clicked() {
            push_open_graph(&state_for_menu, canonical_id.clone());
            ui.close_menu();
        }
    });
}

/// A clickable ref row pointing at another spec, labelled with the edge
/// kind. Double-click opens that spec.
fn ref_row_spec(
    ui: &mut egui::Ui,
    node: &Node,
    edge: EdgeKind,
    workspace_root: &Path,
    state: &Arc<StdMutex<SharedState>>,
) {
    let short = node.id.rsplit('/').next().unwrap_or(&node.id);
    let mut job = LayoutJob::default();
    job.append(
        &format!("{} → ", edge.as_str()),
        0.0,
        TextFormat {
            color: theme::faint_text(),
            ..Default::default()
        },
    );
    job.append(
        short,
        0.0,
        TextFormat {
            color: kind_color(node.kind.as_str()),
            ..Default::default()
        },
    );
    let resp = ui
        .add(SelectableLabel::new(false, job))
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{} — click to preview, double-click to open",
            node.id
        ));
    if resp.clicked() {
        set_selected_preview(state, &node.path);
    }
    if resp.double_clicked() {
        push_open(state, &node.path, workspace_root);
    }
}

/// A clickable ref row pointing at a source file declared in the spec's
/// `code:` frontmatter. Click previews; double-click opens it as a code tab.
fn ref_row_code(
    ui: &mut egui::Ui,
    code: &Path,
    workspace_root: &Path,
    state: &Arc<StdMutex<SharedState>>,
) {
    let rel = code.to_string_lossy().replace('\\', "/");
    let name = code
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| rel.clone());
    let mut job = LayoutJob::default();
    job.append(
        "code → ",
        0.0,
        TextFormat {
            color: theme::faint_text(),
            ..Default::default()
        },
    );
    job.append(
        &name,
        0.0,
        TextFormat {
            color: Color32::from_rgb(180, 220, 255),
            ..Default::default()
        },
    );
    let resp = ui
        .add(SelectableLabel::new(false, job))
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!("{rel} — click to preview, double-click to open"));
    if resp.clicked() {
        set_selected_preview(state, &workspace_root.join(code));
    }
    if resp.double_clicked() {
        push_open_code(state, code);
    }
}

/// Queue a code file (workspace-relative path) as an editable code tab.
fn push_open_code(state: &Arc<StdMutex<SharedState>>, rel_path: &Path) {
    let tab = DockTab::File {
        path: rel_path.to_path_buf(),
        source: FileSource::Code,
    };
    if let Ok(mut s) = state.lock()
        && !s.pending_centre_tabs.contains(&tab)
    {
        s.pending_centre_tabs.push(tab);
    }
}

/// Load a file's contents into `SharedState::selected_preview` and queue
/// the `Selected` tab to the front so a single click previews it
/// read-only. See spec/components/gui/dock-layout.md "Selected preview tab".
fn set_selected_preview(state: &Arc<StdMutex<SharedState>>, abs_path: &Path) {
    let text = std::fs::read_to_string(abs_path);
    if let Ok(mut s) = state.lock() {
        s.selected_preview = Some(SelectedPreview {
            path: abs_path.to_path_buf(),
            text: text.as_deref().unwrap_or("").to_string(),
            error: text.err().map(|e| e.to_string()),
        });
        let sel = DockTab::Selected;
        if !s.pending_centre_tabs.contains(&sel) {
            s.pending_centre_tabs.push(sel);
        }
    }
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

/// Queue a `DockTab::SpecGraph { seed }` for this canonical id. Re-
/// clicking the same spec focuses the existing tab (DockTab equality
/// + open_in_centre's find-and-focus). See the spec-graph-panel spec.
fn push_open_graph(state: &Arc<StdMutex<SharedState>>, canonical_id: String) {
    let tab = DockTab::SpecGraph { seed: canonical_id };
    if let Ok(mut s) = state.lock()
        && !s.pending_centre_tabs.contains(&tab)
    {
        s.pending_centre_tabs.push(tab);
    }
}
