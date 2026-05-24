// Force-directed layout engine for the spec-graph panel.
//
// Realises the "Layout" section of spec/components/gui/spec-graph-panel.md.
//
// Pure-Rust, no deps beyond egui's Pos2/Vec2 (kept here for ergonomics —
// the panel passes them in and back out). Per-frame `step(dt)` applies:
//   - O(n²) inter-node repulsion (k_rep / d², clamped at d_min)
//   - per-edge spring attraction with kind-specific stiffness
//   - centre gravity so disconnected components don't drift away
//   - velocity damping
//
// Pinned nodes (user-dragged) skip force integration; the caller writes
// their pos directly each frame from the drag delta.
//
// Naive O(n²) is fine at the target scale (≤300 nodes); Barnes-Hut is
// recorded as a follow-up for real workloads that blow past that.

use std::collections::HashMap;

use egui::{Pos2, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKindForce {
    Parent,
    Implements,
    DependsOn,
    BodyRef,
    RealisedBy,
    Tests,
}

impl EdgeKindForce {
    /// Spring stiffness per edge kind. Stiffer = pulls neighbours
    /// closer together. Parent is the hierarchy backbone; BodyRef is
    /// the loosest because it's the noisiest signal.
    pub fn spring_k(self) -> f32 {
        match self {
            EdgeKindForce::Parent => 0.08,
            EdgeKindForce::Implements => 0.06,
            EdgeKindForce::RealisedBy => 0.06,
            EdgeKindForce::Tests => 0.05,
            EdgeKindForce::DependsOn => 0.04,
            EdgeKindForce::BodyRef => 0.02,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutParams {
    /// Numerator of the repulsion law (F = k_rep / d²).
    pub k_rep: f32,
    /// Minimum distance for repulsion — clamps F to a finite value
    /// when two nodes overlap.
    pub d_min: f32,
    /// Rest length of every spring.
    pub rest_length: f32,
    /// Pull toward the centre of mass (keeps disconnected components
    /// from drifting off-canvas).
    pub gravity: f32,
    /// Velocity multiplier per step. 0.85 = settles in a few seconds.
    pub damping: f32,
    /// Once total kinetic energy drops below this, callers should stop
    /// stepping until the user perturbs the graph again.
    pub kinetic_threshold: f32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            k_rep: 4000.0,
            d_min: 12.0,
            rest_length: 90.0,
            gravity: 0.01,
            damping: 0.85,
            kinetic_threshold: 0.05,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayoutNode<Id> {
    pub id: Id,
    pub pos: Pos2,
    pub vel: Vec2,
    pub pinned: bool,
}

#[derive(Debug, Clone)]
pub struct LayoutEdge<Id> {
    pub from: Id,
    pub to: Id,
    pub kind: EdgeKindForce,
}

/// Apply one simulation step in-place. Returns total kinetic energy
/// after the step — callers compare against `params.kinetic_threshold`
/// to decide whether to keep stepping.
///
/// `dt` is in frame-units; pass `1.0` for a per-frame call.
pub fn step<Id: Eq + std::hash::Hash + Clone>(
    nodes: &mut [LayoutNode<Id>],
    edges: &[LayoutEdge<Id>],
    params: &LayoutParams,
    dt: f32,
) -> f32 {
    // Index lookup so edge resolution is O(1).
    let idx: HashMap<Id, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();

    // Compute forces into a parallel vector so we can read node
    // positions without aliasing the &mut.
    let n = nodes.len();
    let mut force = vec![Vec2::ZERO; n];

    // Centre of mass (gravity anchor). egui's Vec2 doesn't impl Sum,
    // so fold manually.
    let centre = if n == 0 {
        Pos2::ZERO
    } else {
        let sum = nodes
            .iter()
            .fold(Vec2::ZERO, |acc, node| acc + node.pos.to_vec2());
        (sum / n as f32).to_pos2()
    };

    // Pairwise repulsion. When two nodes are coincident, the raw
    // delta is zero — pick a deterministic-ish offset based on the
    // index pair so they push apart instead of staying stuck.
    for i in 0..n {
        for j in (i + 1)..n {
            let raw_delta = nodes[i].pos - nodes[j].pos;
            let delta = if raw_delta.length_sq() < 1e-6 {
                let theta = ((i * 31 + j * 17) as f32).sin() * std::f32::consts::TAU;
                Vec2::angled(theta)
            } else {
                raw_delta
            };
            let dist = delta.length().max(params.d_min);
            let unit = delta / dist;
            let f = params.k_rep / (dist * dist);
            force[i] += unit * f;
            force[j] -= unit * f;
        }
    }

    // Spring attraction along edges.
    for e in edges {
        let (Some(&i), Some(&j)) = (idx.get(&e.from), idx.get(&e.to)) else {
            continue;
        };
        if i == j {
            continue;
        }
        let delta = nodes[j].pos - nodes[i].pos;
        let dist = delta.length().max(1.0);
        let unit = delta / dist;
        let displacement = dist - params.rest_length;
        let f = displacement * e.kind.spring_k();
        force[i] += unit * f;
        force[j] -= unit * f;
    }

    // Centre gravity.
    for (i, node) in nodes.iter().enumerate() {
        let to_centre = centre - node.pos;
        force[i] += to_centre * params.gravity;
    }

    // Integrate. Pinned nodes ignore forces and keep velocity at zero
    // so the caller's pos write isn't fought.
    let mut kinetic = 0.0_f32;
    for (i, node) in nodes.iter_mut().enumerate() {
        if node.pinned {
            node.vel = Vec2::ZERO;
            continue;
        }
        node.vel = (node.vel + force[i] * dt) * params.damping;
        node.pos += node.vel * dt;
        kinetic += node.vel.length_sq();
    }
    kinetic
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: u32, x: f32, y: f32) -> LayoutNode<u32> {
        LayoutNode {
            id,
            pos: Pos2::new(x, y),
            vel: Vec2::ZERO,
            pinned: false,
        }
    }

    fn e(from: u32, to: u32, kind: EdgeKindForce) -> LayoutEdge<u32> {
        LayoutEdge { from, to, kind }
    }

    fn run(nodes: &mut [LayoutNode<u32>], edges: &[LayoutEdge<u32>], steps: usize) {
        let params = LayoutParams::default();
        for _ in 0..steps {
            step(nodes, edges, &params, 1.0);
        }
    }

    #[test]
    fn nodes_never_nan_over_a_thousand_steps() {
        let mut nodes = vec![n(0, 0.0, 0.0), n(1, 10.0, 0.0), n(2, 5.0, 8.66)];
        let edges = vec![
            e(0, 1, EdgeKindForce::Parent),
            e(1, 2, EdgeKindForce::Implements),
        ];
        run(&mut nodes, &edges, 1000);
        for node in &nodes {
            assert!(
                node.pos.x.is_finite() && node.pos.y.is_finite(),
                "node {} has non-finite pos {:?}",
                node.id,
                node.pos
            );
        }
    }

    #[test]
    fn overlapping_nodes_separate() {
        // Two nodes spawned exactly on top of each other should push
        // apart, not stay coincident or blow up.
        let mut nodes = vec![n(0, 50.0, 50.0), n(1, 50.0, 50.0)];
        run(&mut nodes, &[], 200);
        let separation = (nodes[0].pos - nodes[1].pos).length();
        assert!(
            separation > LayoutParams::default().d_min,
            "expected nodes to separate past d_min, got {separation}"
        );
    }

    #[test]
    fn connected_pair_settles_within_bounded_distance() {
        // A spring should hold a connected pair within roughly the
        // rest length plus some slack.
        let mut nodes = vec![n(0, -200.0, 0.0), n(1, 200.0, 0.0)];
        let edges = vec![e(0, 1, EdgeKindForce::Parent)];
        // Several thousand steps to let the spring fully damp.
        run(&mut nodes, &edges, 5000);
        let d = (nodes[0].pos - nodes[1].pos).length();
        let rest = LayoutParams::default().rest_length;
        assert!(
            d < rest * 3.0,
            "connected pair separation {d} much larger than 3 * rest_length ({})",
            rest * 3.0
        );
    }

    #[test]
    fn pinned_node_does_not_move() {
        let mut nodes = vec![
            LayoutNode {
                id: 0,
                pos: Pos2::new(100.0, 100.0),
                vel: Vec2::ZERO,
                pinned: true,
            },
            n(1, 200.0, 100.0),
        ];
        let edges = vec![e(0, 1, EdgeKindForce::Parent)];
        let pinned_pos = nodes[0].pos;
        run(&mut nodes, &edges, 500);
        assert_eq!(
            nodes[0].pos, pinned_pos,
            "pinned node moved despite the spring force"
        );
    }

    #[test]
    fn empty_graph_does_not_panic() {
        let mut nodes: Vec<LayoutNode<u32>> = Vec::new();
        let edges: Vec<LayoutEdge<u32>> = Vec::new();
        let ke = step(&mut nodes, &edges, &LayoutParams::default(), 1.0);
        assert_eq!(ke, 0.0);
    }

    #[test]
    fn kinetic_energy_decreases_after_initial_perturbation() {
        // Start a pair far apart with a spring; KE peaks then decays.
        let mut nodes = vec![n(0, -300.0, 0.0), n(1, 300.0, 0.0)];
        let edges = vec![e(0, 1, EdgeKindForce::Parent)];
        let params = LayoutParams::default();
        let mut energies = Vec::new();
        for _ in 0..200 {
            energies.push(step(&mut nodes, &edges, &params, 1.0));
        }
        // The last 50 steps' average KE should be less than the first 50's.
        let early: f32 = energies[..50].iter().sum::<f32>() / 50.0;
        let late: f32 = energies[150..].iter().sum::<f32>() / 50.0;
        assert!(
            late < early,
            "expected damping to reduce kinetic energy: early avg {early}, late avg {late}"
        );
    }
}
