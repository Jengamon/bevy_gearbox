use super::view_model::{GraphDoc, UiViewKind};
use bevy_egui::egui;

/// Minimal read-only view with pan/zoom and basic nodes/edges rendering.
pub fn draw_doc(ui: &mut egui::Ui, doc: &mut GraphDoc) {
    let desired = ui.available_size_before_wrap();
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::drag());

    // Background
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(20));

    // Compute dynamic draw order per frame using DFS with per-level subtree ordering
    let effective_selected = doc.dragging.or(doc.selected);
    // Build map: for each ancestor in the selected chain, which child branch is selected
    let mut selected_by_parent: std::collections::HashMap<crate::model::EntityId, crate::model::EntityId> = std::collections::HashMap::new();
    if let Some(sel) = effective_selected {
        if !matches!(doc.views.get(&sel).map(|v| &v.kind), Some(UiViewKind::Edge { .. })) {
            let mut cur = Some(sel);
            while let Some(cid) = cur {
                if let Some(pid) = doc.transform_parent.get(&cid).and_then(|p| *p) {
                    // record that at parent pid, child cid should be ordered last among siblings
                    selected_by_parent.insert(pid, cid);
                    cur = Some(pid);
                } else {
                    break;
                }
            }
        }
    }

    // Build a deterministic global edge order using graph adjacency
    let mut edge_sequence: Vec<crate::model::EntityId> = Vec::new();
    let mut node_rank: std::collections::HashMap<crate::model::EntityId, usize> = std::collections::HashMap::new();
    if let Some(graph) = &doc.graph {
        // Node order by DFS from root
        let mut stack: Vec<crate::model::EntityId> = Vec::new();
        let mut seen: std::collections::HashSet<crate::model::EntityId> = std::collections::HashSet::new();
        stack.push(graph.root);
        let mut node_order: Vec<crate::model::EntityId> = Vec::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) { continue; }
            node_order.push(id);
            if let Some(node) = graph.nodes.get(&id) {
                for &child in node.children.iter().rev() { stack.push(child); }
            }
        }
        for (i, id) in node_order.iter().enumerate() { node_rank.insert(*id, i); }

        // Edge order prefers adjacency_out per node order
        for nid in node_order.iter() {
            if let Some(out) = graph.adjacency_out.get(nid) {
                for eid in out { edge_sequence.push(*eid); }
            } else {
                // Fallback: scan edges from this source and sort by target rank then ID string
                let mut edges: Vec<&crate::model::Edge> = graph.edges.values().filter(|e| &e.source == nid).collect();
                edges.sort_by_key(|e| (*node_rank.get(&e.target).unwrap_or(&usize::MAX), format!("{:?}", e.id)));
                for e in edges { edge_sequence.push(e.id); }
            }
        }
    }

    // Collect overlay edges: if a state is selected, include its (incoming ∪ outgoing);
    // if an edge is selected/dragging, include that edge itself
    let mut overlay_edges: std::collections::HashSet<crate::model::EntityId> = std::collections::HashSet::new();
    if let Some(sel) = effective_selected {
        match doc.views.get(&sel).map(|v| &v.kind) {
            Some(UiViewKind::Edge { .. }) => { overlay_edges.insert(sel); }
            _ => {
                if let Some(graph) = &doc.graph {
                    // Outgoing
                    if let Some(out_ids) = graph.adjacency_out.get(&sel) { for e in out_ids { overlay_edges.insert(*e); } }
                    // Incoming
                    if let Some(in_ids) = graph.adjacency_in.get(&sel) { for e in in_ids { overlay_edges.insert(*e); } }
                    // Filter to known edge views only
                    overlay_edges.retain(|eid| matches!(doc.views.get(eid).map(|v| &v.kind), Some(UiViewKind::Edge { .. })));
                }
            }
        }
    }

    // Gather edges for a given parent id in the deterministic global edge order, excluding overlay edges
    let edges_for_parent = |pid: &crate::model::EntityId, out: &mut Vec<crate::model::EntityId>| {
        for eid in edge_sequence.iter() {
            if overlay_edges.contains(eid) { continue; }
            if doc.transform_parent.get(eid).and_then(|p| *p) == Some(*pid) { out.push(*eid); }
        }
    };

    // DFS traversal building a contiguous order per subtree
    let mut order: Vec<crate::model::EntityId> = Vec::new();
    let visit = |root_id: crate::model::EntityId, order: &mut Vec<crate::model::EntityId>| {
        // Use an explicit stack to avoid borrow issues with recursion
        let mut stack: Vec<crate::model::EntityId> = Vec::new();
        let mut enter_stack: Vec<bool> = Vec::new(); // false = exit, true = enter
        stack.push(root_id);
        enter_stack.push(true);

        while let Some(enter) = enter_stack.pop() {
            let id = stack.pop().unwrap();
            if enter {
                // On enter: emit node and edges according to kind, then schedule children
                let is_parent_kind = matches!(doc.views.get(&id).map(|v| &v.kind), Some(UiViewKind::Parent | UiViewKind::Parallel));

                if is_parent_kind {
                    // Parent backgrounds first
                    order.push(id);
                    // Edges under children but above parent background
                    edges_for_parent(&id, order);
                } else {
                    // Leaf: edges should be below the leaf itself (nodes over edges)
                    edges_for_parent(&id, order);
                    order.push(id);
                }

                // Determine child node order
                let mut children_nodes: Vec<crate::model::EntityId> = Vec::new();
                if let Some(graph) = &doc.graph {
                    if let Some(node) = graph.nodes.get(&id) {
                        for &c in node.children.iter() { children_nodes.push(c); }
                    }
                }
                if let Some(sel_child) = selected_by_parent.get(&id) {
                    if let Some(pos) = children_nodes.iter().position(|c| c == sel_child) {
                        let v = children_nodes.remove(pos);
                        children_nodes.push(v);
                    }
                }

                // Schedule children in reverse because we push onto stack (LIFO)
                for child in children_nodes.into_iter().rev() {
                    stack.push(child);
                    enter_stack.push(true);
                }
            } else {
                // Currently no exit-time work
            }
        }
    };

    // Start traversal from graph root when available; else keep existing order
    if let Some(graph) = &doc.graph {
        visit(graph.root, &mut order);
        // Append overlay edges in the global deterministic edge order
        if !overlay_edges.is_empty() {
            for eid in edge_sequence.iter() {
                if overlay_edges.contains(eid) { order.push(*eid); }
            }
        }
        doc.draw_order = order.clone();
    } else {
        order = doc.draw_order.clone();
    }

    // Interactions: node/pill dragging vs background pan; hit test in front-to-back order
    let pointer_pos = response.ctx.input(|i| i.pointer.hover_pos());
    let mut hovered_entity: Option<crate::model::EntityId> = None;
    if let Some(pos) = pointer_pos {
        for eid in order.iter().rev() {
            if let Some(view) = doc.views.get(eid) {
                match view.kind {
                    UiViewKind::Edge { .. } => {
                        // Measure pill in screen space
                        let zoom = doc.transform.zoom;
                        let font_px = (14.0 * zoom).clamp(6.0, 64.0);
                        let font_id = egui::FontId::proportional(font_px);
                        let text_galley = painter.layout_no_wrap(view.label.clone(), font_id.clone(), egui::Color32::WHITE);
                        let text_size_s = text_galley.size();
                        let pill_pad_x = 10.0 * zoom;
                        let pill_pad_y = 6.0 * zoom;
                        let pill_size_s = egui::vec2(text_size_s.x + 2.0 * pill_pad_x, text_size_s.y + 2.0 * pill_pad_y);
                        let center_w = view.rect.center();
                        let pill_center_s = doc.transform.to_screen(center_w);
                        let pill_rect_s = egui::Rect::from_center_size(pill_center_s, pill_size_s);
                        if pill_rect_s.contains(pos) { hovered_entity = Some(*eid); break; }
                    }
                    _ => {
                        // Only header of containers is interactive; leaves use full rect
                        let rect = if let Some(graph) = &doc.graph {
                            let is_container = matches!(view.kind, UiViewKind::Parent | UiViewKind::Parallel) || graph.nodes.get(eid).map(|n| !n.children.is_empty()).unwrap_or(false);
                            if is_container {
                                let header_h_world = 24.0;
                                let header_rect_w = egui::Rect::from_min_max(view.rect.min, egui::pos2(view.rect.max.x, view.rect.min.y + header_h_world));
                                egui::Rect::from_min_max(doc.transform.to_screen(header_rect_w.min), doc.transform.to_screen(header_rect_w.max))
                            } else {
                                egui::Rect::from_min_max(doc.transform.to_screen(view.rect.min), doc.transform.to_screen(view.rect.max))
                            }
                        } else {
                            egui::Rect::from_min_max(doc.transform.to_screen(view.rect.min), doc.transform.to_screen(view.rect.max))
                        };
                        if rect.contains(pos) { hovered_entity = Some(*eid); break; }
                    }
                }
            }
        }
    }

    // On drag start: capture draggable if hovering; also select it. Else pan background
    if response.drag_started() {
        if let Some(ent) = hovered_entity {
            if let Some(cursor) = response.ctx.input(|i| i.pointer.hover_pos()) {
                let pointer_world = doc.transform.to_world(cursor);
                // Compute rect for entity (node rect or pill rect in world space)
                let Some(view) = doc.views.get(&ent) else { return; };

                let anchor = egui::vec2(pointer_world.x - view.rect.min.x, pointer_world.y - view.rect.min.y);
                    doc.dragging = Some(ent);
                    doc.drag_anchor_world = Some(anchor);
                doc.selected = Some(ent);
            }
        }
    }

    // Click to select; clicking empty background clears selection
    if response.clicked() {
        doc.selected = hovered_entity;
    }

    // During drag: move draggable in world coords, with clamping to parent content
    let delta_screen = response.drag_delta();
    if response.dragged() {
        if let (Some(ent), Some(anchor)) = (doc.dragging, doc.drag_anchor_world) {
            if let Some(cursor) = response.ctx.input(|i| i.pointer.hover_pos()) {
                // Desired new min from anchored pointer
                let pointer_world = doc.transform.to_world(cursor);
                let desired_min = egui::pos2(pointer_world.x - anchor.x, pointer_world.y - anchor.y);

                // If it's a node (non-edge)
                if matches!(doc.views.get(&ent).map(|v| &v.kind), Some(UiViewKind::Leaf | UiViewKind::Parent | UiViewKind::Parallel)) {
                    // Pre-fetch parent rect and header flag
                    let mut parent_rect: Option<egui::Rect> = None;
                    let mut parent_has_header = false;
                    if let Some(graph) = &doc.graph {
                        if let Some(parent_id) = graph.nodes.get(&ent).and_then(|n| n.parent) {
                            parent_rect = doc.views.get(&parent_id).map(|v| v.rect);
                            parent_has_header = graph.nodes.get(&parent_id).map(|p| !p.children.is_empty()).unwrap_or(false);
                        }
                    }
                    // Compute new rect using a temporary copy of the old rect, then write back
                    let (old_rect, size) = match doc.views.get(&ent) { Some(v) => (v.rect, v.rect.size()), None => (egui::Rect::NAN, egui::Vec2::ZERO) };
                    if old_rect.is_finite() {
                        let mut new_rect = egui::Rect::from_min_size(desired_min, size);
                        if let Some(p_rect) = parent_rect {
                            let header_h_world = 24.0;
                            let pad = egui::vec2(24.0, 24.0);
                            let content_min = egui::pos2(
                                p_rect.min.x + pad.x,
                                p_rect.min.y + pad.y + if parent_has_header { header_h_world } else { 0.0 },
                            );
                            if new_rect.min.x < content_min.x { let dx = content_min.x - new_rect.min.x; new_rect = new_rect.translate(egui::vec2(dx, 0.0)); }
                            if new_rect.min.y < content_min.y { let dy = content_min.y - new_rect.min.y; new_rect = new_rect.translate(egui::vec2(0.0, dy)); }
                        }
                        let move_delta = new_rect.min - old_rect.min;
                        if let Some(v) = doc.views.get_mut(&ent) { v.rect = new_rect; }
                        if move_delta != egui::vec2(0.0, 0.0) {
                            // Descendants via transform_children
                            let mut stack: Vec<crate::model::EntityId> = doc.transform_children.get(&ent).cloned().unwrap_or_default();
                            while let Some(cid) = stack.pop() {
                                if let Some(cv) = doc.views.get_mut(&cid) {
                                    cv.rect = cv.rect.translate(move_delta);
                                    // Keep pill center coherent for edge views
                                    if let Some(pill) = cv.pill.as_mut() { pill.center += move_delta; }
                                }
                                if let Some(more) = doc.transform_children.get(&cid) { for &g in more { stack.push(g); } }
                            }
                        }
                    }
                } else if matches!(doc.views.get(&ent).map(|v| &v.kind), Some(UiViewKind::Edge { .. })) {
                    // Compute rect without holding a mutable borrow; only write at the end
                    let label = doc.views.get(&ent).map(|v| v.label.clone()).unwrap_or_default();
                        // Compute pill size in world (measure in screen, convert by zoom)
                        let zoom = doc.transform.zoom;
                        let font_px = (14.0 * zoom).clamp(6.0, 64.0);
                        let font_id = egui::FontId::proportional(font_px);
                    let text_galley = painter.layout_no_wrap(label, font_id, egui::Color32::WHITE);
                        let size_s = text_galley.size();
                        let pad_s = egui::vec2(10.0 * zoom, 6.0 * zoom);
                        let size_w = egui::vec2((size_s.x + 2.0 * pad_s.x) / zoom, (size_s.y + 2.0 * pad_s.y) / zoom);
                        let mut rect = egui::Rect::from_min_size(desired_min, size_w);
                        // Clamp only to left/top of pill_parent content; allow right/bottom to expand parent
                        if let Some(pp) = doc.transform_parent.get(&ent).and_then(|p| *p) {
                        if let Some(parent_view) = doc.views.get(&pp) {
                            if let Some(graph) = &doc.graph {
                                let header_h_world = 24.0;
                                let pad = egui::vec2(24.0, 24.0);
                                let has_header = graph.nodes.get(&pp).map(|p| !p.children.is_empty()).unwrap_or(false);
                                let content_min = egui::pos2(
                                    parent_view.rect.min.x + pad.x,
                                    parent_view.rect.min.y + pad.y + if has_header { header_h_world } else { 0.0 },
                                );
                                if rect.min.x < content_min.x { let dx = content_min.x - rect.min.x; rect = rect.translate(egui::vec2(dx, 0.0)); }
                                if rect.min.y < content_min.y { let dy = content_min.y - rect.min.y; rect = rect.translate(egui::vec2(0.0, dy)); }
                                rect.max = rect.min + rect.size();
                            }
                        }
                        }
                    if let Some(v) = doc.views.get_mut(&ent) {
                        v.rect = rect;
                        if let Some(p) = v.pill.as_mut() { p.center = rect.center(); }
                    }
                    }
                }
        } else {
            // Dragging with no captured entity → pan background
            if delta_screen.length_sq() > 0.0 {
                doc.transform.pan_screen_delta(delta_screen);
            }
        }
    }

    // Auto-pan canvas while dragging near the viewport edges to keep node under cursor
    if doc.dragging.is_some() {
        if let Some(cursor) = response.ctx.input(|i| i.pointer.hover_pos()) {
            let margin = 24.0;
            let step = 10.0; // screen points per frame
            if cursor.x < rect.min.x + margin { doc.transform.pan_screen_delta(egui::vec2(step, 0.0)); }
            if cursor.x > rect.max.x - margin { doc.transform.pan_screen_delta(egui::vec2(-step, 0.0)); }
            if cursor.y < rect.min.y + margin { doc.transform.pan_screen_delta(egui::vec2(0.0, step)); }
            if cursor.y > rect.max.y - margin { doc.transform.pan_screen_delta(egui::vec2(0.0, -step)); }
        }
    }

    if response.drag_stopped() { doc.dragging = None; doc.drag_anchor_world = None; }

    // Zoom at cursor with scroll
    let scroll_y = response.ctx.input(|i| i.smooth_scroll_delta.y);
    if scroll_y != 0.0 {
        let scroll: f32 = scroll_y;
        if scroll.abs() > 0.0 {
            let factor = 1.0 + (-scroll * 0.001);
            let cursor = response.ctx.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
            let cursor = cursor.clamp(rect.min, rect.max);
            doc.transform.zoom_around_screen_point(factor, cursor);
        }
    }

    // Draw graph if any
    if doc.graph.is_none() { return; }

    // Read-only container pass BEFORE drawing: clamp children to parent content left/top, then expand parent right/bottom
    if let Some(graph) = &doc.graph {
        let header_h_world = 24.0;
        let content_padding = egui::vec2(24.0, 24.0);

        // 1) Clamp children to parent's content left/top (no shrinking on right/bottom here)
        let mut clamped_children: Vec<(super::super::model::EntityId, egui::Rect)> = Vec::new();
        for (id, node) in graph.nodes.iter() {
            let Some(parent_id) = node.parent else { continue };
            let Some(parent_view) = doc.views.get(&parent_id) else { continue };
            let Some(child_view) = doc.views.get(id) else { continue };
            let mut rect = child_view.rect;
            let content_min = egui::pos2(
                parent_view.rect.min.x + content_padding.x,
                parent_view.rect.min.y + content_padding.y + if graph.nodes.get(&parent_id).map(|p| !p.children.is_empty()).unwrap_or(false) { header_h_world } else { 0.0 },
            );
            if rect.min.x < content_min.x { let dx = content_min.x - rect.min.x; rect = rect.translate(egui::vec2(dx, 0.0)); }
            if rect.min.y < content_min.y { let dy = content_min.y - rect.min.y; rect = rect.translate(egui::vec2(0.0, dy)); }
            if rect != child_view.rect { clamped_children.push((*id, rect)); }
        }
        for (id, rect) in clamped_children.into_iter() {
            if let Some(view) = doc.views.get_mut(&id) { view.rect = rect; }
        }

        // 2) Expand/shrink each parent right/bottom to tightly include its children and pills plus padding
        let mut new_bounds: Vec<(super::super::model::EntityId, egui::Rect)> = Vec::new();
        for (id, view) in doc.views.iter() {
            if !matches!(view.kind, UiViewKind::Leaf | UiViewKind::Parent | UiViewKind::Parallel) { continue; }
            if let Some(node) = graph.nodes.get(id) {
                if node.children.is_empty() { continue; }
                let base_min = view.rect.min;
                // Start with minimum content area (header + padding), then grow by children
                let mut req_max = egui::pos2(
                    base_min.x + content_padding.x,
                    base_min.y + content_padding.y + header_h_world,
                );
                for child_id in node.children.iter() {
                    if let Some(child_view) = doc.views.get(child_id) {
                        req_max.x = req_max.x.max(child_view.rect.max.x + content_padding.x);
                        req_max.y = req_max.y.max(child_view.rect.max.y + content_padding.y);
                    }
                }
                // Include any pills whose pill_parent is this container via transform_children
                if let Some(children) = doc.transform_children.get(id) {
                    for cid in children.iter() {
                        if let Some(cv) = doc.views.get(cid) {
                            if matches!(cv.kind, UiViewKind::Edge { .. }) {
                                req_max.x = req_max.x.max(cv.rect.max.x + content_padding.x);
                                req_max.y = req_max.y.max(cv.rect.max.y + content_padding.y);
                            }
                        }
                    }
                }
                // Enforce a minimal size
                req_max.x = req_max.x.max(base_min.x + 140.0);
                req_max.y = req_max.y.max(base_min.y + 60.0);
                new_bounds.push((*id, egui::Rect::from_min_max(base_min, req_max)));
            }
        }
        for (id, rect) in new_bounds.into_iter() {
            if let Some(view) = doc.views.get_mut(&id) { view.rect = rect; }
        }
    }

    // Single-pass layered draw using computed order
    let zoom = doc.transform.zoom;
    let font_px = (14.0 * zoom).clamp(6.0, 64.0);
    let font_id = egui::FontId::proportional(font_px);
    let pad = 8.0 * zoom;
    let header_h_world = 24.0;

    // Helpers for edge geometry
    let rect_from_inside_toward = |rect: egui::Rect, toward: egui::Pos2| -> egui::Pos2 {
        let c = rect.center();
        let d = toward - c;
        let hx = rect.width() * 0.5_f32;
        let hy = rect.height() * 0.5_f32;
        let sx = if d.x.abs() > 0.0001 { hx / d.x.abs() } else { f32::INFINITY };
        let sy = if d.y.abs() > 0.0001 { hy / d.y.abs() } else { f32::INFINITY };
        let s = sx.min(sy);
        c + d * s
    };
    let rect_from_outside_toward_center = |rect: egui::Rect, from: egui::Pos2| -> egui::Pos2 {
        rect_from_inside_toward(rect, from)
    };

    // Helper to draw a dashed rounded-rectangle border in screen space
    let draw_dashed_rounded_rect = |rect: egui::Rect, radius: f32, color: egui::Color32, thickness: f32, dash: f32, gap: f32| {
        let draw_segmented = |a: egui::Pos2, b: egui::Pos2| {
            let total_len = (b - a).length();
            if total_len <= 0.0 { return; }
            let dir = (b - a) / total_len;
            let mut t = 0.0;
            while t < total_len {
                let seg_len = dash.min(total_len - t);
                let start = a + dir * t;
                let end = a + dir * (t + seg_len);
                painter.line_segment([start, end], egui::Stroke { width: thickness, color });
                t += dash + gap;
            }
        };
        let draw_dashed_arc = |center: egui::Pos2, r: f32, a0: f32, a1: f32| {
            if r <= 0.0 { return; }
            let arc_len = r * (a1 - a0).abs();
            if arc_len <= 0.0 { return; }
            let dir_sign = if a1 >= a0 { 1.0 } else { -1.0 };
            let mut s = 0.0;
            while s < arc_len {
                let seg_len = dash.min(arc_len - s);
                let a_start = a0 + dir_sign * (s / r);
                let a_end = a0 + dir_sign * ((s + seg_len) / r);
                let p0 = egui::pos2(center.x + r * a_start.cos(), center.y + r * a_start.sin());
                let p1 = egui::pos2(center.x + r * a_end.cos(), center.y + r * a_end.sin());
                painter.line_segment([p0, p1], egui::Stroke { width: thickness, color });
                s += dash + gap;
            }
        };

        let x0 = rect.min.x;
        let y0 = rect.min.y;
        let x1 = rect.max.x;
        let y1 = rect.max.y;
        let r = radius.clamp(0.0, ((x1 - x0).abs().min((y1 - y0).abs())) * 0.5);

        if r <= 0.0 {
            // Fallback: square rectangle
            draw_segmented(egui::pos2(x0, y0), egui::pos2(x1, y0));
            draw_segmented(egui::pos2(x1, y0), egui::pos2(x1, y1));
            draw_segmented(egui::pos2(x1, y1), egui::pos2(x0, y1));
            draw_segmented(egui::pos2(x0, y1), egui::pos2(x0, y0));
            return;
        }

        // Straight segments (shortened by radius on both ends)
        draw_segmented(egui::pos2(x0 + r, y0), egui::pos2(x1 - r, y0)); // top
        draw_segmented(egui::pos2(x1, y0 + r), egui::pos2(x1, y1 - r)); // right
        draw_segmented(egui::pos2(x1 - r, y1), egui::pos2(x0 + r, y1)); // bottom
        draw_segmented(egui::pos2(x0, y1 - r), egui::pos2(x0, y0 + r)); // left

        // Corner arcs (screen space). Angles assume +x right, +y down.
        let pi = std::f32::consts::PI;
        // Top-left: from pi to 1.5*pi
        draw_dashed_arc(egui::pos2(x0 + r, y0 + r), r, pi, 1.5 * pi);
        // Top-right: from 1.5*pi to 2*pi
        draw_dashed_arc(egui::pos2(x1 - r, y0 + r), r, 1.5 * pi, 2.0 * pi);
        // Bottom-right: from 0 to 0.5*pi
        draw_dashed_arc(egui::pos2(x1 - r, y1 - r), r, 0.0, 0.5 * pi);
        // Bottom-left: from 0.5*pi to pi
        draw_dashed_arc(egui::pos2(x0 + r, y1 - r), r, 0.5 * pi, pi);
    };

    // Helper to draw "initial" indicator: small solid circle outside top-left with a curved arrow to the node's left edge
    let draw_initial_indicator = |rect_screen: egui::Rect| {
        // Solid circle, half previous size
        let r = 4.0 * zoom;
        let x_offset = 16.0 * zoom;
        let y_offset = 4.0 * zoom;
        let start = egui::pos2(rect_screen.min.x - x_offset, rect_screen.min.y - y_offset);

        // Terminate on the left edge of the node, slightly below the top-left corner
        let end = egui::pos2(rect_screen.min.x, rect_screen.min.y + 8.0 * zoom);

        // Cubic Bézier controls to start downward then turn right
        let k = 14.0 * zoom;
        let c1 = egui::pos2(start.x, start.y + k);         // vertical tangent at start
        let c2 = egui::pos2(end.x - k, end.y);             // horizontal tangent at end (pointing right)

        // Draw solid circle
        painter.circle_filled(start, r, egui::Color32::WHITE);

        // Sample and draw the cubic Bézier
        let segments = 20;
        let mut prev = start;
        for i in 1..=segments {
            let t = (i as f32) / (segments as f32);
            let omt = 1.0 - t;
            // Cubic Bézier interpolation
            let x = omt * omt * omt * start.x
                + 3.0 * omt * omt * t * c1.x
                + 3.0 * omt * t * t * c2.x
                + t * t * t * end.x;
            let y = omt * omt * omt * start.y
                + 3.0 * omt * omt * t * c1.y
                + 3.0 * omt * t * t * c2.y
                + t * t * t * end.y;
            let p = egui::pos2(x, y);
            painter.line_segment([prev, p], egui::Stroke::new(2.0, egui::Color32::WHITE));
            prev = p;
        }

        // Arrowhead pointing along the end tangent (cubic derivative at t=1 is proportional to end - c2)
        let end_tangent = (end - c2).normalized();
        let arrow_len = 10.0 * zoom;
        let arrow_w = 8.0 * zoom;
        let tip = end;
        let base = tip - end_tangent * arrow_len;
        let perp = egui::pos2(-end_tangent.y, end_tangent.x);
        let left = base + perp.to_vec2() * (arrow_w * 0.5);
        let right = base - perp.to_vec2() * (arrow_w * 0.5);
        painter.add(egui::Shape::convex_polygon(
            vec![tip, left, right],
            egui::Color32::WHITE,
            egui::Stroke::new(0.0, egui::Color32::TRANSPARENT),
        ));
    };

    // Helper: is this view a direct child of a Parallel state (by view kind or by graph components)?
    let is_direct_child_of_parallel_fn = |child_id: &crate::model::EntityId| -> bool {
        let parent_opt = doc.transform_parent.get(child_id).and_then(|p| *p);
        let by_view = parent_opt
            .and_then(|pid| doc.views.get(&pid))
            .map(|pv| matches!(pv.kind, UiViewKind::Parallel))
            .unwrap_or(false);
        if by_view { return true; }
        if let (Some(graph), Some(pid)) = (&doc.graph, parent_opt) {
            if let Some(parent_node) = graph.nodes.get(&pid) {
                let has_parallel = parent_node.components.keys().any(|k| k == crate::component::PARALLEL || k.ends_with("::Parallel") || k.ends_with("::Parallel>"));
                return has_parallel;
            }
        }
        false
    };

    for id in order.iter() {
        let Some(view) = doc.views.get(id) else { continue };
        match view.kind {
            UiViewKind::Parent | UiViewKind::Parallel => {
                let rect_world = view.rect;
                let min = doc.transform.to_screen(rect_world.min);
                let max = doc.transform.to_screen(rect_world.max);
                let rect_screen = egui::Rect::from_min_max(min, max);
                let rounding = egui::CornerRadius::same(6);
                // Fill
                painter.rect_filled(rect_screen, rounding, egui::Color32::from_rgb(30, 30, 35));
                let header_min = rect_world.min;
                let header_max = egui::pos2(rect_world.max.x, rect_world.min.y + header_h_world);
                let header_rect = egui::Rect::from_min_max(doc.transform.to_screen(header_min), doc.transform.to_screen(header_max));
                painter.rect_filled(header_rect, egui::CornerRadius::same(6), egui::Color32::from_rgb(38, 38, 46));
                painter.hline(header_rect.x_range(), header_rect.max.y, egui::Stroke::new(1.0, egui::Color32::from_gray(90)));
                let label_pos = header_rect.min + egui::vec2(pad, pad * 0.5);
                painter.text(label_pos, egui::Align2::LEFT_TOP, &view.label, font_id.clone(), egui::Color32::WHITE);
                // Border: dashed if direct child of a Parallel (draw after header so it stays visible)
                let is_direct_child_of_parallel = is_direct_child_of_parallel_fn(id);
                if is_direct_child_of_parallel {
                    let dash = 6.0;
                    let gap = 4.0;
                    draw_dashed_rounded_rect(rect_screen, 6.0, egui::Color32::from_gray(160), 1.0, dash, gap);
                } else {
                    painter.rect(
                        rect_screen,
                        rounding,
                        egui::Color32::TRANSPARENT,
                        egui::Stroke::new(1.0, egui::Color32::from_gray(160)),
                        egui::StrokeKind::Outside,
                    );
                }
                // Initial indicator for nodes that are the parent's initial child
                if doc.is_initial_child.contains(id) {
                    draw_initial_indicator(rect_screen);
                }
            }
            UiViewKind::Edge { source, target } => {
                // Endpoints
                let Some(src_view) = doc.views.get(&source) else { continue };
                let Some(dst_view) = doc.views.get(&target) else { continue };
                let src_rect_w = src_view.rect;
                let dst_rect_w = dst_view.rect;

                let src_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(src_rect_w.min), doc.transform.to_screen(src_rect_w.max));
                let dst_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(dst_rect_w.min), doc.transform.to_screen(dst_rect_w.max));

                let pill_center_s = doc.transform.to_screen(view.rect.center());
                let text_galley = painter.layout_no_wrap(view.label.clone(), font_id.clone(), egui::Color32::WHITE);
                let text_size_s = text_galley.size();
                let pill_pad_x = 10.0 * zoom;
                let pill_pad_y = 6.0 * zoom;
                let pill_size_s = egui::vec2(text_size_s.x + 2.0 * pill_pad_x, text_size_s.y + 2.0 * pill_pad_y);
                let pill_rect_s = egui::Rect::from_center_size(pill_center_s, pill_size_s);

                let a_start = rect_from_inside_toward(src_rect_s, pill_center_s);
                let a_end = rect_from_outside_toward_center(pill_rect_s, a_start);
                // If source is an ancestor of target, draw an outward-and-back curve to create a loop illusion
                let mut is_ancestor_edge = false;
                {
                    let mut cur = Some(target);
                    while let Some(cid) = cur {
                        if cid == source { is_ancestor_edge = true; break; }
                        cur = doc.transform_parent.get(&cid).and_then(|p| *p);
                    }
                }
                if is_ancestor_edge {
                    // Determine outward normal based on which side a_start lies on
                    let eps = 0.5;
                    let normal = if (a_start.x - src_rect_s.min.x).abs() <= eps { egui::vec2(-1.0, 0.0) }
                    else if (a_start.x - src_rect_s.max.x).abs() <= eps { egui::vec2(1.0, 0.0) }
                    else if (a_start.y - src_rect_s.min.y).abs() <= eps { egui::vec2(0.0, -1.0) }
                    else if (a_start.y - src_rect_s.max.y).abs() <= eps { egui::vec2(0.0, 1.0) }
                    else {
                        // Fallback: pick axis-aligned direction away from rect center
                        let d = (a_start - src_rect_s.center()).normalized();
                        if d.x.abs() >= d.y.abs() { egui::vec2(d.x.signum(), 0.0) } else { egui::vec2(0.0, d.y.signum()) }
                    };
                    let loop_out = 48.0 * zoom; // loop distance in screen space
                    let p_out = a_start + normal * loop_out;
                    // Sample a quadratic bezier (a_start -> p_out -> a_end)
                    let segments = 20;
                    let mut prev = a_start;
                    for i in 1..=segments {
                        let t = (i as f32) / (segments as f32);
                        let omt = 1.0 - t;
                        let x = omt * omt * a_start.x + 2.0 * omt * t * p_out.x + t * t * a_end.x;
                        let y = omt * omt * a_start.y + 2.0 * omt * t * p_out.y + t * t * a_end.y;
                        let p = egui::pos2(x, y);
                        painter.line_segment([prev, p], egui::Stroke::new(2.0, egui::Color32::from_gray(120)));
                        prev = p;
                    }
                } else {
                painter.line_segment([a_start, a_end], egui::Stroke::new(2.0, egui::Color32::from_gray(120)));
                }

                let b_end = rect_from_inside_toward(dst_rect_s, pill_center_s);
                let b_start = rect_from_outside_toward_center(pill_rect_s, b_end);
                painter.line_segment([b_start, b_end], egui::Stroke::new(2.0, egui::Color32::from_gray(120)));

                let dir = (b_end - b_start).normalized();
                let arrow_len = 10.0 * zoom;
                let arrow_w = 8.0 * zoom;
                let tip = b_end;
                let base = tip - dir * arrow_len;
                let perp = egui::pos2(-dir.y, dir.x);
                let left = base + perp.to_vec2() * (arrow_w * 0.5);
                let right = base - perp.to_vec2() * (arrow_w * 0.5);
                painter.add(egui::Shape::convex_polygon(
                    vec![tip, left, right],
                    egui::Color32::from_gray(160),
                    egui::Stroke::new(0.0, egui::Color32::TRANSPARENT),
                ));

                let rounding = egui::CornerRadius::same((pill_size_s.y * 0.5).round() as u8);
                painter.rect(
                    pill_rect_s,
                    rounding,
                    egui::Color32::from_rgb(30, 30, 35),
                    egui::Stroke::new(1.0, egui::Color32::from_gray(160)),
                    egui::StrokeKind::Outside,
                );
                painter.text(pill_rect_s.center(), egui::Align2::CENTER_CENTER, &view.label, font_id.clone(), egui::Color32::WHITE);
            }
            UiViewKind::Leaf => {
                let rect_world = view.rect;
            let min = doc.transform.to_screen(rect_world.min);
            let max = doc.transform.to_screen(rect_world.max);
            let rect_screen = egui::Rect::from_min_max(min, max);
            let rounding = egui::CornerRadius::same(6);
                // Fill
                painter.rect_filled(rect_screen, rounding, egui::Color32::from_rgb(30, 30, 35));
                // Border: dashed if direct child of a Parallel
                let is_direct_child_of_parallel = is_direct_child_of_parallel_fn(id);
                if is_direct_child_of_parallel {
                    let dash = 6.0;
                    let gap = 4.0;
                    draw_dashed_rounded_rect(rect_screen, 6.0, egui::Color32::from_gray(160), 1.0, dash, gap);
                } else {
            painter.rect(
                rect_screen,
                rounding,
                        egui::Color32::TRANSPARENT,
                egui::Stroke::new(1.0, egui::Color32::from_gray(160)),
                egui::StrokeKind::Outside,
            );
                }
                painter.text(rect_screen.center_top() + egui::vec2(0.0, 12.0 * zoom), egui::Align2::CENTER_TOP, &view.label, font_id.clone(), egui::Color32::WHITE);
                // Initial indicator for nodes that are the parent's initial child
                if doc.is_initial_child.contains(id) {
                    draw_initial_indicator(rect_screen);
                }
            }
        }
    }

    // (old extra sizing pass removed; handled above)
}


