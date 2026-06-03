// Realises spec/components/gui/file-tree-panel.md.
//
// Left-docked filesystem browser. Walks the workspace using the
// `ignore` crate (so .gitignore + the standard ignore files are
// honoured automatically), caches a tree, and renders it as
// CollapsingHeaders. Double-clicking a file pushes a centre-tab
// open onto SharedState::pending_centre_tabs; the host drains and
// open_in_centre's it next frame.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use egui::text::LayoutJob;
use egui::{Color32, CursorIcon, RichText, SelectableLabel, TextFormat};
use ignore::WalkBuilder;

use oxidant_spec_tools::walk_specs;

use crate::app::{SelectedPreview, SharedState};
use crate::dock::{DockTab, FileSource};
use crate::panels::new_item_dialog::{NewItemDialog, NewKind};
use crate::theme;

/// A spec that declares a given source file via its `code:` frontmatter.
/// Used for the file leaf "Refs in" subtree.
#[derive(Debug, Clone)]
struct SpecRef {
    canonical_id: String,
    kind: String,
    /// Absolute path to the spec file (for opening).
    path: PathBuf,
}

/// Files larger than this are skipped from the tree — the editor isn't
/// built for big binaries. Matches the spec.
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Per-spec excluded directory names. `ignore` already filters
/// .gitignore entries, but `target/` etc. are also excluded
/// unconditionally so a workspace without a gitignore still hides them.
const ALWAYS_SKIP_DIRS: &[&str] = &["target", ".git", "node_modules", "dist", "build"];

pub struct FileTreePanel {
    workspace_root: PathBuf,
    tree: Option<DirNode>,
    /// Reverse map: workspace-relative code path (forward slashes) → the
    /// specs whose `code:` frontmatter declares it. Backs the file leaf
    /// "Refs in" subtree. Rebuilt with the tree.
    code_to_specs: Option<HashMap<String, Vec<SpecRef>>>,
    new_item: NewItemDialog,
}

#[derive(Debug, Default)]
struct DirNode {
    dirs: BTreeMap<String, DirNode>,
    files: Vec<FileEntry>,
}

#[derive(Debug, Clone)]
struct FileEntry {
    name: String,
    /// Absolute path on disk.
    path: PathBuf,
}

impl FileTreePanel {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            tree: None,
            code_to_specs: None,
            new_item: NewItemDialog::new(),
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui, state: &Arc<StdMutex<SharedState>>) {
        if self.tree.is_none() {
            self.tree = Some(self.build_tree());
        }
        if self.code_to_specs.is_none() {
            self.code_to_specs = Some(build_code_to_specs(&self.workspace_root));
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("files").strong());
            if ui
                .small_button("⟳")
                .on_hover_text("rebuild from disk")
                .clicked()
            {
                self.tree = Some(self.build_tree());
                self.code_to_specs = Some(build_code_to_specs(&self.workspace_root));
            }
        });
        ui.separator();

        let workspace_root = self.workspace_root.clone();
        let empty_refs: HashMap<String, Vec<SpecRef>> = HashMap::new();
        let code_to_specs = self.code_to_specs.as_ref().unwrap_or(&empty_refs);
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if let Some(tree) = &self.tree {
                    render_node(
                        ui,
                        tree,
                        &workspace_label(&workspace_root),
                        &workspace_root,
                        &workspace_root,
                        true,
                        state,
                        &mut self.new_item,
                        code_to_specs,
                    );
                }
            });

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
        let walker = WalkBuilder::new(&self.workspace_root)
            .hidden(false)
            .follow_links(false)
            .filter_entry(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    && ALWAYS_SKIP_DIRS.contains(&name.as_str())
                {
                    return false;
                }
                true
            })
            .build();

        for result in walker {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };
            let file_type = match entry.file_type() {
                Some(t) => t,
                None => continue,
            };
            if !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            let rel = match path.strip_prefix(&self.workspace_root) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if rel.as_os_str().is_empty() {
                continue;
            }
            let metadata = entry.metadata().ok();
            if let Some(m) = &metadata
                && m.len() > MAX_FILE_BYTES
            {
                continue;
            }
            if looks_binary(path) {
                continue;
            }
            insert(&mut root, rel, path.to_path_buf());
        }
        sort(&mut root);
        root
    }
}

fn workspace_label(workspace_root: &Path) -> String {
    workspace_root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| workspace_root.to_string_lossy().to_string())
}

fn insert(root: &mut DirNode, rel: &Path, abs: PathBuf) {
    let mut cursor = root;
    let comps: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    if comps.is_empty() {
        return;
    }
    let (file, dirs) = comps.split_last().unwrap();
    for dir in dirs {
        cursor = cursor.dirs.entry(dir.clone()).or_default();
    }
    cursor.files.push(FileEntry {
        name: file.clone(),
        path: abs,
    });
}

fn sort(node: &mut DirNode) {
    node.files.sort_by(|a, b| a.name.cmp(&b.name));
    for v in node.dirs.values_mut() {
        sort(v);
    }
}

fn looks_binary(path: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 8192];
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return true, // unreadable → treat as not-tree-worthy
    };
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return true,
    };
    buf[..n].contains(&0)
}

/// Normalise a path to the key form used in the code→specs map:
/// workspace-relative with forward slashes (mirrors `spec_for_file`).
fn normalise_code_key(p: &str) -> String {
    p.replace('\\', "/")
}

/// Build the reverse map from each spec's `code:` frontmatter: declared
/// source file (workspace-relative, forward slashes) → declaring specs.
fn build_code_to_specs(workspace_root: &Path) -> HashMap<String, Vec<SpecRef>> {
    let mut map: HashMap<String, Vec<SpecRef>> = HashMap::new();
    for rec in walk_specs(workspace_root) {
        for code in &rec.file.frontmatter.code {
            let key = normalise_code_key(&code.to_string_lossy());
            map.entry(key).or_default().push(SpecRef {
                canonical_id: rec.canonical_id.clone(),
                kind: rec.file.frontmatter.kind.as_str().to_string(),
                path: rec.path.clone(),
            });
        }
    }
    map
}

#[allow(clippy::too_many_arguments)]
fn render_node(
    ui: &mut egui::Ui,
    node: &DirNode,
    label: &str,
    node_dir: &Path,
    workspace_root: &Path,
    default_open: bool,
    state: &Arc<StdMutex<SharedState>>,
    new_item: &mut NewItemDialog,
    code_to_specs: &HashMap<String, Vec<SpecRef>>,
) {
    let header = egui::CollapsingHeader::new(RichText::new(label).strong())
        .default_open(default_open)
        .show(ui, |ui| {
            for (name, child) in &node.dirs {
                render_node(
                    ui,
                    child,
                    name,
                    &node_dir.join(name),
                    workspace_root,
                    false,
                    state,
                    new_item,
                    code_to_specs,
                );
            }
            for file in &node.files {
                render_leaf(ui, file, workspace_root, state, code_to_specs);
            }
        });
    header.header_response.context_menu(|ui| {
        if ui.button("New file").clicked() {
            new_item.open(node_dir.to_path_buf(), NewKind::File);
            ui.close_menu();
        }
        if ui.button("New directory").clicked() {
            new_item.open(node_dir.to_path_buf(), NewKind::Directory);
            ui.close_menu();
        }
    });
}

fn render_leaf(
    ui: &mut egui::Ui,
    file: &FileEntry,
    workspace_root: &Path,
    state: &Arc<StdMutex<SharedState>>,
    code_to_specs: &HashMap<String, Vec<SpecRef>>,
) {
    let (tag, tag_color) = tag_for(&file.name);
    // Colour the filename itself by type (no [xxx] prefix); unknown
    // types stay the normal text colour.
    let name_color = if tag.is_some() {
        tag_color
    } else {
        ui.visuals().text_color()
    };
    let mut job = LayoutJob::default();
    job.append(
        &file.name,
        0.0,
        TextFormat {
            color: name_color,
            ..Default::default()
        },
    );

    // Specs that declare this file via their `code:` frontmatter.
    let key = file
        .path
        .strip_prefix(workspace_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let specs = code_to_specs.get(&key);

    match specs {
        None => {
            // No declaring spec — plain leaf, no expander.
            let resp = ui.add(SelectableLabel::new(false, job));
            wire_file_actions(resp, file, workspace_root, state);
        }
        Some(specs) => {
            let header = egui::CollapsingHeader::new(job)
                .id_salt(&file.path)
                .show(ui, |ui| {
                    egui::CollapsingHeader::new(
                        RichText::new(format!("Refs in ({})", specs.len()))
                            .italics()
                            .color(theme::muted_text()),
                    )
                    .show(ui, |ui| {
                        for sr in specs {
                            ref_row_spec(ui, sr, workspace_root, state);
                        }
                    });
                });
            wire_file_actions(header.header_response, file, workspace_root, state);
        }
    }
}

/// Wire the shared leaf interactions (hover, double-click to open, the
/// right-click context menu) onto a file leaf's response.
fn wire_file_actions(
    resp: egui::Response,
    file: &FileEntry,
    workspace_root: &Path,
    state: &Arc<StdMutex<SharedState>>,
) {
    let resp = resp
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{} — double-click to open, right-click for history",
            file.path
                .strip_prefix(workspace_root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| file.path.to_string_lossy().to_string())
        ));
    if resp.clicked() {
        set_selected_preview(state, &file.path);
    }
    if resp.double_clicked() {
        push_open(state, &file.path, workspace_root);
    }
    let abs_path = file.path.clone();
    let workspace_root_owned = workspace_root.to_path_buf();
    let state_for_menu = state.clone();
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
            push_open_graph(&state_for_menu, &abs_path, &workspace_root_owned);
            ui.close_menu();
        }
    });
}

/// A clickable ref row pointing at a spec that declares this file.
/// Click previews; double-click opens the spec (sourced as Spec by `push_open`).
fn ref_row_spec(
    ui: &mut egui::Ui,
    sr: &SpecRef,
    workspace_root: &Path,
    state: &Arc<StdMutex<SharedState>>,
) {
    let short = sr
        .canonical_id
        .rsplit('/')
        .next()
        .unwrap_or(&sr.canonical_id);
    let mut job = LayoutJob::default();
    job.append(
        short,
        0.0,
        TextFormat {
            color: spec_kind_color(&sr.kind),
            ..Default::default()
        },
    );
    let resp = ui
        .add(SelectableLabel::new(false, job))
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{} — click to preview, double-click to open",
            sr.canonical_id
        ));
    if resp.clicked() {
        set_selected_preview(state, &sr.path);
    }
    if resp.double_clicked() {
        push_open(state, &sr.path, workspace_root);
    }
}

/// Spec-kind colour, matching the spec tree's leaf colouring.
fn spec_kind_color(kind: &str) -> Color32 {
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

fn tag_for(name: &str) -> (Option<&'static str>, Color32) {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".rs") {
        (Some("rs"), Color32::from_rgb(180, 220, 255))
    } else if lower.ends_with(".md") {
        (Some("md"), Color32::from_rgb(255, 160, 0))
    } else if lower.ends_with(".toml") {
        (Some("toml"), theme::muted_text())
    } else if lower.ends_with(".json") || lower.ends_with(".yml") || lower.ends_with(".yaml") {
        (Some("data"), theme::faint_text())
    } else {
        (None, theme::faint_text())
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

fn push_open(state: &Arc<StdMutex<SharedState>>, abs_path: &Path, workspace_root: &Path) {
    let path = abs_path
        .strip_prefix(workspace_root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| abs_path.to_path_buf());
    let source = source_for(abs_path, workspace_root);
    let tab = DockTab::File { path, source };
    if let Ok(mut s) = state.lock()
        && !s.pending_centre_tabs.contains(&tab)
    {
        s.pending_centre_tabs.push(tab);
    }
}

/// Like `push_open` but queues a read-only `DiffHistory` tab. The source
/// flavour drives the syntect syntax used inside the panel — `.md` under
/// `spec/` gets markdown highlighting, everything else gets matched by
/// extension. See [[flows/view-spec-history]].
fn push_open_history(state: &Arc<StdMutex<SharedState>>, abs_path: &Path, workspace_root: &Path) {
    let path = abs_path
        .strip_prefix(workspace_root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| abs_path.to_path_buf());
    let source = source_for(abs_path, workspace_root);
    let tab = DockTab::DiffHistory { path, source };
    if let Ok(mut s) = state.lock()
        && !s.pending_centre_tabs.contains(&tab)
    {
        s.pending_centre_tabs.push(tab);
    }
}

/// Queue a `DockTab::SpecGraph { seed }` for this file. The seed id
/// uses the `code:{rel_path}` format the spec-graph universe builder
/// emits for code-file nodes. If no spec claims this file via `code:`,
/// the graph panel renders an empty-state message — we don't try to
/// hide the menu item from here (we'd have to walk every spec's
/// frontmatter, defeating the lazy-build of the universe).
fn push_open_graph(state: &Arc<StdMutex<SharedState>>, abs_path: &Path, workspace_root: &Path) {
    let rel = abs_path
        .strip_prefix(workspace_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| abs_path.to_string_lossy().to_string());
    let seed = format!("code:{rel}");
    let tab = DockTab::SpecGraph { seed };
    if let Ok(mut s) = state.lock()
        && !s.pending_centre_tabs.contains(&tab)
    {
        s.pending_centre_tabs.push(tab);
    }
}

/// `.md` files under `spec/` are treated as specs (matches the
/// spec-tree double-click flow). Everything else opens as Code.
fn source_for(abs_path: &Path, workspace_root: &Path) -> FileSource {
    let rel = abs_path.strip_prefix(workspace_root).unwrap_or(abs_path);
    let is_md = abs_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md"))
        .unwrap_or(false);
    let in_spec = rel
        .components()
        .next()
        .and_then(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .map(|c| c == "spec")
        .unwrap_or(false);
    if is_md && in_spec {
        FileSource::Spec
    } else {
        FileSource::Code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_for_routes_spec_markdown_as_spec() {
        let root = Path::new("/work/repo");
        assert_eq!(
            source_for(&root.join("spec").join("overview.md"), root),
            FileSource::Spec
        );
    }

    #[test]
    fn source_for_routes_rust_as_code() {
        let root = Path::new("/work/repo");
        assert_eq!(
            source_for(
                &root
                    .join("crates")
                    .join("oxidant-gui")
                    .join("src")
                    .join("app.rs"),
                root
            ),
            FileSource::Code
        );
    }

    #[test]
    fn source_for_routes_non_spec_markdown_as_code() {
        // README.md at the repo root is code, not a spec.
        let root = Path::new("/work/repo");
        assert_eq!(source_for(&root.join("README.md"), root), FileSource::Code);
    }

    #[test]
    fn tag_for_known_extensions() {
        assert_eq!(tag_for("foo.rs").0, Some("rs"));
        assert_eq!(tag_for("README.md").0, Some("md"));
        assert_eq!(tag_for("Cargo.toml").0, Some("toml"));
        assert_eq!(tag_for("settings.json").0, Some("data"));
    }

    #[test]
    fn tag_for_unknown_returns_none() {
        assert_eq!(tag_for("Makefile").0, None);
        assert_eq!(tag_for("LICENSE").0, None);
    }

    #[test]
    fn normalise_code_key_uses_forward_slashes() {
        assert_eq!(
            normalise_code_key("crates\\oxidant-gui\\src\\app.rs"),
            "crates/oxidant-gui/src/app.rs"
        );
        assert_eq!(
            normalise_code_key("crates/oxidant-gui/src/app.rs"),
            "crates/oxidant-gui/src/app.rs"
        );
    }
}
