// Realises spec/components/gui/spec-graph-panel.md.
//
// Force-directed visualiser of the spec graph + the code files and
// tests that realise each spec. Populates progressively: starts with
// a single seed node (`overview`) and grows when the user clicks the
// per-node `+S` / `+C` / `+T` expand chips. Refcounting keeps shared
// neighbours alive when one of multiple expansion paths is collapsed.
//
// All rendering goes through egui::Painter directly — no third-party
// graph crate. The force simulation lives in crate::graph_layout.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use oxidant_spec_tools::{
    EdgeKind as SpecEdgeKind, GraphInput, SpecGraph, SpecKind, TestRef, walk_specs,
};

use crate::app::SharedState;
use crate::dock::{DockTab, FileSource};
use crate::graph_layout::{EdgeKindForce, LayoutEdge, LayoutNode, LayoutParams, step};
use crate::theme;

// ---------------------------------------------------------------- types

type NodeId = String;

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeKindUi {
    SpecOverview,
    SpecGlossary,
    SpecComponent,
    SpecContract,
    SpecTool,
    SpecFlow,
    SpecInvariant,
    SpecDecision,
    CodeFile,
    Test,
}

impl NodeKindUi {
    fn from_spec_kind(k: SpecKind) -> Self {
        match k {
            SpecKind::Overview => NodeKindUi::SpecOverview,
            SpecKind::Glossary => NodeKindUi::SpecGlossary,
            SpecKind::Component => NodeKindUi::SpecComponent,
            SpecKind::Contract => NodeKindUi::SpecContract,
            SpecKind::Tool => NodeKindUi::SpecTool,
            SpecKind::Flow => NodeKindUi::SpecFlow,
            SpecKind::Invariant => NodeKindUi::SpecInvariant,
            SpecKind::Decision => NodeKindUi::SpecDecision,
        }
    }

    fn colour(self) -> Color32 {
        // Mirrors spec-tree-panel's kind_color match for consistency.
        match self {
            NodeKindUi::SpecOverview | NodeKindUi::SpecGlossary => Color32::LIGHT_BLUE,
            NodeKindUi::SpecContract => Color32::from_rgb(255, 160, 0),
            NodeKindUi::SpecComponent => Color32::LIGHT_GREEN,
            NodeKindUi::SpecTool => Color32::from_rgb(180, 220, 255),
            NodeKindUi::SpecFlow => Color32::from_rgb(220, 180, 255),
            NodeKindUi::SpecInvariant => Color32::from_rgb(255, 200, 200),
            NodeKindUi::SpecDecision => Color32::from_rgb(200, 200, 200),
            NodeKindUi::CodeFile => Color32::from_rgb(120, 200, 240),
            NodeKindUi::Test => Color32::from_rgb(160, 220, 160),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum EdgeKindUi {
    Parent,
    Implements,
    DependsOn,
    BodyRef,
    RealisedBy,
    Tests,
}

impl EdgeKindUi {
    fn from_spec_edge(k: SpecEdgeKind) -> Self {
        match k {
            SpecEdgeKind::Parent => EdgeKindUi::Parent,
            SpecEdgeKind::Implements => EdgeKindUi::Implements,
            SpecEdgeKind::DependsOn => EdgeKindUi::DependsOn,
            SpecEdgeKind::BodyRef => EdgeKindUi::BodyRef,
        }
    }

    fn to_force(self) -> EdgeKindForce {
        match self {
            EdgeKindUi::Parent => EdgeKindForce::Parent,
            EdgeKindUi::Implements => EdgeKindForce::Implements,
            EdgeKindUi::DependsOn => EdgeKindForce::DependsOn,
            EdgeKindUi::BodyRef => EdgeKindForce::BodyRef,
            EdgeKindUi::RealisedBy => EdgeKindForce::RealisedBy,
            EdgeKindUi::Tests => EdgeKindForce::Tests,
        }
    }

    fn stroke(self) -> Stroke {
        match self {
            EdgeKindUi::Parent => Stroke::new(2.0, Color32::from_rgb(100, 160, 240)),
            EdgeKindUi::Implements => Stroke::new(2.0, Color32::from_rgb(120, 220, 140)),
            EdgeKindUi::DependsOn => Stroke::new(1.5, Color32::from_rgb(240, 170, 90)),
            EdgeKindUi::BodyRef => Stroke::new(1.0, theme::faint_text()),
            EdgeKindUi::RealisedBy => Stroke::new(1.5, Color32::from_rgb(120, 200, 240)),
            EdgeKindUi::Tests => Stroke::new(1.5, Color32::from_rgb(160, 220, 160)),
        }
    }

    /// Tests / RealisedBy render as dashed lines.
    fn dashed(self) -> bool {
        matches!(self, EdgeKindUi::Tests | EdgeKindUi::RealisedBy)
    }
}

#[derive(Clone)]
struct UniverseNode {
    kind: NodeKindUi,
    label: String,
    open_path: Option<PathBuf>,
}

#[derive(Default)]
struct NeighbourBuckets {
    /// Both directions of all four spec→spec edge kinds.
    specs: Vec<NodeId>,
    /// `RealisedBy` outgoing from a spec.
    source: Vec<NodeId>,
    /// `Tests` outgoing from a spec.
    tests: Vec<NodeId>,
}

struct Universe {
    nodes: HashMap<NodeId, UniverseNode>,
    edges: Vec<(NodeId, NodeId, EdgeKindUi)>,
    neighbours: HashMap<NodeId, NeighbourBuckets>,
}

#[derive(Clone, Copy, Default)]
struct ExpandedFlags {
    specs: bool,
    source: bool,
    tests: bool,
}

#[derive(Clone)]
struct VisibleNode {
    pos: Pos2,
    vel: Vec2,
    pinned: bool,
}

#[derive(Default)]
struct VisibleGraph {
    nodes: HashMap<NodeId, VisibleNode>,
    edges: HashSet<(NodeId, NodeId, EdgeKindUi)>,
    refcounts: HashMap<NodeId, u32>,
    expanded: HashMap<NodeId, ExpandedFlags>,
    seeds: HashSet<NodeId>,
}

#[derive(Copy, Clone)]
struct Camera {
    centre: Pos2,
    zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            centre: Pos2::ZERO,
            zoom: 1.0,
        }
    }
}

struct DragState {
    id: NodeId,
    /// Pointer position at drag start, in screen coords.
    grab_screen: Pos2,
    /// Node position at drag start, in world coords.
    grab_world: Pos2,
    moved: bool,
}

struct EdgeFilters {
    parent: bool,
    implements: bool,
    depends_on: bool,
    body_ref: bool,
    realised_by: bool,
    tests: bool,
}

impl Default for EdgeFilters {
    fn default() -> Self {
        Self {
            parent: true,
            implements: true,
            depends_on: true,
            body_ref: false, // Off — body refs are noise unless deliberately enabled.
            realised_by: true,
            tests: true,
        }
    }
}

impl EdgeFilters {
    fn is_on(&self, kind: EdgeKindUi) -> bool {
        match kind {
            EdgeKindUi::Parent => self.parent,
            EdgeKindUi::Implements => self.implements,
            EdgeKindUi::DependsOn => self.depends_on,
            EdgeKindUi::BodyRef => self.body_ref,
            EdgeKindUi::RealisedBy => self.realised_by,
            EdgeKindUi::Tests => self.tests,
        }
    }
}

// ---------------------------------------------------------------- panel

pub struct SpecGraphPanel {
    workspace_root: PathBuf,
    universe: Option<Universe>,
    visible: VisibleGraph,
    camera: Camera,
    params: LayoutParams,
    last_kinetic: f32,
    filters: EdgeFilters,
    hover: Option<NodeId>,
    selected: Option<NodeId>,
    drag: Option<DragState>,
    rng_seed: u64,
}

impl SpecGraphPanel {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            universe: None,
            visible: VisibleGraph::default(),
            camera: Camera::default(),
            params: LayoutParams::default(),
            last_kinetic: f32::INFINITY,
            filters: EdgeFilters::default(),
            hover: None,
            selected: None,
            drag: None,
            rng_seed: 0xC0FFEE,
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui, state: &Arc<StdMutex<SharedState>>) {
        if self.universe.is_none() {
            self.rebuild_universe();
        }

        // Header row.
        let mut refresh = false;
        let mut collapse_all = false;
        let mut fit_view = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("spec graph").strong());
            if ui
                .small_button("⟳")
                .on_hover_text("rebuild from disk")
                .clicked()
            {
                refresh = true;
            }
            if ui
                .small_button("⌖")
                .on_hover_text("collapse all to seeds")
                .clicked()
            {
                collapse_all = true;
            }
            if ui
                .small_button("fit")
                .on_hover_text("centre & zoom to fit visible")
                .clicked()
            {
                fit_view = true;
            }
            ui.separator();
            ui.label(RichText::new("edges:").color(theme::muted_text()));
            ui.checkbox(&mut self.filters.parent, "parent");
            ui.checkbox(&mut self.filters.implements, "impl");
            ui.checkbox(&mut self.filters.depends_on, "deps");
            ui.checkbox(&mut self.filters.body_ref, "refs");
            ui.checkbox(&mut self.filters.realised_by, "code");
            ui.checkbox(&mut self.filters.tests, "tests");
        });
        ui.separator();

        if refresh {
            self.rebuild_universe();
            self.visible = VisibleGraph::default();
            self.seed_overview();
        }
        if collapse_all {
            self.collapse_all_to_seeds();
        }

        // Canvas allocation.
        let available = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(available, Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        // Zoom around pointer.
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let factor = (scroll * 0.005).exp();
                let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
                let world_before = self.screen_to_world(pointer, rect);
                self.camera.zoom = (self.camera.zoom * factor).clamp(0.1, 4.0);
                let world_after = self.screen_to_world(pointer, rect);
                self.camera.centre += world_before - world_after;
            }
        }

        if fit_view {
            self.fit_view(rect);
        }

        // Pointer handling.
        let pointer = response.interact_pointer_pos();
        let primary_down = ui.input(|i| i.pointer.primary_down());
        let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
        let primary_released = ui.input(|i| i.pointer.primary_released());
        let secondary_clicked = response.secondary_clicked();

        // Hit-test the pointer.
        let pointer_world = pointer.map(|p| self.screen_to_world(p, rect));
        let hit = pointer_world.and_then(|p| self.hit_test(p));

        // Chip clicks — fire on primary release.
        let mut chip_target: Option<(NodeId, ChipKind)> = None;
        if primary_released && let (Some(pos), Some((id, hit_kind))) = (pointer_world, &hit) {
            if let HitKind::Chip(chip) = hit_kind {
                chip_target = Some((id.clone(), *chip));
            }
            let _ = pos; // hit-test already used the position
        }
        if let Some((id, chip)) = chip_target {
            self.toggle(&id, chip);
        }

        // Drag handling on node bodies.
        if primary_pressed && let Some((id, HitKind::Body)) = &hit {
            let world = pointer_world.unwrap();
            let node_pos = self.visible.nodes.get(id).map(|n| n.pos).unwrap_or(world);
            self.drag = Some(DragState {
                id: id.clone(),
                grab_screen: pointer.unwrap(),
                grab_world: node_pos,
                moved: false,
            });
        }
        if primary_down
            && let Some(drag) = &mut self.drag
            && let Some(curr_screen) = pointer
        {
            let delta_screen = curr_screen - drag.grab_screen;
            let delta_world = delta_screen / self.camera.zoom;
            let new_pos = drag.grab_world + delta_world;
            if delta_screen.length_sq() > 4.0 {
                drag.moved = true;
            }
            if let Some(n) = self.visible.nodes.get_mut(&drag.id) {
                n.pos = new_pos;
                n.vel = Vec2::ZERO;
                n.pinned = true;
            }
            self.last_kinetic = f32::INFINITY; // keep stepping while drag-ish
        }
        if primary_released {
            self.drag = None;
        }

        // Pan: secondary-button-drag on empty canvas.
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Secondary) && hit.is_none())
        {
            let delta = response.drag_delta();
            self.camera.centre -= delta / self.camera.zoom;
        }

        // Double-click → open as centre tab.
        if response.double_clicked()
            && let Some((id, HitKind::Body)) = &hit
            && let Some(meta) = self.universe.as_ref().and_then(|u| u.nodes.get(id))
            && let Some(path) = &meta.open_path
        {
            push_open(state, &self.workspace_root, path.clone(), meta.kind);
        }

        // Single-click on body → select (focus).
        if response.clicked() && self.drag.is_none() {
            if let Some((id, HitKind::Body)) = &hit {
                self.selected = Some(id.clone());
            } else {
                self.selected = None;
            }
        }

        // Right-click on node → context menu.
        if secondary_clicked && let Some((id, HitKind::Body)) = hit.clone() {
            let id_for_menu = id.clone();
            response.clone().context_menu(|ui| {
                if ui.button("Open").clicked() {
                    if let Some(meta) = self
                        .universe
                        .as_ref()
                        .and_then(|u| u.nodes.get(&id_for_menu))
                        && let Some(path) = &meta.open_path
                    {
                        push_open(state, &self.workspace_root, path.clone(), meta.kind);
                    }
                    ui.close_menu();
                }
                if ui.button("Hide").clicked() {
                    self.hide_node(&id_for_menu);
                    ui.close_menu();
                }
                if ui.button("Unpin").clicked() {
                    if let Some(n) = self.visible.nodes.get_mut(&id_for_menu) {
                        n.pinned = false;
                        self.last_kinetic = f32::INFINITY;
                    }
                    ui.close_menu();
                }
            });
        }

        self.hover = hit.as_ref().map(|(id, _)| id.clone());

        // Run a physics step if not yet settled.
        if self.last_kinetic > self.params.kinetic_threshold {
            let (mut sim_nodes, sim_edges) = self.snapshot_for_sim();
            self.last_kinetic = step(&mut sim_nodes, &sim_edges, &self.params, 1.0);
            self.apply_sim_back(sim_nodes);
            ui.ctx().request_repaint();
        }

        // Draw edges, then nodes, then chips, then hover label.
        self.draw_edges(&painter, rect);
        self.draw_nodes(&painter, rect);
        if let Some(id) = &self.hover {
            self.draw_hover_label(&painter, rect, id);
        }
    }

    // ------------------------------------------------------------ build

    fn rebuild_universe(&mut self) {
        let records = walk_specs(&self.workspace_root);
        let inputs: Vec<GraphInput> = records
            .into_iter()
            .map(|r| GraphInput {
                canonical_id: r.canonical_id,
                file: r.file,
                path: r.path,
            })
            .collect();
        let graph = SpecGraph::build(&inputs);

        let mut nodes: HashMap<NodeId, UniverseNode> = HashMap::new();
        let mut edges: Vec<(NodeId, NodeId, EdgeKindUi)> = Vec::new();
        let mut neighbours: HashMap<NodeId, NeighbourBuckets> = HashMap::new();

        // 1) Spec nodes
        for spec in graph.nodes() {
            nodes.insert(
                spec.id.clone(),
                UniverseNode {
                    kind: NodeKindUi::from_spec_kind(spec.kind),
                    label: short_label(&spec.id),
                    open_path: Some(spec.path.clone()),
                },
            );
            neighbours.entry(spec.id.clone()).or_default();
        }

        // 2) Spec→spec edges, both directions for the `specs` bucket.
        for input in &inputs {
            let src_id = &input.canonical_id;
            for (n, kind) in graph.outbound(src_id) {
                edges.push((
                    src_id.clone(),
                    n.id.clone(),
                    EdgeKindUi::from_spec_edge(kind),
                ));
                neighbours
                    .entry(src_id.clone())
                    .or_default()
                    .specs
                    .push(n.id.clone());
                neighbours
                    .entry(n.id.clone())
                    .or_default()
                    .specs
                    .push(src_id.clone());
            }
        }

        // 3) Code-file nodes + RealisedBy edges
        for input in &inputs {
            let src_id = &input.canonical_id;
            for code in &input.file.frontmatter.code {
                let abs = if code.is_absolute() {
                    code.clone()
                } else {
                    self.workspace_root.join(code)
                };
                let id = format!("code:{}", code.to_string_lossy().replace('\\', "/"));
                nodes.entry(id.clone()).or_insert_with(|| UniverseNode {
                    kind: NodeKindUi::CodeFile,
                    label: file_basename(code),
                    open_path: Some(abs),
                });
                edges.push((src_id.clone(), id.clone(), EdgeKindUi::RealisedBy));
                neighbours
                    .entry(src_id.clone())
                    .or_default()
                    .source
                    .push(id.clone());
                neighbours.entry(id).or_default();
            }
        }

        // 4) Test nodes + Tests edges
        for input in &inputs {
            let src_id = &input.canonical_id;
            for test in &input.file.frontmatter.tests {
                let (id, label, open_path) = match test {
                    TestRef::Function { path, name } => {
                        let abs = if path.is_absolute() {
                            path.clone()
                        } else {
                            self.workspace_root.join(path)
                        };
                        (
                            format!("test:{}::{name}", path.to_string_lossy().replace('\\', "/")),
                            format!("{}::{name}", file_basename(path)),
                            Some(abs),
                        )
                    }
                    TestRef::WholeFile { path } => {
                        let abs = if path.is_absolute() {
                            path.clone()
                        } else {
                            self.workspace_root.join(path)
                        };
                        (
                            format!("test:{}::*", path.to_string_lossy().replace('\\', "/")),
                            format!("{}::*", file_basename(path)),
                            Some(abs),
                        )
                    }
                };
                nodes.entry(id.clone()).or_insert_with(|| UniverseNode {
                    kind: NodeKindUi::Test,
                    label,
                    open_path,
                });
                edges.push((src_id.clone(), id.clone(), EdgeKindUi::Tests));
                neighbours
                    .entry(src_id.clone())
                    .or_default()
                    .tests
                    .push(id.clone());
                neighbours.entry(id).or_default();
            }
        }

        // Dedup neighbour buckets.
        for nb in neighbours.values_mut() {
            dedup(&mut nb.specs);
            dedup(&mut nb.source);
            dedup(&mut nb.tests);
        }

        self.universe = Some(Universe {
            nodes,
            edges,
            neighbours,
        });

        // Seed with overview if not already.
        if self.visible.nodes.is_empty() {
            self.seed_overview();
        }
    }

    fn seed_overview(&mut self) {
        let candidate = "overview".to_string();
        let id = self
            .universe
            .as_ref()
            .filter(|u| u.nodes.contains_key(&candidate))
            .map(|_| candidate)
            .or_else(|| {
                // Fallback: any node with kind Overview.
                self.universe.as_ref().and_then(|u| {
                    u.nodes
                        .iter()
                        .find(|(_, m)| matches!(m.kind, NodeKindUi::SpecOverview))
                        .map(|(id, _)| id.clone())
                })
            });
        if let Some(id) = id {
            self.add_node(&id, Pos2::ZERO);
            self.visible.seeds.insert(id);
        }
    }

    fn add_node(&mut self, id: &NodeId, near: Pos2) {
        if !self.visible.nodes.contains_key(id) {
            let offset = self.tiny_offset();
            self.visible.nodes.insert(
                id.clone(),
                VisibleNode {
                    pos: near + offset,
                    vel: Vec2::ZERO,
                    pinned: false,
                },
            );
        }
        *self.visible.refcounts.entry(id.clone()).or_insert(0) += 1;
        self.last_kinetic = f32::INFINITY;
    }

    fn remove_node(&mut self, id: &NodeId) {
        if self.visible.seeds.contains(id) {
            return; // seeds are protected
        }
        if let Some(rc) = self.visible.refcounts.get(id).copied()
            && rc == 0
        {
            self.visible.nodes.remove(id);
            self.visible.refcounts.remove(id);
            self.visible.expanded.remove(id);
            self.visible.edges.retain(|(a, b, _)| a != id && b != id);
        }
    }

    fn refresh_edges_for(&mut self, id: &NodeId) {
        let Some(u) = self.universe.as_ref() else {
            return;
        };
        for (from, to, kind) in &u.edges {
            if (from == id || to == id)
                && self.visible.nodes.contains_key(from)
                && self.visible.nodes.contains_key(to)
            {
                self.visible.edges.insert((from.clone(), to.clone(), *kind));
            }
        }
    }

    fn toggle(&mut self, id: &NodeId, chip: ChipKind) {
        let already = self.visible.expanded.get(id).copied().unwrap_or_default();
        let on = match chip {
            ChipKind::Specs => already.specs,
            ChipKind::Source => already.source,
            ChipKind::Tests => already.tests,
        };
        if on {
            self.collapse(id, chip);
        } else {
            self.expand(id, chip);
        }
    }

    fn expand(&mut self, id: &NodeId, chip: ChipKind) {
        let near = self
            .visible
            .nodes
            .get(id)
            .map(|n| n.pos)
            .unwrap_or(Pos2::ZERO);
        let neighbours: Vec<NodeId> = {
            let Some(u) = self.universe.as_ref() else {
                return;
            };
            let Some(buckets) = u.neighbours.get(id) else {
                return;
            };
            match chip {
                ChipKind::Specs => buckets.specs.clone(),
                ChipKind::Source => buckets.source.clone(),
                ChipKind::Tests => buckets.tests.clone(),
            }
        };
        for n in &neighbours {
            self.add_node(n, near);
        }
        // Refresh edges for all newly-added (and the parent) so the new
        // connections show up.
        let mut to_refresh = neighbours.clone();
        to_refresh.push(id.clone());
        for nid in to_refresh {
            self.refresh_edges_for(&nid);
        }
        let entry = self.visible.expanded.entry(id.clone()).or_default();
        match chip {
            ChipKind::Specs => entry.specs = true,
            ChipKind::Source => entry.source = true,
            ChipKind::Tests => entry.tests = true,
        }
    }

    fn collapse(&mut self, id: &NodeId, chip: ChipKind) {
        let neighbours: Vec<NodeId> = {
            let Some(u) = self.universe.as_ref() else {
                return;
            };
            let Some(buckets) = u.neighbours.get(id) else {
                return;
            };
            match chip {
                ChipKind::Specs => buckets.specs.clone(),
                ChipKind::Source => buckets.source.clone(),
                ChipKind::Tests => buckets.tests.clone(),
            }
        };
        for n in &neighbours {
            if let Some(rc) = self.visible.refcounts.get_mut(n) {
                *rc = rc.saturating_sub(1);
            }
            self.remove_node(n);
        }
        let entry = self.visible.expanded.entry(id.clone()).or_default();
        match chip {
            ChipKind::Specs => entry.specs = false,
            ChipKind::Source => entry.source = false,
            ChipKind::Tests => entry.tests = false,
        }
    }

    fn hide_node(&mut self, id: &NodeId) {
        if self.visible.seeds.contains(id) {
            return;
        }
        self.visible.refcounts.insert(id.clone(), 0);
        self.visible.nodes.remove(id);
        self.visible.expanded.remove(id);
        self.visible.edges.retain(|(a, b, _)| a != id && b != id);
    }

    fn collapse_all_to_seeds(&mut self) {
        let seeds = self.visible.seeds.clone();
        let to_keep: HashSet<NodeId> = seeds.iter().cloned().collect();
        self.visible.nodes.retain(|k, _| to_keep.contains(k));
        self.visible.refcounts.retain(|k, _| to_keep.contains(k));
        for id in &to_keep {
            self.visible.refcounts.insert(id.clone(), 1);
        }
        self.visible.expanded.clear();
        self.visible.edges.clear();
        self.last_kinetic = f32::INFINITY;
    }

    // ------------------------------------------------------------ rendering helpers

    fn screen_to_world(&self, p: Pos2, rect: Rect) -> Pos2 {
        let centre = rect.center();
        let offset = (p - centre) / self.camera.zoom;
        self.camera.centre + offset
    }

    fn world_to_screen(&self, p: Pos2, rect: Rect) -> Pos2 {
        let centre = rect.center();
        centre + (p - self.camera.centre) * self.camera.zoom
    }

    fn fit_view(&mut self, rect: Rect) {
        if self.visible.nodes.is_empty() {
            self.camera.centre = Pos2::ZERO;
            self.camera.zoom = 1.0;
            return;
        }
        let mut min = Pos2::new(f32::INFINITY, f32::INFINITY);
        let mut max = Pos2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        for n in self.visible.nodes.values() {
            min.x = min.x.min(n.pos.x);
            min.y = min.y.min(n.pos.y);
            max.x = max.x.max(n.pos.x);
            max.y = max.y.max(n.pos.y);
        }
        let span = (max - min).max(Vec2::splat(40.0));
        self.camera.centre = Pos2::new((min.x + max.x) / 2.0, (min.y + max.y) / 2.0);
        let zx = (rect.width() * 0.85) / span.x;
        let zy = (rect.height() * 0.85) / span.y;
        self.camera.zoom = zx.min(zy).clamp(0.1, 4.0);
    }

    fn node_radius(&self) -> f32 {
        14.0 * self.camera.zoom
    }

    fn draw_nodes(&self, painter: &egui::Painter, rect: Rect) {
        let radius = self.node_radius();
        let font = FontId::proportional(11.0 * self.camera.zoom.max(0.7));
        for (id, node) in &self.visible.nodes {
            let Some(meta) = self.universe.as_ref().and_then(|u| u.nodes.get(id)) else {
                continue;
            };
            let screen = self.world_to_screen(node.pos, rect);
            let fill = meta.kind.colour();
            painter.circle(
                screen,
                radius,
                fill.linear_multiply(0.25),
                Stroke::new(1.5, fill),
            );
            if self.selected.as_ref() == Some(id) {
                painter.circle_stroke(screen, radius + 4.0, Stroke::new(2.0, Color32::WHITE));
            }
            painter.text(
                screen + Vec2::new(0.0, radius + 4.0),
                Align2::CENTER_TOP,
                &meta.label,
                font.clone(),
                Color32::WHITE,
            );
            // Chips below the label.
            self.draw_chips(painter, screen, radius, id, meta);
        }
    }

    fn chip_rects(&self, anchor: Pos2, radius: f32, id: &NodeId) -> Vec<(Rect, ChipKind, bool)> {
        // Returns (rect, chip_kind, expanded) for chips that should
        // exist for this node (bucket non-empty).
        let Some(u) = self.universe.as_ref() else {
            return Vec::new();
        };
        let Some(buckets) = u.neighbours.get(id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let chip_w = 22.0 * self.camera.zoom.max(0.7);
        let chip_h = 14.0 * self.camera.zoom.max(0.7);
        let gap = 4.0 * self.camera.zoom.max(0.7);
        let row_y = anchor.y + radius + 20.0 * self.camera.zoom.max(0.7);
        let exp = self.visible.expanded.get(id).copied().unwrap_or_default();
        let mut chips = Vec::new();
        if !buckets.specs.is_empty() {
            chips.push((ChipKind::Specs, exp.specs));
        }
        if !buckets.source.is_empty() {
            chips.push((ChipKind::Source, exp.source));
        }
        if !buckets.tests.is_empty() {
            chips.push((ChipKind::Tests, exp.tests));
        }
        let total_w = chips.len() as f32 * chip_w + (chips.len().saturating_sub(1) as f32) * gap;
        let start_x = anchor.x - total_w / 2.0;
        for (i, (kind, expanded)) in chips.into_iter().enumerate() {
            let x = start_x + i as f32 * (chip_w + gap);
            let rect = Rect::from_min_size(Pos2::new(x, row_y), Vec2::new(chip_w, chip_h));
            out.push((rect, kind, expanded));
        }
        out
    }

    fn draw_chips(
        &self,
        painter: &egui::Painter,
        anchor: Pos2,
        radius: f32,
        id: &NodeId,
        _meta: &UniverseNode,
    ) {
        let font = FontId::monospace(10.0 * self.camera.zoom.max(0.7));
        for (rect, kind, expanded) in self.chip_rects(anchor, radius, id) {
            let bg = Color32::from_rgb(50, 54, 64);
            painter.rect_filled(rect, 3.0, bg);
            painter.rect_stroke(rect, 3.0, Stroke::new(1.0, theme::muted_text()));
            let label = chip_label(kind, expanded);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                label,
                font.clone(),
                Color32::WHITE,
            );
        }
    }

    fn draw_edges(&self, painter: &egui::Painter, rect: Rect) {
        for (from, to, kind) in &self.visible.edges {
            if !self.filters.is_on(*kind) {
                continue;
            }
            let (Some(a), Some(b)) = (self.visible.nodes.get(from), self.visible.nodes.get(to))
            else {
                continue;
            };
            let sa = self.world_to_screen(a.pos, rect);
            let sb = self.world_to_screen(b.pos, rect);
            let stroke = kind.stroke();
            if kind.dashed() {
                draw_dashed(painter, sa, sb, stroke);
            } else {
                painter.line_segment([sa, sb], stroke);
            }
        }
    }

    fn draw_hover_label(&self, painter: &egui::Painter, rect: Rect, id: &NodeId) {
        let Some(node) = self.visible.nodes.get(id) else {
            return;
        };
        let _ = self.universe.as_ref().and_then(|u| u.nodes.get(id));
        let screen = self.world_to_screen(node.pos, rect);
        let font = FontId::monospace(10.0);
        painter.text(
            screen + Vec2::new(0.0, -self.node_radius() - 4.0),
            Align2::CENTER_BOTTOM,
            id,
            font,
            theme::muted_text(),
        );
    }

    // ------------------------------------------------------------ hit test

    fn hit_test(&self, world: Pos2) -> Option<(NodeId, HitKind)> {
        // Chips first (they sit below the node).
        let radius = self.node_radius();
        for (id, node) in &self.visible.nodes {
            let anchor_screen = self.world_to_screen(node.pos, dummy_rect());
            let _ = anchor_screen; // chip_rects works in screen coords; convert pointer instead
            let chips = self.chip_rects(self.world_to_screen(node.pos, dummy_rect()), radius, id);
            let p_screen_for_chip = self.world_to_screen(world, dummy_rect());
            for (rect, kind, _) in chips {
                if rect.contains(p_screen_for_chip) {
                    return Some((id.clone(), HitKind::Chip(kind)));
                }
            }
        }
        // Then bodies.
        for (id, node) in &self.visible.nodes {
            let r_world = self.node_radius() / self.camera.zoom;
            if (world - node.pos).length() <= r_world {
                return Some((id.clone(), HitKind::Body));
            }
        }
        None
    }

    // ------------------------------------------------------------ sim glue

    fn snapshot_for_sim(&self) -> (Vec<LayoutNode<NodeId>>, Vec<LayoutEdge<NodeId>>) {
        let nodes = self
            .visible
            .nodes
            .iter()
            .map(|(id, n)| LayoutNode {
                id: id.clone(),
                pos: n.pos,
                vel: n.vel,
                pinned: n.pinned,
            })
            .collect();
        let edges = self
            .visible
            .edges
            .iter()
            .map(|(a, b, k)| LayoutEdge {
                from: a.clone(),
                to: b.clone(),
                kind: k.to_force(),
            })
            .collect();
        (nodes, edges)
    }

    fn apply_sim_back(&mut self, sim_nodes: Vec<LayoutNode<NodeId>>) {
        for s in sim_nodes {
            if let Some(n) = self.visible.nodes.get_mut(&s.id) {
                n.pos = s.pos;
                n.vel = s.vel;
            }
        }
    }

    fn tiny_offset(&mut self) -> Vec2 {
        // Cheap deterministic-ish jitter via xorshift on rng_seed.
        self.rng_seed ^= self.rng_seed << 13;
        self.rng_seed ^= self.rng_seed >> 7;
        self.rng_seed ^= self.rng_seed << 17;
        let theta = (self.rng_seed as f32 * 0.0001) % std::f32::consts::TAU;
        Vec2::angled(theta) * 8.0
    }
}

// ---------------------------------------------------------------- helpers

#[derive(Clone)]
enum HitKind {
    Body,
    Chip(ChipKind),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChipKind {
    Specs,
    Source,
    Tests,
}

fn chip_label(kind: ChipKind, expanded: bool) -> &'static str {
    match (kind, expanded) {
        (ChipKind::Specs, false) => "+S",
        (ChipKind::Specs, true) => "−S",
        (ChipKind::Source, false) => "+C",
        (ChipKind::Source, true) => "−C",
        (ChipKind::Tests, false) => "+T",
        (ChipKind::Tests, true) => "−T",
    }
}

fn short_label(canonical_id: &str) -> String {
    // Keep the trailing segment for compactness ("file-tabs" instead of
    // "components/gui/file-tabs").
    canonical_id
        .rsplit('/')
        .next()
        .unwrap_or(canonical_id)
        .to_string()
}

fn file_basename(path: &std::path::Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn dedup(v: &mut Vec<NodeId>) {
    let mut seen = HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
}

fn dummy_rect() -> Rect {
    // hit_test's chip code needs a rect to project through. The rect's
    // centre cancels out because we project both the anchor and the
    // pointer through the same camera+rect, so any rect works.
    Rect::from_min_size(Pos2::ZERO, Vec2::splat(1000.0))
}

fn draw_dashed(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    let total = (b - a).length();
    if total < 0.001 {
        return;
    }
    let dir = (b - a) / total;
    let dash = 6.0;
    let gap = 4.0;
    let mut t = 0.0;
    while t < total {
        let end = (t + dash).min(total);
        painter.line_segment([a + dir * t, a + dir * end], stroke);
        t = end + gap;
    }
}

fn push_open(
    state: &Arc<StdMutex<SharedState>>,
    workspace_root: &std::path::Path,
    abs_path: PathBuf,
    kind: NodeKindUi,
) {
    let path = abs_path
        .strip_prefix(workspace_root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| abs_path.clone());
    let source = if matches!(kind, NodeKindUi::CodeFile | NodeKindUi::Test) {
        FileSource::Code
    } else {
        FileSource::Spec
    };
    let tab = DockTab::File { path, source };
    if let Ok(mut s) = state.lock()
        && !s.pending_centre_tabs.contains(&tab)
    {
        s.pending_centre_tabs.push(tab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chip_label_flips_on_expanded() {
        assert_eq!(chip_label(ChipKind::Specs, false), "+S");
        assert_eq!(chip_label(ChipKind::Specs, true), "−S");
        assert_eq!(chip_label(ChipKind::Source, false), "+C");
        assert_eq!(chip_label(ChipKind::Tests, true), "−T");
    }

    #[test]
    fn short_label_keeps_trailing_segment() {
        assert_eq!(short_label("components/gui/file-tabs"), "file-tabs");
        assert_eq!(short_label("overview"), "overview");
    }

    #[test]
    fn dedup_preserves_order_and_removes_duplicates() {
        let mut v: Vec<NodeId> = vec!["a".into(), "b".into(), "a".into(), "c".into(), "b".into()];
        dedup(&mut v);
        assert_eq!(v, vec!["a".to_string(), "b".into(), "c".into()]);
    }
}
