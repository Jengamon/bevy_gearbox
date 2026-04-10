//! Sugiyama-style automatic layout for state machines.
//!
//! Lays out a (sub)tree of a `StateMachineGraph` so that compound states
//! contain their children with minimal edge crossings, using a layered
//! drawing algorithm:
//!
//! 1. Greedy feedback arc set (Eades) for cycle removal.
//! 2. Longest-path layer assignment on the resulting DAG.
//! 3. Dummy-vertex insertion for edges that span multiple layers.
//! 4. Barycenter crossing minimization (alternating up/down sweeps).
//! 5. Variable-width coordinate assignment.
//!
//! Compound states are handled by recursing post-order: deepest compounds
//! are laid out first, their final size becomes the input size for the
//! enclosing compound's own layout, and so on up to the requested root.
//!
//! Cross-hierarchy edges (transitions whose endpoints are not direct
//! children of the same compound) are "bubbled up" to the level being laid
//! out: each endpoint walks up its ancestor chain until it hits a direct
//! child of the current compound. This matches how the editor's pill
//! parents already work.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy_egui::egui;

use crate::editor::layout::{LayoutConfig, NodeLayout};
use crate::editor::view_model::ViewScene;
use crate::model::StateMachineGraph;
use crate::types::EntityId;

// =============================================================================
// Public API
// =============================================================================

/// Direction of layered flow. State machines almost always read best as LR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutDirection {
    /// Left-to-right (layers are vertical strips, flow goes →).
    LR,
    /// Right-to-left (mirror of LR).
    RL,
    /// Top-to-bottom (layers are horizontal strips, flow goes ↓).
    TB,
    /// Bottom-to-top (mirror of TB).
    BT,
}

/// Tunables for the auto-layout algorithm.
#[derive(Debug, Clone)]
pub struct AutoLayoutConfig {
    pub direction: LayoutDirection,
    /// Gap (world units) between adjacent layers along the flow axis,
    /// in addition to the layer's own thickness.
    pub layer_spacing: f32,
    /// Gap (world units) between adjacent nodes within the same layer,
    /// along the cross axis.
    pub node_spacing: f32,
    /// Number of barycenter sweeps to perform per compound.
    pub barycenter_sweeps: u32,
    /// Extra layer gap (world units) to leave room for edge pills.
    pub reserve_pill_gap: f32,
}

impl Default for AutoLayoutConfig {
    fn default() -> Self {
        Self {
            direction: LayoutDirection::LR,
            layer_spacing: 80.0,
            node_spacing: 32.0,
            barycenter_sweeps: 24,
            reserve_pill_gap: 40.0,
        }
    }
}

/// Diagnostics returned from a layout pass. Mostly useful for tests/logs.
#[derive(Debug, Default, Clone)]
pub struct AutoLayoutReport {
    pub nodes_moved: usize,
    pub compounds_processed: usize,
    pub reversed_edges: usize,
    pub long_edges: usize,
}

/// Run auto-layout over the subtree rooted at `target` and write results
/// back into the `scene`. If `target` is a leaf, lays out its parent's
/// subtree instead. Pills are repositioned to the midpoint of their new
/// source→target segment.
pub(crate) fn auto_layout_subtree(
    scene: &mut ViewScene,
    graph: &StateMachineGraph,
    target: EntityId,
    cfg: &AutoLayoutConfig,
    layout_cfg: &LayoutConfig,
) -> AutoLayoutReport {
    // If the user invoked auto-layout on a leaf, lay out its parent so the
    // surrounding siblings get re-organized — the leaf alone is uninteresting.
    let root = if graph.get_children(&target).is_empty() {
        graph.get_parent(&target).unwrap_or(target)
    } else {
        target
    };

    // Pin the root's current top-left so the user's mental anchor doesn't move.
    let root_min = scene
        .node_rects
        .get(&root)
        .map(|r| r.min)
        .unwrap_or(egui::pos2(0.0, 0.0));

    // Snapshot the input sizes for every node in the subtree.
    let subtree = collect_subtree(graph, root);
    let initial_sizes = snapshot_sizes(scene, &subtree, layout_cfg);

    // Bottom-up: compute new size for each compound and store local positions
    // (relative to that compound's content origin).
    let mut local_positions: HashMap<EntityId, HashMap<EntityId, egui::Pos2>> = HashMap::new();
    let mut new_sizes: HashMap<EntityId, egui::Vec2> = HashMap::new();
    let mut report = AutoLayoutReport::default();

    compute_layout_recursive(
        graph,
        root,
        &initial_sizes,
        cfg,
        layout_cfg,
        &mut local_positions,
        &mut new_sizes,
        &mut report,
    );

    // Top-down: convert local positions to world rects, anchored at root_min.
    let mut new_rects: HashMap<EntityId, egui::Rect> = HashMap::new();
    apply_layout_recursive(
        graph,
        root,
        root_min,
        &new_sizes,
        &local_positions,
        &mut new_rects,
        layout_cfg,
    );

    // Track how many node positions actually changed.
    for (id, new_rect) in new_rects.iter() {
        let old = scene.node_rects.get(id).copied();
        if old.map(|r| r != *new_rect).unwrap_or(true) {
            report.nodes_moved += 1;
        }
    }

    // Write the new node rects back into the scene.
    write_back_node_rects(scene, &new_rects);

    // Reposition any pills whose source/target moved.
    reposition_pills_for_subtree(scene, graph, &subtree);

    // Final pass: use the existing NodeLayout machinery to clamp children
    // into their parent content areas and expand any compound that needs to
    // grow to contain a pill (e.g. a cross-hierarchy edge whose midpoint
    // landed outside its declared pill_parent).
    finalize_with_node_layout(scene, layout_cfg);

    report
}

// =============================================================================
// Subtree collection / size snapshot
// =============================================================================

/// Returns the set of all node EntityIds in the subtree rooted at `root`.
fn collect_subtree(graph: &StateMachineGraph, root: EntityId) -> HashSet<EntityId> {
    let mut out: HashSet<EntityId> = HashSet::new();
    let mut stack: Vec<EntityId> = vec![root];
    while let Some(id) = stack.pop() {
        if !out.insert(id) {
            continue;
        }
        for child in graph.get_children(&id) {
            stack.push(child);
        }
    }
    out
}

/// Snapshot the current rect sizes for every node in the subtree, falling
/// back to `min_node_size` for anything missing.
fn snapshot_sizes(
    scene: &ViewScene,
    subtree: &HashSet<EntityId>,
    layout_cfg: &LayoutConfig,
) -> HashMap<EntityId, egui::Vec2> {
    let mut sizes: HashMap<EntityId, egui::Vec2> = HashMap::new();
    for id in subtree.iter() {
        let sz = scene
            .node_rects
            .get(id)
            .map(|r| r.size())
            .unwrap_or(layout_cfg.min_node_size);
        sizes.insert(*id, sz);
    }
    sizes
}

// =============================================================================
// Compound recursion
// =============================================================================

/// Returns the new size of `node` after laying out its descendants. For
/// leaves this is just `max(initial_size, min_node_size)`. For compounds
/// this is the bounding box of children plus padding plus header.
fn compute_layout_recursive(
    graph: &StateMachineGraph,
    node: EntityId,
    initial_sizes: &HashMap<EntityId, egui::Vec2>,
    cfg: &AutoLayoutConfig,
    layout_cfg: &LayoutConfig,
    local_positions: &mut HashMap<EntityId, HashMap<EntityId, egui::Pos2>>,
    new_sizes: &mut HashMap<EntityId, egui::Vec2>,
    report: &mut AutoLayoutReport,
) -> egui::Vec2 {
    let children = graph.get_children(&node);
    if children.is_empty() {
        let raw = initial_sizes
            .get(&node)
            .copied()
            .unwrap_or(layout_cfg.min_node_size);
        let size = egui::vec2(
            raw.x.max(layout_cfg.min_node_size.x),
            raw.y.max(layout_cfg.min_node_size.y),
        );
        new_sizes.insert(node, size);
        return size;
    }

    // Recurse into children first so their sizes are finalized.
    let mut child_sizes: HashMap<EntityId, egui::Vec2> = HashMap::new();
    for child in children.iter() {
        let s = compute_layout_recursive(
            graph,
            *child,
            initial_sizes,
            cfg,
            layout_cfg,
            local_positions,
            new_sizes,
            report,
        );
        child_sizes.insert(*child, s);
    }

    // Run Sugiyama on the direct children of this compound.
    let positions = run_sugiyama_for_compound(
        graph,
        node,
        &children,
        &child_sizes,
        cfg,
        layout_cfg,
        report,
    );

    // Compute compound's new size from the child extents.
    let pad = layout_cfg.content_padding;
    let header_h = layout_cfg.header_height_world;
    let mut max_extent = egui::vec2(0.0, 0.0);
    for child in children.iter() {
        let pos = positions.get(child).copied().unwrap_or(egui::pos2(0.0, 0.0));
        let sz = child_sizes
            .get(child)
            .copied()
            .unwrap_or(layout_cfg.min_node_size);
        max_extent.x = max_extent.x.max(pos.x + sz.x);
        max_extent.y = max_extent.y.max(pos.y + sz.y);
    }
    let size = egui::vec2(
        (2.0 * pad.x + max_extent.x).max(layout_cfg.min_node_size.x),
        (2.0 * pad.y + header_h + max_extent.y).max(layout_cfg.min_node_size.y),
    );

    local_positions.insert(node, positions);
    new_sizes.insert(node, size);
    report.compounds_processed += 1;
    size
}

/// Top-down pass: takes the locally-stored positions and resolves them into
/// absolute world rects starting from `root_min`.
fn apply_layout_recursive(
    graph: &StateMachineGraph,
    node: EntityId,
    node_min: egui::Pos2,
    new_sizes: &HashMap<EntityId, egui::Vec2>,
    local_positions: &HashMap<EntityId, HashMap<EntityId, egui::Pos2>>,
    out_rects: &mut HashMap<EntityId, egui::Rect>,
    layout_cfg: &LayoutConfig,
) {
    let size = new_sizes
        .get(&node)
        .copied()
        .unwrap_or(layout_cfg.min_node_size);
    out_rects.insert(node, egui::Rect::from_min_size(node_min, size));

    let children = graph.get_children(&node);
    if children.is_empty() {
        return;
    }

    let pad = layout_cfg.content_padding;
    let header_h = layout_cfg.header_height_world;
    let content_origin = egui::pos2(node_min.x + pad.x, node_min.y + pad.y + header_h);

    if let Some(positions) = local_positions.get(&node) {
        for child in children.iter() {
            let local = positions
                .get(child)
                .copied()
                .unwrap_or(egui::pos2(0.0, 0.0));
            let child_min = egui::pos2(content_origin.x + local.x, content_origin.y + local.y);
            apply_layout_recursive(
                graph,
                *child,
                child_min,
                new_sizes,
                local_positions,
                out_rects,
                layout_cfg,
            );
        }
    }
}

/// Write the freshly computed rects into the scene's `node_rects`,
/// `states[*].rect`, and `edges[*].rect` mirrors. Pills are excluded — they
/// are repositioned in a separate pass.
fn write_back_node_rects(scene: &mut ViewScene, new_rects: &HashMap<EntityId, egui::Rect>) {
    for (id, rect) in new_rects.iter() {
        if let Some(r) = scene.node_rects.get_mut(id) {
            *r = *rect;
        }
        if let Some(sv) = scene.states.get_mut(id) {
            sv.rect = *rect;
        }
        // Edges are repositioned separately (their rects are pill rects, not
        // node rects); auto-layout never produces an entry for an edge id here.
    }
}

// =============================================================================
// Pill repositioning
// =============================================================================

/// After nodes have been moved, reposition every pill whose source or target
/// lies inside `subtree`. The placement strategy depends on the structural
/// relationship between source and target:
///
/// - **Self-loop**: tucked just below the host node.
/// - **Containment** (one endpoint is the immediate parent of the other):
///   placed on the side of the inner (child) rect *farthest* from the outer
///   (parent) rect's centroid. This avoids the case where the midpoint of
///   the two centers falls inside the parent and the renderer routes the
///   arrow back across the entire parent body.
/// - **Sibling/cross-hierarchy**: midpoint of the *border-to-border* visible
///   segment, computed from each rect's boundary point along the line to
///   the other rect's center. This is robust against one rect being much
///   larger than the other (the naive center-to-center midpoint can fall
///   inside the larger rect).
fn reposition_pills_for_subtree(
    scene: &mut ViewScene,
    graph: &StateMachineGraph,
    subtree: &HashSet<EntityId>,
) {
    let edge_ids: Vec<EntityId> = scene
        .edges
        .iter()
        .filter(|(_, ev)| subtree.contains(&ev.source) || subtree.contains(&ev.target))
        .map(|(id, _)| *id)
        .collect();

    for eid in edge_ids {
        let (src, dst) = match scene.edges.get(&eid) {
            Some(ev) => (ev.source, ev.target),
            None => continue,
        };
        let src_rect = match scene.node_rects.get(&src).copied() {
            Some(r) => r,
            None => continue,
        };
        let dst_rect = match scene.node_rects.get(&dst).copied() {
            Some(r) => r,
            None => continue,
        };

        let new_center = if src == dst {
            // Self-loop: tuck the pill just below the node.
            egui::pos2(src_rect.center().x, src_rect.max.y + 32.0)
        } else if graph.get_parent(&dst) == Some(src) {
            // src is the parent of dst — outer = src, inner = dst.
            place_pill_on_far_side(dst_rect, src_rect)
        } else if graph.get_parent(&src) == Some(dst) {
            // dst is the parent of src — outer = dst, inner = src.
            place_pill_on_far_side(src_rect, dst_rect)
        } else {
            // Sibling / cross-hierarchy: midpoint of the visible
            // border-to-border segment.
            let from_src = rect_border_toward(src_rect, dst_rect.center());
            let from_dst = rect_border_toward(dst_rect, src_rect.center());
            egui::pos2(
                (from_src.x + from_dst.x) * 0.5,
                (from_src.y + from_dst.y) * 0.5,
            )
        };

        let old_size = scene
            .edges
            .get(&eid)
            .map(|ev| ev.rect.size())
            .unwrap_or(egui::vec2(80.0, 24.0));
        let new_rect = egui::Rect::from_center_size(new_center, old_size);
        if let Some(r) = scene.node_rects.get_mut(&eid) {
            *r = new_rect;
        }
        if let Some(ev) = scene.edges.get_mut(&eid) {
            ev.rect = new_rect;
        }
    }
}

/// Returns the point on `rect`'s boundary along the ray from `rect.center()`
/// toward `target`. If `target` coincides with the center, returns the center.
fn rect_border_toward(rect: egui::Rect, target: egui::Pos2) -> egui::Pos2 {
    let c = rect.center();
    let dir = target - c;
    if dir.length_sq() < 1e-6 {
        return c;
    }
    let half = rect.size() * 0.5;
    let tx = if dir.x.abs() > 1e-6 {
        half.x / dir.x.abs()
    } else {
        f32::INFINITY
    };
    let ty = if dir.y.abs() > 1e-6 {
        half.y / dir.y.abs()
    } else {
        f32::INFINITY
    };
    let t = tx.min(ty);
    c + dir * t
}

/// Place a pill on the side of `inner` farthest from `outer`'s centroid,
/// padded slightly outside `inner`. Used for transitions where one endpoint
/// contains the other (parent ↔ child), so the rendered arrow leaves the
/// inner node away from the outer's body instead of cutting back through it.
fn place_pill_on_far_side(inner: egui::Rect, outer: egui::Rect) -> egui::Pos2 {
    const PAD: f32 = 24.0;
    let inner_c = inner.center();
    let outer_c = outer.center();
    let raw_dir = inner_c - outer_c;
    let unit_dir = if raw_dir.length_sq() < 1e-6 {
        // Inner sits exactly at outer's center — no preferred direction;
        // fall back to "below the inner".
        egui::vec2(0.0, 1.0)
    } else {
        raw_dir / raw_dir.length()
    };
    // Distance from inner_center to its boundary along `unit_dir`.
    let half = inner.size() * 0.5;
    let tx = if unit_dir.x.abs() > 1e-6 {
        half.x / unit_dir.x.abs()
    } else {
        f32::INFINITY
    };
    let ty = if unit_dir.y.abs() > 1e-6 {
        half.y / unit_dir.y.abs()
    } else {
        f32::INFINITY
    };
    let t = tx.min(ty);
    let boundary = inner_c + unit_dir * t;
    boundary + unit_dir * PAD
}

/// Run the existing `NodeLayout::clamp_children_left_top` and
/// `fit_parents_to_children` passes over the entire scene so that:
///   - children are clamped into their parents' content areas, and
///   - compounds expand to contain any pill whose midpoint landed outside
///     its declared `pill_parent`.
///
/// This is a safety net for the cross-hierarchy edge case; for fully
/// internal layouts it is a no-op.
fn finalize_with_node_layout(scene: &mut ViewScene, layout_cfg: &LayoutConfig) {
    // Build a NodeLayout view from the scene's tree.
    let mut nl = NodeLayout {
        node_rects: scene.node_rects.clone(),
        parent_of: scene.tree.parent_of.clone(),
        children_of: scene.tree.children_of.clone(),
        container_nodes: scene.tree.containers.clone(),
        root: None,
        draw_order_nodes: Vec::new(),
    };

    // Build attachments_by_parent from edge pills.
    let mut attachments: HashMap<EntityId, Vec<egui::Rect>> = HashMap::new();
    for (eid, ev) in scene.edges.iter() {
        if let Some(pid) = ev.pill_parent {
            if let Some(rect) = scene.node_rects.get(eid).copied() {
                attachments.entry(pid).or_default().push(rect);
            }
        }
    }

    nl.clamp_children_left_top(layout_cfg);
    nl.fit_parents_to_children(layout_cfg, Some(&attachments));

    // Write back any rects that the cleanup pass changed.
    for (id, rect) in nl.node_rects.iter() {
        let changed = scene
            .node_rects
            .get(id)
            .map(|cur| *cur != *rect)
            .unwrap_or(true);
        if changed {
            if let Some(r) = scene.node_rects.get_mut(id) {
                *r = *rect;
            }
            if let Some(sv) = scene.states.get_mut(id) {
                sv.rect = *rect;
            }
            if let Some(ev) = scene.edges.get_mut(id) {
                ev.rect = *rect;
            }
        }
    }
}

// =============================================================================
// Sugiyama core (one compound at a time)
// =============================================================================

/// Run the four Sugiyama phases on the direct children of `compound`.
/// Returns local positions (top-left, relative to the compound's content
/// origin) for each child.
fn run_sugiyama_for_compound(
    graph: &StateMachineGraph,
    compound: EntityId,
    children: &[EntityId],
    child_sizes: &HashMap<EntityId, egui::Vec2>,
    cfg: &AutoLayoutConfig,
    layout_cfg: &LayoutConfig,
    report: &mut AutoLayoutReport,
) -> HashMap<EntityId, egui::Pos2> {
    let n = children.len();
    if n == 0 {
        return HashMap::new();
    }
    if n == 1 {
        // Single child: just place at (0, 0).
        let mut out = HashMap::new();
        out.insert(children[0], egui::pos2(0.0, 0.0));
        return out;
    }

    // Phase 0: collect candidate edges (after bubble-up).
    let mut node_idx: HashMap<EntityId, usize> = HashMap::new();
    for (i, c) in children.iter().enumerate() {
        node_idx.insert(*c, i);
    }
    let direct_children: HashSet<EntityId> = children.iter().copied().collect();

    let mut raw_edges: Vec<(usize, usize)> = Vec::new();
    let mut seen_pairs: HashSet<(usize, usize)> = HashSet::new();
    for (_eid, edge) in graph.edges.iter() {
        let pair = bubble_edge_to_level(graph, compound, &direct_children, edge.source, edge.target);
        if let Some((src_rep, dst_rep)) = pair {
            if src_rep == dst_rep {
                continue;
            }
            let si = node_idx[&src_rep];
            let di = node_idx[&dst_rep];
            if seen_pairs.insert((si, di)) {
                raw_edges.push((si, di));
            }
        }
    }

    // Phase 1: cycle removal.
    let reversed = greedy_fas(n, &raw_edges);
    let oriented: Vec<(usize, usize)> = raw_edges
        .iter()
        .enumerate()
        .map(|(i, &(s, d))| if reversed[i] { (d, s) } else { (s, d) })
        .collect();
    report.reversed_edges += reversed.iter().filter(|x| **x).count();

    // Phase 2: longest-path layer assignment.
    let layers = assign_layers(n, &oriented);

    // Phase 2b: insert dummy vertices for long edges.
    let mut lg = build_layered_graph(n, &layers, &oriented, report);

    // Phase 3: barycenter crossing minimization.
    barycenter_sweeps(&mut lg, cfg.barycenter_sweeps);

    // Phase 4: coordinate assignment.
    assign_coordinates(&lg, children, child_sizes, cfg, layout_cfg)
}

// =============================================================================
// Bubble-up rule
// =============================================================================

/// Walk both endpoints up the parent chain until they land on direct
/// children of `level`. Returns `None` if the edge is not relevant to this
/// level (e.g., both endpoints lie outside the subtree, or the edge is at a
/// higher level entirely).
fn bubble_edge_to_level(
    graph: &StateMachineGraph,
    _level: EntityId,
    direct_children: &HashSet<EntityId>,
    src: EntityId,
    dst: EntityId,
) -> Option<(EntityId, EntityId)> {
    let s = bubble_up(graph, direct_children, src)?;
    let d = bubble_up(graph, direct_children, dst)?;
    Some((s, d))
}

fn bubble_up(
    graph: &StateMachineGraph,
    direct_children: &HashSet<EntityId>,
    start: EntityId,
) -> Option<EntityId> {
    let mut node = start;
    loop {
        if direct_children.contains(&node) {
            return Some(node);
        }
        match graph.get_parent(&node) {
            Some(p) => node = p,
            None => return None,
        }
    }
}

// =============================================================================
// Phase 1: Greedy feedback arc set (Eades, Lin, Smyth)
// =============================================================================

/// Returns a `reversed` flag per input edge. Reversed edges are oriented
/// against their layering direction so the layered graph becomes a DAG;
/// the rendered graph keeps the original direction.
fn greedy_fas(n: usize, edges: &[(usize, usize)]) -> Vec<bool> {
    let m = edges.len();
    let mut reversed = vec![false; m];
    if n == 0 || m == 0 {
        return reversed;
    }

    // Adjacency: edge indices, not vertex indices, so we can update degrees.
    let mut out_eidx: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_eidx: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (ei, &(s, d)) in edges.iter().enumerate() {
        if s == d {
            continue;
        }
        out_eidx[s].push(ei);
        in_eidx[d].push(ei);
    }

    let mut alive = vec![true; n];
    let mut in_deg: Vec<i32> = (0..n).map(|v| in_eidx[v].len() as i32).collect();
    let mut out_deg: Vec<i32> = (0..n).map(|v| out_eidx[v].len() as i32).collect();

    let mut s1: Vec<usize> = Vec::new();
    let mut s2: Vec<usize> = Vec::new();
    let mut remaining = n;

    while remaining > 0 {
        // Strip sinks (out_deg == 0). They're "free" — no edges leave them,
        // so they always go at the end of the order.
        let mut changed = true;
        while changed {
            changed = false;
            for v in 0..n {
                if alive[v] && out_deg[v] == 0 {
                    s2.push(v);
                    remove_vertex(v, &mut alive, &mut in_deg, &mut out_deg, &out_eidx, &in_eidx, edges);
                    remaining -= 1;
                    changed = true;
                }
            }
        }
        // Strip sources (in_deg == 0). Same idea, but at the front.
        changed = true;
        while changed {
            changed = false;
            for v in 0..n {
                if alive[v] && in_deg[v] == 0 {
                    s1.push(v);
                    remove_vertex(v, &mut alive, &mut in_deg, &mut out_deg, &out_eidx, &in_eidx, edges);
                    remaining -= 1;
                    changed = true;
                }
            }
        }
        // Of what remains, pick the vertex with the highest (out_deg - in_deg)
        // and push it to the front; ties are broken by index for determinism.
        if remaining > 0 {
            let mut best: Option<(usize, i32)> = None;
            for v in 0..n {
                if alive[v] {
                    let diff = out_deg[v] - in_deg[v];
                    match best {
                        None => best = Some((v, diff)),
                        Some((_, b)) if diff > b => best = Some((v, diff)),
                        _ => {}
                    }
                }
            }
            if let Some((v, _)) = best {
                s1.push(v);
                remove_vertex(v, &mut alive, &mut in_deg, &mut out_deg, &out_eidx, &in_eidx, edges);
                remaining -= 1;
            }
        }
    }

    let mut order: Vec<usize> = s1;
    s2.reverse();
    order.extend(s2);

    let mut pos: Vec<usize> = vec![0; n];
    for (i, &v) in order.iter().enumerate() {
        pos[v] = i;
    }
    for (i, &(s, d)) in edges.iter().enumerate() {
        if s == d {
            continue;
        }
        if pos[s] > pos[d] {
            reversed[i] = true;
        }
    }
    reversed
}

fn remove_vertex(
    v: usize,
    alive: &mut [bool],
    in_deg: &mut [i32],
    out_deg: &mut [i32],
    out_eidx: &[Vec<usize>],
    in_eidx: &[Vec<usize>],
    edges: &[(usize, usize)],
) {
    alive[v] = false;
    for &ei in &out_eidx[v] {
        let d = edges[ei].1;
        if alive[d] {
            in_deg[d] -= 1;
        }
    }
    for &ei in &in_eidx[v] {
        let s = edges[ei].0;
        if alive[s] {
            out_deg[s] -= 1;
        }
    }
}

// =============================================================================
// Phase 2: Longest-path layer assignment
// =============================================================================

/// Assigns each vertex its earliest possible layer (longest path from any
/// source). Disconnected components and isolated nodes get layer 0.
fn assign_layers(n: usize, oriented_edges: &[(usize, usize)]) -> Vec<usize> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_count: Vec<usize> = vec![0; n];
    for &(s, d) in oriented_edges {
        if s == d {
            continue;
        }
        out[s].push(d);
        in_count[d] += 1;
    }
    // Kahn's topological sort.
    let mut deg = in_count.clone();
    let mut queue: VecDeque<usize> = VecDeque::new();
    for v in 0..n {
        if deg[v] == 0 {
            queue.push_back(v);
        }
    }
    let mut topo: Vec<usize> = Vec::with_capacity(n);
    while let Some(v) = queue.pop_front() {
        topo.push(v);
        for &d in &out[v] {
            deg[d] -= 1;
            if deg[d] == 0 {
                queue.push_back(d);
            }
        }
    }
    // Defensive: if topo is incomplete (cycle leaked through), append leftover
    // vertices in index order so we still produce something.
    if topo.len() < n {
        for v in 0..n {
            if !topo.contains(&v) {
                topo.push(v);
            }
        }
    }

    let mut layers = vec![0usize; n];
    for &v in &topo {
        for &d in &out[v] {
            if layers[d] < layers[v] + 1 {
                layers[d] = layers[v] + 1;
            }
        }
    }
    layers
}

// =============================================================================
// Phase 2b: Layered graph construction with dummy vertices
// =============================================================================

/// A layered graph after dummy insertion. Vertex indices `[0, n_real)` map to
/// real nodes (children of the compound being laid out); indices `[n_real, …)`
/// are dummies introduced to span long edges.
struct LayeredGraph {
    n_real: usize,
    /// `layers[layer_idx]` is a list of vertex indices, in their current order.
    layers: Vec<Vec<usize>>,
    /// Predecessor list (one layer up) for each vertex.
    pred: Vec<Vec<usize>>,
    /// Successor list (one layer down) for each vertex.
    succ: Vec<Vec<usize>>,
}

fn build_layered_graph(
    n_real: usize,
    real_layers: &[usize],
    oriented_edges: &[(usize, usize)],
    report: &mut AutoLayoutReport,
) -> LayeredGraph {
    let max_layer = *real_layers.iter().max().unwrap_or(&0);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_layer + 1];
    let mut layer_of: Vec<usize> = real_layers.to_vec();
    for v in 0..n_real {
        layers[real_layers[v]].push(v);
    }

    let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n_real];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n_real];

    for &(s, d) in oriented_edges {
        if s == d {
            continue;
        }
        let ls = real_layers[s];
        let ld = real_layers[d];
        if ld <= ls {
            // Should not happen after cycle removal; ignore defensively.
            continue;
        }
        if ld - ls == 1 {
            succ[s].push(d);
            pred[d].push(s);
        } else {
            // Insert one dummy per intermediate layer and chain them.
            report.long_edges += 1;
            let mut prev_v = s;
            for layer in (ls + 1)..ld {
                let dummy_idx = layer_of.len();
                layer_of.push(layer);
                layers[layer].push(dummy_idx);
                succ.push(Vec::new());
                pred.push(Vec::new());
                succ[prev_v].push(dummy_idx);
                pred[dummy_idx].push(prev_v);
                prev_v = dummy_idx;
            }
            succ[prev_v].push(d);
            pred[d].push(prev_v);
        }
    }

    LayeredGraph {
        n_real,
        layers,
        pred,
        succ,
    }
}

// =============================================================================
// Phase 3: Barycenter crossing minimization
// =============================================================================

fn barycenter_sweeps(lg: &mut LayeredGraph, sweeps: u32) {
    let n_layers = lg.layers.len();
    if n_layers <= 1 {
        return;
    }
    for _ in 0..sweeps {
        // Down sweep: each layer is reordered based on its predecessors.
        for li in 1..n_layers {
            reorder_layer_by_neighbors(lg, li, true);
        }
        // Up sweep: each layer is reordered based on its successors.
        for li in (0..n_layers - 1).rev() {
            reorder_layer_by_neighbors(lg, li, false);
        }
    }
}

/// Reorder `layers[li]` so that vertices appear sorted by the average index
/// of their neighbors in the adjacent layer. `use_pred = true` looks at
/// layer-1 predecessors; `false` looks at layer+1 successors.
fn reorder_layer_by_neighbors(lg: &mut LayeredGraph, li: usize, use_pred: bool) {
    let neighbor_layer_idx = if use_pred {
        if li == 0 {
            return;
        }
        li - 1
    } else {
        if li + 1 >= lg.layers.len() {
            return;
        }
        li + 1
    };

    // Position lookup for the neighbor layer.
    let neighbor_layer = &lg.layers[neighbor_layer_idx];
    let mut pos_of: HashMap<usize, usize> = HashMap::with_capacity(neighbor_layer.len());
    for (i, &v) in neighbor_layer.iter().enumerate() {
        pos_of.insert(v, i);
    }

    let mut keyed: Vec<(usize, f64, usize)> = lg.layers[li]
        .iter()
        .enumerate()
        .map(|(cur_pos, &v)| {
            let neighbors: &[usize] = if use_pred { &lg.pred[v] } else { &lg.succ[v] };
            let mut sum = 0.0f64;
            let mut cnt = 0usize;
            for nb in neighbors {
                if let Some(p) = pos_of.get(nb) {
                    sum += *p as f64;
                    cnt += 1;
                }
            }
            // No neighbors → preserve current position by using it as the key.
            let bary = if cnt > 0 {
                sum / cnt as f64
            } else {
                cur_pos as f64
            };
            (v, bary, cur_pos)
        })
        .collect();
    // Stable sort: ties broken by previous index, which preserves locality.
    keyed.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.2.cmp(&b.2))
    });
    lg.layers[li] = keyed.into_iter().map(|(v, _, _)| v).collect();
}

// =============================================================================
// Phase 4: Coordinate assignment (variable-width packing)
// =============================================================================

fn assign_coordinates(
    lg: &LayeredGraph,
    children: &[EntityId],
    child_sizes: &HashMap<EntityId, egui::Vec2>,
    cfg: &AutoLayoutConfig,
    layout_cfg: &LayoutConfig,
) -> HashMap<EntityId, egui::Pos2> {
    let n_layers = lg.layers.len();
    if n_layers == 0 {
        return HashMap::new();
    }

    let is_horizontal = matches!(cfg.direction, LayoutDirection::LR | LayoutDirection::RL);

    // Step 1: per-layer thickness along the flow axis = max(t-dim) of any
    // real node in that layer. Dummies don't contribute thickness.
    let mut layer_thickness: Vec<f32> = vec![0.0; n_layers];
    for (li, layer) in lg.layers.iter().enumerate() {
        let mut max_t = 0.0f32;
        for &v in layer {
            if v < lg.n_real {
                let id = children[v];
                let sz = child_sizes
                    .get(&id)
                    .copied()
                    .unwrap_or(layout_cfg.min_node_size);
                let t = if is_horizontal { sz.x } else { sz.y };
                if t > max_t {
                    max_t = t;
                }
            }
        }
        layer_thickness[li] = max_t.max(0.0);
    }

    // Step 2: t-coordinate of each layer's centerline.
    let inter_layer_gap = cfg.layer_spacing + cfg.reserve_pill_gap;
    let mut layer_t_center = vec![0.0f32; n_layers];
    let mut t_cursor = 0.0f32;
    for li in 0..n_layers {
        t_cursor += layer_thickness[li] * 0.5;
        layer_t_center[li] = t_cursor;
        t_cursor += layer_thickness[li] * 0.5 + inter_layer_gap;
    }
    let total_t_extent = if n_layers == 0 {
        0.0
    } else {
        layer_t_center[n_layers - 1] + layer_thickness[n_layers - 1] * 0.5
    };

    // Step 3: pack each layer along the cross axis. Real nodes contribute
    // their cross-axis dimension; dummies contribute zero (they only matter
    // for ordering during barycenter, not for spacing here).
    let mut s_min_of: HashMap<usize, f32> = HashMap::new();
    let mut layer_s_extent: Vec<f32> = vec![0.0; n_layers];
    for (li, layer) in lg.layers.iter().enumerate() {
        let mut s = 0.0f32;
        let mut placed_real = false;
        for &v in layer {
            if v < lg.n_real {
                let id = children[v];
                let sz = child_sizes
                    .get(&id)
                    .copied()
                    .unwrap_or(layout_cfg.min_node_size);
                let s_dim = if is_horizontal { sz.y } else { sz.x };
                if placed_real {
                    s += cfg.node_spacing;
                }
                s_min_of.insert(v, s);
                s += s_dim;
                placed_real = true;
            } else {
                // Dummy: anchor at current s without consuming space.
                s_min_of.insert(v, s);
            }
        }
        layer_s_extent[li] = s;
    }

    // Step 4: convert (t, s) into world coordinates per direction.
    let mut out: HashMap<EntityId, egui::Pos2> = HashMap::new();
    for (li, layer) in lg.layers.iter().enumerate() {
        for &v in layer {
            if v >= lg.n_real {
                continue;
            }
            let id = children[v];
            let sz = child_sizes
                .get(&id)
                .copied()
                .unwrap_or(layout_cfg.min_node_size);
            let (t_dim, _s_dim) = if is_horizontal {
                (sz.x, sz.y)
            } else {
                (sz.y, sz.x)
            };
            let t_min = layer_t_center[li] - t_dim * 0.5;
            let s_min = *s_min_of.get(&v).unwrap_or(&0.0);
            let pos = match cfg.direction {
                LayoutDirection::LR => egui::pos2(t_min, s_min),
                LayoutDirection::RL => egui::pos2(total_t_extent - t_min - t_dim, s_min),
                LayoutDirection::TB => egui::pos2(s_min, t_min),
                LayoutDirection::BT => egui::pos2(s_min, total_t_extent - t_min - t_dim),
            };
            out.insert(id, pos);
        }
    }

    out
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pair(s: usize, d: usize) -> (usize, usize) {
        (s, d)
    }

    #[test]
    fn greedy_fas_breaks_simple_cycle() {
        // A -> B -> C -> A: exactly one edge must be reversed.
        let edges = vec![make_pair(0, 1), make_pair(1, 2), make_pair(2, 0)];
        let reversed = greedy_fas(3, &edges);
        let count = reversed.iter().filter(|x| **x).count();
        assert_eq!(count, 1, "exactly one edge should be reversed for a 3-cycle");
    }

    #[test]
    fn greedy_fas_acyclic_reverses_nothing() {
        // A -> B -> C, A -> C: already a DAG.
        let edges = vec![make_pair(0, 1), make_pair(1, 2), make_pair(0, 2)];
        let reversed = greedy_fas(3, &edges);
        assert!(reversed.iter().all(|x| !*x));
    }

    #[test]
    fn longest_path_diamond() {
        // A -> B, A -> C, B -> D, C -> D
        let edges = vec![
            make_pair(0, 1),
            make_pair(0, 2),
            make_pair(1, 3),
            make_pair(2, 3),
        ];
        let layers = assign_layers(4, &edges);
        assert_eq!(layers[0], 0);
        assert_eq!(layers[1], 1);
        assert_eq!(layers[2], 1);
        assert_eq!(layers[3], 2);
    }

    #[test]
    fn longest_path_chain_with_shortcut() {
        // A -> B -> C -> D, plus A -> D directly. A->D is a long edge.
        let edges = vec![
            make_pair(0, 1),
            make_pair(1, 2),
            make_pair(2, 3),
            make_pair(0, 3),
        ];
        let layers = assign_layers(4, &edges);
        assert_eq!(layers[0], 0);
        assert_eq!(layers[1], 1);
        assert_eq!(layers[2], 2);
        assert_eq!(layers[3], 3);
    }

    #[test]
    fn isolated_nodes_get_layer_zero() {
        let edges: Vec<(usize, usize)> = vec![];
        let layers = assign_layers(4, &edges);
        for l in layers {
            assert_eq!(l, 0);
        }
    }

    #[test]
    fn dummy_insertion_for_long_edges() {
        // 0 -> 3 spanning layers 0..3, plus 0->1->2->3
        let real_layers = vec![0, 1, 2, 3];
        let edges = vec![
            make_pair(0, 1),
            make_pair(1, 2),
            make_pair(2, 3),
            make_pair(0, 3),
        ];
        let mut report = AutoLayoutReport::default();
        let lg = build_layered_graph(4, &real_layers, &edges, &mut report);
        // 4 layers total
        assert_eq!(lg.layers.len(), 4);
        assert_eq!(report.long_edges, 1, "the 0->3 edge should be marked long");
        // The long edge spans layers 0..3, so 2 dummies inserted (in layers 1 and 2).
        assert_eq!(lg.layers[1].len(), 2, "layer 1 has node 1 + dummy");
        assert_eq!(lg.layers[2].len(), 2, "layer 2 has node 2 + dummy");
    }

    #[test]
    fn barycenter_reduces_or_keeps_crossings() {
        // Simple K2,2 in two layers. Barycenter should produce a stable order.
        // Layer 0: [0, 1]; Layer 1: [2, 3]; edges: 0->3, 1->2 (crossed)
        let real_layers = vec![0, 0, 1, 1];
        let edges = vec![make_pair(0, 3), make_pair(1, 2)];
        let mut report = AutoLayoutReport::default();
        let mut lg = build_layered_graph(4, &real_layers, &edges, &mut report);
        // Force the crossed initial order so barycenter has work to do.
        lg.layers[0] = vec![0, 1];
        lg.layers[1] = vec![2, 3];
        let crossings_before = count_crossings_for_test(&lg);
        barycenter_sweeps(&mut lg, 8);
        let crossings_after = count_crossings_for_test(&lg);
        assert!(
            crossings_after <= crossings_before,
            "barycenter should not increase crossings ({} -> {})",
            crossings_before,
            crossings_after
        );
        // For a single K2,2 it should be solvable to 0.
        assert_eq!(crossings_after, 0, "K2,2 should resolve to zero crossings");
    }

    /// Naive O(E^2) bilayer crossing count, used only by tests.
    fn count_crossings_for_test(lg: &LayeredGraph) -> u32 {
        let mut total = 0u32;
        for li in 0..lg.layers.len().saturating_sub(1) {
            let upper = &lg.layers[li];
            let lower = &lg.layers[li + 1];
            let mut upper_pos: HashMap<usize, usize> = HashMap::new();
            let mut lower_pos: HashMap<usize, usize> = HashMap::new();
            for (i, &v) in upper.iter().enumerate() {
                upper_pos.insert(v, i);
            }
            for (i, &v) in lower.iter().enumerate() {
                lower_pos.insert(v, i);
            }
            // Collect (upper_pos, lower_pos) pairs for each edge crossing
            // this gap.
            let mut pairs: Vec<(usize, usize)> = Vec::new();
            for (vi, _) in upper.iter().enumerate() {
                let v = upper[vi];
                for s in &lg.succ[v] {
                    if let Some(lp) = lower_pos.get(s) {
                        pairs.push((vi, *lp));
                    }
                }
            }
            for i in 0..pairs.len() {
                for j in (i + 1)..pairs.len() {
                    let (a1, b1) = pairs[i];
                    let (a2, b2) = pairs[j];
                    if (a1 < a2 && b1 > b2) || (a1 > a2 && b1 < b2) {
                        total += 1;
                    }
                }
            }
        }
        total
    }

    #[test]
    fn coords_no_overlap_within_layer_lr() {
        // Two layers, two real nodes per layer, no edges. Each node has the
        // default size; verify that within-layer s-coordinates are spaced.
        let real_layers = vec![0, 0, 1];
        let edges: Vec<(usize, usize)> = vec![];
        let mut report = AutoLayoutReport::default();
        let lg = build_layered_graph(3, &real_layers, &edges, &mut report);

        let children = vec![EntityId(10), EntityId(20), EntityId(30)];
        let mut sizes = HashMap::new();
        sizes.insert(EntityId(10), egui::vec2(140.0, 60.0));
        sizes.insert(EntityId(20), egui::vec2(140.0, 60.0));
        sizes.insert(EntityId(30), egui::vec2(140.0, 60.0));

        let cfg = AutoLayoutConfig::default();
        let layout_cfg = LayoutConfig::default();
        let positions = assign_coordinates(&lg, &children, &sizes, &cfg, &layout_cfg);

        // Both layer-0 nodes should have y > the other's y + 60 - 32 (separated).
        let p10 = positions[&EntityId(10)];
        let p20 = positions[&EntityId(20)];
        let dy = (p10.y - p20.y).abs();
        assert!(
            dy >= 60.0,
            "layer-0 nodes should not overlap vertically (dy = {})",
            dy
        );

        // Layers should be horizontally separated for LR.
        let p30 = positions[&EntityId(30)];
        let dx_layer = p30.x - p10.x;
        assert!(
            dx_layer > 0.0,
            "layer 1 should be to the right of layer 0 (dx = {})",
            dx_layer
        );
    }
}
