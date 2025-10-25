use super::view_model::GraphDoc;
use bevy_egui::egui;

/// Minimal read-only view with pan/zoom and basic nodes/edges rendering.
pub fn draw_doc(ui: &mut egui::Ui, doc: &mut GraphDoc) {
    let desired = ui.available_size_before_wrap();
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::drag());

    // Background
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(20));

    // Interactions: node/pill dragging vs background pan
    // We compute hits against nodes in reverse draw order (foreground first)
    let pointer_pos = response.ctx.input(|i| i.pointer.hover_pos());
    let mut hovered_entity: Option<crate::model::EntityId> = None;
    if let Some(pos) = pointer_pos {
        for nid in doc.draw_order_nodes.iter().rev() {
            if let Some(node) = doc.node_views.get(nid) {
                // Screen rect for hit-testing
                let min = doc.transform.to_screen(node.rect.min);
                let max = doc.transform.to_screen(node.rect.max);
                let rect = egui::Rect::from_min_max(min, max);
                if rect.contains(pos) { hovered_entity = Some(*nid); break; }
            }
        }
        // Check pills above edges
        for eid in doc.draw_order_edges.iter().rev() {
            if let Some(edge) = doc.edge_views.get(eid) {
                let zoom = doc.transform.zoom;
                let font_px = (14.0 * zoom).clamp(6.0, 64.0);
                let font_id = egui::FontId::proportional(font_px);
                let text_galley = painter.layout_no_wrap(edge.label.clone(), font_id.clone(), egui::Color32::WHITE);
                let text_size_s = text_galley.size();
                let pill_pad_x = 10.0 * zoom;
                let pill_pad_y = 6.0 * zoom;
                let pill_size_s = egui::vec2(text_size_s.x + 2.0 * pill_pad_x, text_size_s.y + 2.0 * pill_pad_y);
                let pill_center_s = doc.transform.to_screen(edge.pill.pos);
                let pill_rect_s = egui::Rect::from_center_size(pill_center_s, pill_size_s);
                if pill_rect_s.contains(pos) { hovered_entity = Some(*eid); break; }
            }
        }
    }

    // On drag start: capture draggable if hovering; else pan background
    if response.drag_started() {
        if let Some(ent) = hovered_entity {
            if let Some(cursor) = response.ctx.input(|i| i.pointer.hover_pos()) {
                let pointer_world = doc.transform.to_world(cursor);
                // Compute rect for entity (node rect or pill rect in world space)
                let rect_world = if let Some(nid) = doc.node_views.keys().find(|nid| **nid == ent).cloned() {
                    doc.node_views.get(&nid).map(|n| n.rect)
                } else if let Some(eid) = doc.edge_views.keys().find(|eid| **eid == ent).cloned() {
                    doc.edge_views.get(&eid).map(|e| {
                        let zoom = doc.transform.zoom;
                        let font_px = (14.0 * zoom).clamp(6.0, 64.0);
                        let font_id = egui::FontId::proportional(font_px);
                        let text_galley = painter.layout_no_wrap(e.label.clone(), font_id, egui::Color32::WHITE);
                        let size_s = text_galley.size();
                        let pad_s = egui::vec2(10.0 * zoom, 6.0 * zoom);
                        let size_w = egui::vec2((size_s.x + 2.0 * pad_s.x) / zoom, (size_s.y + 2.0 * pad_s.y) / zoom);
                        egui::Rect::from_center_size(e.pill.pos, size_w)
                    })
                } else { None };
                if let Some(rect) = rect_world {
                    let anchor = egui::vec2(pointer_world.x - rect.min.x, pointer_world.y - rect.min.y);
                    doc.dragging = Some(ent);
                    doc.drag_anchor_world = Some(anchor);
                }
            }
        }
    }

    // During drag: move draggable in world coords, with clamping to parent content
    if response.dragged() {
        let delta_screen = response.drag_delta();
        if let (Some(ent), Some(anchor)) = (doc.dragging, doc.drag_anchor_world) {
            if let Some(cursor) = response.ctx.input(|i| i.pointer.hover_pos()) {
                // Desired new min from anchored pointer
                let pointer_world = doc.transform.to_world(cursor);
                let desired_min = egui::pos2(pointer_world.x - anchor.x, pointer_world.y - anchor.y);

                // If it's a node
                if let Some(nid) = doc.node_views.keys().find(|nid| **nid == ent).cloned() {
                    // Pre-fetch parent rect and header flag
                    let mut parent_rect: Option<egui::Rect> = None;
                    let mut parent_has_header = false;
                    if let Some(graph) = &doc.graph {
                        if let Some(parent_id) = graph.nodes.get(&nid).and_then(|n| n.parent) {
                            parent_rect = doc.node_views.get(&parent_id).map(|v| v.rect);
                            parent_has_header = graph.nodes.get(&parent_id).map(|p| !p.children.is_empty()).unwrap_or(false);
                        }
                    }
                    if let Some(node) = doc.node_views.get_mut(&nid) {
                        let old = node.rect;
                        let size = node.rect.size();
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
                        // Apply transform propagation for this container (children nodes and pills)
                        let move_delta = new_rect.min - old.min;
                        node.rect = new_rect;
                        if move_delta != egui::vec2(0.0, 0.0) {
                            if let Some(graph) = &doc.graph {
                                // Move descendant nodes
                                if let Some(n) = graph.nodes.get(&nid) {
                                    if !n.children.is_empty() {
                                        let mut stack: Vec<crate::model::EntityId> = n.children.clone();
                                        while let Some(cid) = stack.pop() {
                                            if let Some(cv) = doc.node_views.get_mut(&cid) { cv.rect = cv.rect.translate(move_delta); }
                                            if let Some(cn) = graph.nodes.get(&cid) { for &g in cn.children.iter() { stack.push(g); } }
                                        }
                                    }
                                }
                                // Move pills whose pill_parent is within affected set
                                let mut affected_parents: std::collections::HashSet<crate::model::EntityId> = std::collections::HashSet::new();
                                affected_parents.insert(nid);
                                if let Some(n) = graph.nodes.get(&nid) {
                                    let mut stack: Vec<crate::model::EntityId> = n.children.clone();
                                    while let Some(cid) = stack.pop() {
                                        affected_parents.insert(cid);
                                        if let Some(cn) = graph.nodes.get(&cid) { for &g in cn.children.iter() { stack.push(g); } }
                                    }
                                }
                                for (_, e) in doc.edge_views.iter_mut() {
                                    if let Some(pp) = e.pill_parent { if affected_parents.contains(&pp) { e.pill.pos += move_delta; } }
                                }
                            }
                        }
                    }
                } else if let Some(eid) = doc.edge_views.keys().find(|eid| **eid == ent).cloned() {
                    if let Some(edge) = doc.edge_views.get_mut(&eid) {
                        // Compute pill size in world (measure in screen, convert by zoom)
                        let zoom = doc.transform.zoom;
                        let font_px = (14.0 * zoom).clamp(6.0, 64.0);
                        let font_id = egui::FontId::proportional(font_px);
                        let text_galley = painter.layout_no_wrap(edge.label.clone(), font_id, egui::Color32::WHITE);
                        let size_s = text_galley.size();
                        let pad_s = egui::vec2(10.0 * zoom, 6.0 * zoom);
                        let size_w = egui::vec2((size_s.x + 2.0 * pad_s.x) / zoom, (size_s.y + 2.0 * pad_s.y) / zoom);
                        let mut rect = egui::Rect::from_min_size(desired_min, size_w);
                        // Clamp only to left/top of pill_parent content; allow right/bottom to expand parent
                        if let Some(graph) = &doc.graph {
                            if let Some(pp) = edge.pill_parent {
                                if let Some(parent_view) = doc.node_views.get(&pp) {
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
                        edge.pill.pos = rect.center();
                    }
                }
            }
        } else if delta_screen.length_sq() > 0.0 {
            // Background pan
            doc.transform.pan_screen_delta(delta_screen);
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
            let Some(parent_view) = doc.node_views.get(&parent_id) else { continue };
            let Some(child_view) = doc.node_views.get(id) else { continue };
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
            if let Some(view) = doc.node_views.get_mut(&id) { view.rect = rect; }
        }

        // 2) Expand/shrink each parent right/bottom to tightly include its children and pills plus padding
        let mut new_bounds: Vec<(super::super::model::EntityId, egui::Rect)> = Vec::new();
        for (id, view) in doc.node_views.iter() {
            if let Some(node) = graph.nodes.get(id) {
                if node.children.is_empty() { continue; }
                let base_min = view.rect.min;
                // Start with minimum content area (header + padding), then grow by children
                let mut req_max = egui::pos2(
                    base_min.x + content_padding.x,
                    base_min.y + content_padding.y + header_h_world,
                );
                for child_id in node.children.iter() {
                    if let Some(child_view) = doc.node_views.get(child_id) {
                        req_max.x = req_max.x.max(child_view.rect.max.x + content_padding.x);
                        req_max.y = req_max.y.max(child_view.rect.max.y + content_padding.y);
                    }
                }
                // Include any pills whose pill_parent is this container
                for e in doc.edge_views.values() {
                    if e.pill_parent == Some(*id) {
                        // Estimate pill rect in world: use a conservative default height and measured width if available later in the frame
                        let pill_center_w = e.pill.pos;
                        // Approximate half-size; conservative 12 world units vertically, 40 horizontally
                        let half = egui::vec2(40.0, 12.0);
                        let pill_max = egui::pos2(pill_center_w.x + half.x, pill_center_w.y + half.y);
                        req_max.x = req_max.x.max(pill_max.x + content_padding.x);
                        req_max.y = req_max.y.max(pill_max.y + content_padding.y);
                    }
                }
                // Enforce a minimal size
                req_max.x = req_max.x.max(base_min.x + 140.0);
                req_max.y = req_max.y.max(base_min.y + 60.0);
                new_bounds.push((*id, egui::Rect::from_min_max(base_min, req_max)));
            }
        }
        for (id, rect) in new_bounds.into_iter() {
            if let Some(view) = doc.node_views.get_mut(&id) { view.rect = rect; }
        }
    }

    // Layered draw order:
    // 1) Parents (backgrounds, headers)
    // 2) Edges (above parents, below children)
    // 3) Children / non-parents (and any remaining nodes treated as foreground)

    let zoom = doc.transform.zoom;
    let font_px = (14.0 * zoom).clamp(6.0, 64.0);
    let font_id = egui::FontId::proportional(font_px);
    let pad = 8.0 * zoom;
    let header_h_world = 24.0;

    // 1) Parents first (backgrounds and headers)
    if let Some(graph) = &doc.graph {
        for nid in doc.draw_order_nodes.iter() {
            if let Some(node) = doc.node_views.get(nid) {
                let is_container = graph.nodes.get(nid).map(|n| !n.children.is_empty()).unwrap_or(false);
                if !is_container { continue; }
                let rect_world = node.rect;
                let min = doc.transform.to_screen(rect_world.min);
                let max = doc.transform.to_screen(rect_world.max);
                let rect_screen = egui::Rect::from_min_max(min, max);
                let rounding = egui::CornerRadius::same(6);
                painter.rect(
                    rect_screen,
                    rounding,
                    egui::Color32::from_rgb(30, 30, 35),
                    egui::Stroke::new(1.0, egui::Color32::from_gray(160)),
                    egui::StrokeKind::Outside,
                );
                // Header
                let header_min = rect_world.min;
                let header_max = egui::pos2(rect_world.max.x, rect_world.min.y + header_h_world);
                let header_rect = egui::Rect::from_min_max(doc.transform.to_screen(header_min), doc.transform.to_screen(header_max));
                painter.rect_filled(header_rect, egui::CornerRadius::same(6), egui::Color32::from_rgb(38, 38, 46));
                painter.hline(header_rect.x_range(), header_rect.max.y, egui::Stroke::new(1.0, egui::Color32::from_gray(90)));
                let label_pos = header_rect.min + egui::vec2(pad, pad * 0.5);
                painter.text(label_pos, egui::Align2::LEFT_TOP, &node.label, font_id.clone(), egui::Color32::WHITE);
            }
        }
    }

    // 2) Edges and Event Pills (above parents, under foreground nodes)
    if let Some(graph) = &doc.graph {
        // Helpers for shortest attachment points on rectangles
        let rect_from_inside_toward = |rect: egui::Rect, toward: egui::Pos2| -> egui::Pos2 {
            // Assumes start point is inside rect: use center-ray intersection
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
            // Intersection point on rect boundary along ray from rect.center() to `from`
            rect_from_inside_toward(rect, from)
        };

        for eid in doc.draw_order_edges.iter() {
            if let Some(edge) = doc.edge_views.get(eid) {
                // Fetch endpoint rects in world, adjust centers to ignore header area for containers
                let src_rect_w = match doc.node_views.get(&edge.source) { Some(n) => n.rect, None => continue };
                let dst_rect_w = match doc.node_views.get(&edge.target) { Some(n) => n.rect, None => continue };
                let src_has_header = graph.nodes.get(&edge.source).map(|n| !n.children.is_empty()).unwrap_or(false);
                let dst_has_header = graph.nodes.get(&edge.target).map(|n| !n.children.is_empty()).unwrap_or(false);
                let _src_center_w = if src_has_header {
                    egui::pos2(src_rect_w.center().x, (src_rect_w.min.y + header_h_world + src_rect_w.max.y) * 0.5)
                } else { src_rect_w.center() };
                let _dst_center_w = if dst_has_header {
                    egui::pos2(dst_rect_w.center().x, (dst_rect_w.min.y + header_h_world + dst_rect_w.max.y) * 0.5)
                } else { dst_rect_w.center() };

                // Transform to screen for geometry math and drawing
                let src_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(src_rect_w.min), doc.transform.to_screen(src_rect_w.max));
                let dst_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(dst_rect_w.min), doc.transform.to_screen(dst_rect_w.max));
                // centers not needed further; keep in world for future use if needed

                // Measure pill label in screen space
                let pill_center_s = doc.transform.to_screen(edge.pill.pos);
                let text_galley = painter.layout_no_wrap(edge.label.clone(), font_id.clone(), egui::Color32::WHITE);
                let text_size_s = text_galley.size();
                let pill_pad_x = 10.0 * zoom;
                let pill_pad_y = 6.0 * zoom;
                let pill_size_s = egui::vec2(text_size_s.x + 2.0 * pill_pad_x, text_size_s.y + 2.0 * pill_pad_y);
                let pill_rect_s = egui::Rect::from_center_size(pill_center_s, pill_size_s);

                // Segment A: source perimeter → pill perimeter (shortest)
                let a_start = rect_from_inside_toward(src_rect_s, pill_center_s);
                let a_end = rect_from_outside_toward_center(pill_rect_s, a_start);
                painter.line_segment([a_start, a_end], egui::Stroke::new(2.0, egui::Color32::from_gray(120)));

                // Segment B: pill perimeter → target perimeter (shortest), with arrow at target
                let b_end = rect_from_inside_toward(dst_rect_s, pill_center_s);
                let b_start = rect_from_outside_toward_center(pill_rect_s, b_end);
                painter.line_segment([b_start, b_end], egui::Stroke::new(2.0, egui::Color32::from_gray(120)));

                // Arrowhead at b_end
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

                // Pill capsule draw above edges
                let rounding = egui::CornerRadius::same((pill_size_s.y * 0.5).round() as u8);
                // Style like nodes (grey fill and border)
                painter.rect(
                    pill_rect_s,
                    rounding,
                    egui::Color32::from_rgb(30, 30, 35),
                    egui::Stroke::new(1.0, egui::Color32::from_gray(160)),
                    egui::StrokeKind::Outside,
                );
                painter.text(pill_rect_s.center(), egui::Align2::CENTER_CENTER, &edge.label, font_id.clone(), egui::Color32::WHITE);
            }
        }
    }

    // 3) Foreground nodes (non-parents)
    if let Some(graph) = &doc.graph {
        for nid in doc.draw_order_nodes.iter() {
            let is_container = graph.nodes.get(nid).map(|n| !n.children.is_empty()).unwrap_or(false);
            if is_container { continue; }
            if let Some(node) = doc.node_views.get(nid) {
            let rect_world = node.rect;
            let min = doc.transform.to_screen(rect_world.min);
            let max = doc.transform.to_screen(rect_world.max);
            let rect_screen = egui::Rect::from_min_max(min, max);
            let rounding = egui::CornerRadius::same(6);
            // Node background
            painter.rect(
                rect_screen,
                rounding,
                egui::Color32::from_rgb(30, 30, 35),
                egui::Stroke::new(1.0, egui::Color32::from_gray(160)),
                egui::StrokeKind::Outside,
            );

            // Scaled font and padding based on zoom
            let zoom = doc.transform.zoom;
            let font_px = (14.0 * zoom).clamp(6.0, 64.0);
            let font_id = egui::FontId::proportional(font_px);
            let _pad = 8.0 * zoom;

            // Leaf label (center-top with scaled offset)
            painter.text(rect_screen.center_top() + egui::vec2(0.0, 12.0 * zoom), egui::Align2::CENTER_TOP, &node.label, font_id, egui::Color32::WHITE);
            }
        }
    }

    // (old extra sizing pass removed; handled above)
}


