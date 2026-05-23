// Realises spec/components/gui/spec-tree-panel.md.
//
// Left-docked hierarchical view of spec/. MVP: walks the spec tree on
// init and caches the result. Live updates via notify watcher are
// deferred. Drag-and-drop, validate-warning badges, and recent-change
// dots will plug in once the SQLite index + watcher land in the GUI.

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::{Color32, RichText};

use oxidant_spec_tools::{SpecRecord, walk_specs};

use crate::theme;

pub struct SpecTreePanel {
    workspace_root: PathBuf,
    tree: Option<DirNode>,
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
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
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

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if let Some(tree) = &self.tree {
                    render_node(ui, tree, "spec");
                }
            });
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

fn render_node(ui: &mut egui::Ui, node: &DirNode, label: &str) {
    egui::CollapsingHeader::new(RichText::new(label).strong())
        .default_open(label == "spec")
        .show(ui, |ui| {
            for (name, child) in &node.dirs {
                render_node(ui, child, name);
            }
            for rec in &node.files {
                render_leaf(ui, rec);
            }
        });
}

fn render_leaf(ui: &mut egui::Ui, rec: &SpecRecord) {
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
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("[{kind}]")).color(kind_color));
        let text = RichText::new(leaf_name);
        let text = if matches!(status, oxidant_spec_tools::SpecStatus::Deprecated) {
            text.color(theme::faint_text()).strikethrough()
        } else if matches!(status, oxidant_spec_tools::SpecStatus::Draft) {
            text.color(Color32::from_rgb(255, 200, 100))
        } else {
            text
        };
        ui.label(text).on_hover_text(&rec.canonical_id);
    });
}
