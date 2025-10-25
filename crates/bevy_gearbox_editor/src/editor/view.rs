use super::view_model::GraphDoc;
use bevy_egui::egui;

/// Minimal read-only view with pan/zoom and basic nodes/edges rendering.
pub fn draw_doc(ui: &mut egui::Ui, doc: &mut GraphDoc) {
    let desired = ui.available_size_before_wrap();
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::drag());

    // Background
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(20));

    // Pan with drag on empty background
    if response.dragged() {
        let delta = response.drag_delta();
        if delta.length_sq() > 0.0 {
            doc.transform.pan_screen_delta(delta);
        }
    }

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

    // Read-only sizing pass for parents BEFORE drawing, so edges/nodes use updated bounds
    if let Some(graph) = &doc.graph {
        let mut new_bounds: Vec<(super::super::model::NodeId, egui::Rect)> = Vec::new();
        for (id, view) in doc.node_views.iter() {
            if let Some(node) = graph.nodes.get(id) {
                if node.children.is_empty() { continue; }
                let mut bounds = view.rect;
                for child_id in node.children.iter() {
                    if let Some(child_view) = doc.node_views.get(child_id) {
                        bounds.max.x = bounds.max.x.max(child_view.rect.max.x + 20.0);
                        bounds.max.y = bounds.max.y.max(child_view.rect.max.y + 20.0);
                    }
                }
                new_bounds.push((*id, bounds));
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

    // 2) Edges (above parents, under foreground nodes)
    if let Some(graph) = &doc.graph {
        for eid in doc.draw_order_edges.iter() {
            if let Some(edge) = doc.edge_views.get(eid) {
                let src_rect = match doc.node_views.get(&edge.source) { Some(n) => n.rect, None => continue };
                let dst_rect = match doc.node_views.get(&edge.target) { Some(n) => n.rect, None => continue };
                let src_has_header = graph.nodes.get(&edge.source).map(|n| !n.children.is_empty()).unwrap_or(false);
                let dst_has_header = graph.nodes.get(&edge.target).map(|n| !n.children.is_empty()).unwrap_or(false);
                let src_center_world = if src_has_header {
                    egui::pos2(src_rect.center().x, (src_rect.min.y + header_h_world + src_rect.max.y) * 0.5)
                } else { src_rect.center() };
                let dst_center_world = if dst_has_header {
                    egui::pos2(dst_rect.center().x, (dst_rect.min.y + header_h_world + dst_rect.max.y) * 0.5)
                } else { dst_rect.center() };

                let a = doc.transform.to_screen(src_center_world);
                let b = doc.transform.to_screen(dst_center_world);
                painter.line_segment([a, b], egui::Stroke::new(2.0, egui::Color32::from_gray(120)));
                // Pill on top of edge
                let pill_pos = doc.transform.to_screen(edge.pill.pos);
                painter.circle_filled(pill_pos, 6.0, egui::Color32::from_rgb(110, 170, 220));
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

    // Simple read-only sizing pass for parents: compute then apply to avoid borrow conflicts
    if let Some(graph) = &doc.graph {
        let mut new_bounds: Vec<(super::super::model::NodeId, egui::Rect)> = Vec::new();
        for (id, view) in doc.node_views.iter() {
            if let Some(node) = graph.nodes.get(id) {
                if node.children.is_empty() { continue; }
                let mut bounds = view.rect;
                for child_id in node.children.iter() {
                    if let Some(child_view) = doc.node_views.get(child_id) {
                        bounds.max.x = bounds.max.x.max(child_view.rect.max.x + 20.0);
                        bounds.max.y = bounds.max.y.max(child_view.rect.max.y + 20.0);
                    }
                }
                new_bounds.push((*id, bounds));
            }
        }
        for (id, rect) in new_bounds.into_iter() {
            if let Some(view) = doc.node_views.get_mut(&id) { view.rect = rect; }
        }
    }
}


