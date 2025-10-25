use bevy::prelude::*;
use bevy_egui::egui;
use crate::model::{StateMachineGraph, NodeId, EdgeId};
use crate::component as c;
use super::view_model::{GraphDoc, UiNode, UiNodeKind, UiEdge, EdgePill};

/// Merge a fresh snapshot into an existing GraphDoc, preserving layout where possible
pub fn project_graph_into_doc(doc: &mut GraphDoc, snapshot: StateMachineGraph) {
    // Preserve existing rects and pill offsets by id
    let mut preserved_nodes = std::mem::take(&mut doc.node_views);
    let mut preserved_edges = std::mem::take(&mut doc.edge_views);

    // Rebuild nodes with preserved rects where available
    let mut node_views = std::collections::HashMap::new();
    for (id, node) in snapshot.nodes.iter() {
        let rect = preserved_nodes
            .remove(id)
            .map(|n| n.rect)
            .unwrap_or_else(|| egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(140.0, 60.0)));
        let label = node
            .display_name
            .clone()
            .unwrap_or_else(|| format!("{}", id.0));
        // Derive kind from components; Parallel is a parent as well
        let has_parallel = node.components.keys().any(|k| k == c::PARALLEL || k.ends_with("::Parallel") || k.ends_with("::Parallel>"));
        let is_container = !node.children.is_empty();
        let kind = if has_parallel { UiNodeKind::Parallel } else if is_container { UiNodeKind::Parent } else { UiNodeKind::Leaf };
        node_views.insert(*id, UiNode { id: *id, rect, kind, label, is_container });
    }

    // Deterministic layout for nodes lacking a rect (at origin). Use DFS grid/stack.
    apply_initial_layout_for_unseen_nodes(&snapshot, &mut node_views);

    // Rebuild edges
    let mut edge_views = std::collections::HashMap::new();
    for (eid, edge) in snapshot.edges.iter() {
        let preserved = preserved_edges.remove(eid);
        let midpoint = {
            let s = node_views.get(&edge.source).map(|n| n.rect.center()).unwrap_or(egui::pos2(0.0, 0.0));
            let t = node_views.get(&edge.target).map(|n| n.rect.center()).unwrap_or(egui::pos2(0.0, 0.0));
            egui::pos2((s.x + t.x) * 0.5, (s.y + t.y) * 0.5)
        };
        let pill = if let Some(prev) = preserved.as_ref() { prev.pill.clone() } else { EdgePill { pos: midpoint, offset_from_midpoint: egui::Vec2::ZERO, dragging: false } };
        let label = edge.display_label.clone().unwrap_or_else(|| "Edge".to_string());
        edge_views.insert(*eid, UiEdge { id: *eid, source: edge.source, target: edge.target, label, pill });
    }

    // Compute deterministic draw orders
    let (node_order, edge_order) = compute_draw_orders(&snapshot);

    doc.graph = Some(snapshot);
    doc.node_views = node_views;
    doc.edge_views = edge_views;
    doc.draw_order_nodes = node_order;
    doc.draw_order_edges = edge_order;
}

fn apply_initial_layout_for_unseen_nodes(graph: &StateMachineGraph, node_views: &mut std::collections::HashMap<NodeId, UiNode>) {
    let default_size = egui::vec2(140.0, 60.0);
    let v_spacing = 100.0;
    let content_padding = egui::vec2(24.0, 24.0);
    let header_h_world = 24.0;
    let origin = egui::pos2(100.0, 100.0);

    // DFS traversal to assign positions; maintain next row per parent
    let mut stack: Vec<NodeId> = Vec::new();
    if graph.nodes.contains_key(&graph.root) { stack.push(graph.root); }
    let mut next_row_per_parent: std::collections::HashMap<NodeId, usize> = std::collections::HashMap::new();

    while let Some(id) = stack.pop() {
        // Ensure parent processed before children
        if let Some(node) = graph.nodes.get(&id) {
            for &child in node.children.iter().rev() { stack.push(child); }
        }

        // Assign root if unseen
        if id == graph.root {
            if let Some(root_view) = node_views.get_mut(&id) {
                if root_view.rect.min == egui::pos2(0.0, 0.0) {
                    root_view.rect = egui::Rect::from_min_size(origin, default_size);
                }
            }
            continue;
        }

        // For other nodes, if unseen, place inside parent's content area
        let parent_id = match graph.nodes.get(&id).and_then(|n| n.parent) { Some(p) => p, None => continue };
        let parent_min = node_views.get(&parent_id).map(|p| p.rect.min).unwrap_or(origin);
        let parent_has_header = graph.nodes.get(&parent_id).map(|p| !p.children.is_empty()).unwrap_or(false);
        let row = *next_row_per_parent.entry(parent_id).or_insert(0);
        if let Some(view) = node_views.get_mut(&id) {
            if view.rect.min == egui::pos2(0.0, 0.0) {
                let x = parent_min.x + content_padding.x;
                let y = parent_min.y + content_padding.y + (if parent_has_header { header_h_world } else { 0.0 }) + row as f32 * v_spacing;
                view.rect = egui::Rect::from_min_size(egui::pos2(x, y), default_size);
                if let Some(r) = next_row_per_parent.get_mut(&parent_id) { *r += 1; }
            }
        }
    }
}

fn compute_draw_orders(graph: &StateMachineGraph) -> (Vec<NodeId>, Vec<EdgeId>) {
    // Node order: DFS from root following children order
    let mut node_order: Vec<NodeId> = Vec::new();
    let mut stack: Vec<NodeId> = Vec::new();
    let mut seen: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    if graph.nodes.contains_key(&graph.root) { stack.push(graph.root); }
    while let Some(id) = stack.pop() {
        if !seen.insert(id) { continue; }
        node_order.push(id);
        if let Some(node) = graph.nodes.get(&id) {
            for &child in node.children.iter().rev() { stack.push(child); }
        }
    }

    // Build a ranking for nodes to order edges by source appearance
    let mut rank: std::collections::HashMap<NodeId, usize> = std::collections::HashMap::new();
    for (i, id) in node_order.iter().enumerate() { rank.insert(*id, i); }

    let mut edge_order: Vec<EdgeId> = Vec::new();
    // Prefer adjacency_out per node order for determinism
    for node_id in node_order.iter() {
        if let Some(out) = graph.adjacency_out.get(node_id) {
            for eid in out { edge_order.push(*eid); }
        } else {
            // fallback: scan edges with this source, sorted by target rank then by Debug of id
            let mut edges: Vec<&crate::model::Edge> = graph.edges.values().filter(|e| &e.source == node_id).collect();
            edges.sort_by_key(|e| (*rank.get(&e.target).unwrap_or(&usize::MAX), format!("{:?}", e.id)));
            for e in edges { edge_order.push(e.id); }
        }
    }

    (node_order, edge_order)
}


