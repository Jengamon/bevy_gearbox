use super::view_model::{GraphDoc, StateKind};
use super::layout::{NodeLayout, LayoutConfig};
use super::context_menu::{build_context_menu, MenuItemKind, MenuSelection};
use crate::editor::workspace::{ RenameInline, Workspace, EdgeBuildState, EdgeMenuState};
use crate::types::EntityId;
use bevy_egui::egui;

/// Minimal read-only view with pan/zoom and basic nodes/edges rendering.
pub fn draw_doc(
    ui: &mut egui::Ui,
    doc: &mut GraphDoc,
    selection: &mut Option<EntityId>,
    doc_id: EntityId,
    workspace: &mut Workspace,
) -> Option<MenuSelection> {
    let desired = ui.available_size_before_wrap();
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());

    // Background
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(20));

    // Tick highlight animations and request repaint while animating
    let animating = doc.tick_highlights(0.92);
    if animating { ui.ctx().request_repaint(); }

    // Scene provides stable ordering; no per-frame edge ordering needed

    // Construct NodeLayout from the prebuilt scene
    let mut layout = NodeLayout::new(
        doc.scene.node_rects.clone(),
        doc.scene.tree.parent_of.clone(),
        doc.scene.tree.children_of.clone(),
        doc.scene.tree.containers.clone(),
        doc.graph.as_ref().map(|g| g.root),
    );
    let cfg = LayoutConfig::default();
    // Pre-draw layout updates
    layout.clamp_children_left_top(&cfg);
    layout.fit_parents_to_children(&cfg, None);
    // Sync rects back to scene
    for (id, rect) in layout.node_rects.iter() { doc.set_rect(id, *rect); }

    // Selection-aware draw order from layout: when selecting an edge, bias to its parent (or source)
    let effective_selected = doc.dragging.or(*selection);
    let selected_for_bias = effective_selected.and_then(|sel| {
        if let Some(ev) = doc.scene.edges.get(&sel) {
            if let Some(pid) = doc.scene.tree.parent_of.get(&sel).and_then(|p| *p) { Some(pid) } else { Some(ev.source) }
        } else { Some(sel) }
    });
    let base_order = layout.compute_draw_order(selected_for_bias).to_vec();

    // Overlay edges: selected state's incoming ∪ outgoing, or the selected edge itself
    let mut overlay_edges: std::collections::HashSet<EntityId> = std::collections::HashSet::new();
    if let Some(sel) = effective_selected {
        if doc.scene.edges.contains_key(&sel) { overlay_edges.insert(sel); }
        else if let Some(graph) = &doc.graph {
                    if let Some(out_ids) = graph.adjacency_out.get(&sel) { for e in out_ids { overlay_edges.insert(*e); } }
                    if let Some(in_ids) = graph.adjacency_in.get(&sel) { for e in in_ids { overlay_edges.insert(*e); } }
            overlay_edges.retain(|eid| doc.scene.edges.contains_key(eid));
        }
    }

    // Draw each edge only once: remove overlay edges from base order, then append them on top
    let mut order: Vec<EntityId> = base_order
        .into_iter()
        .filter(|id| !overlay_edges.contains(id))
        .collect();
    if !overlay_edges.is_empty() {
        for eid in doc.scene.draw_order.iter() { if overlay_edges.contains(eid) { order.push(*eid); } }
    }
    doc.scene.draw_order = order.clone();

    // Interactions: node/pill dragging vs background pan; hit test in front-to-back order
    let pointer_pos = response.ctx.input(|i| i.pointer.hover_pos());
    let mut hovered_entity: Option<EntityId> = None;
    if let Some(pos) = pointer_pos {
        for eid in order.iter().rev() {
            if doc.scene.edges.contains_key(eid) {
                if let Some(edge) = doc.scene.edges.get(eid) {
                        let zoom = doc.transform.zoom;
                    let text_size_s = doc.cached_label_size_screen(&edge.label, zoom, &painter);
                        let pill_pad_x = 10.0 * zoom;
                        let pill_pad_y = 6.0 * zoom;
                        let pill_size_s = egui::vec2(text_size_s.x + 2.0 * pill_pad_x, text_size_s.y + 2.0 * pill_pad_y);
                    let center_w = doc.scene.node_rects.get(eid).map(|r| r.center()).unwrap_or(egui::pos2(0.0, 0.0));
                        let pill_center_s = doc.transform.to_screen(center_w);
                        let pill_rect_s = egui::Rect::from_center_size(pill_center_s, pill_size_s);
                        if pill_rect_s.contains(pos) { hovered_entity = Some(*eid); break; }
                    }
            } else {
                        if let Some(rect) = layout.interactive_rect_screen(eid, &cfg, &doc.transform) {
                            if rect.contains(pos) { hovered_entity = Some(*eid); break; }
                }
            }
        }
    }

    // On drag start: capture draggable if hovering; also select it. Else pan background
    if response.drag_started() && response.ctx.input(|i| i.pointer.primary_down()) {
        if let Some(ent) = hovered_entity {
            if let Some(cursor) = response.ctx.input(|i| i.pointer.hover_pos()) {
                let pointer_world = doc.transform.to_world(cursor);
                // Compute rect for entity (node rect or pill rect in world space)
                let rect_w = doc.scene.node_rects.get(&ent).copied().unwrap_or(egui::Rect::from_min_max(pointer_world, pointer_world));
                let anchor = egui::vec2(pointer_world.x - rect_w.min.x, pointer_world.y - rect_w.min.y);
                    doc.dragging = Some(ent);
                    doc.drag_anchor_world = Some(anchor);
                *selection = Some(ent);
            }
        }
    }

    // Click to select; clicking empty background clears selection
    if response.clicked() {
        *selection = hovered_entity;
    }

    // Right-click context menu trigger: only create an interact response for the hovered entity
    // so topmost widget wins for hit-testing, but keep menu open independently of hover.
    let mut context_menu_selection: Option<MenuSelection> = None;
    if doc.graph.is_some() && workspace.edge_build.is_none() {
        for eid in order.iter() {
            // Compute interactive rect in screen space: full rect for nodes, pill rect for edges
            let rect_screen = if let Some(sv) = doc.scene.states.get(eid) {
                egui::Rect::from_min_max(doc.transform.to_screen(sv.rect.min), doc.transform.to_screen(sv.rect.max))
            } else if let Some(ev) = doc.scene.edges.get(eid) {
                    let zoom = doc.transform.zoom;
                let text_size_s = doc.cached_label_size_screen(&ev.label, zoom, &ui.painter());
                    let pill_pad_x = 10.0 * zoom;
                    let pill_pad_y = 6.0 * zoom;
                    let pill_size_s = egui::vec2(text_size_s.x + 2.0 * pill_pad_x, text_size_s.y + 2.0 * pill_pad_y);
                let center_w = doc.scene.node_rects.get(eid).map(|r| r.center()).unwrap_or(ev.rect.center());
                    let pill_center_s = doc.transform.to_screen(center_w);
                    egui::Rect::from_center_size(pill_center_s, pill_size_s)
            } else { continue };
            // Build a stable, collision-free id per doc, per entity, and per kind (node vs edge)
            let kind_tag: &str = if doc.scene.edges.contains_key(eid) { "edge" } else { "node" };
            let id = egui::Id::new(("node_ctx", doc_id, kind_tag)).with(*eid);
            let resp = ui.interact(rect_screen, id, egui::Sense::click());
            if resp.clicked() { *selection = Some(*eid); }
            resp.context_menu(|menu_ui| {
                *selection = Some(*eid);
                menu_ui.set_min_width(160.0);
                // If we're in edge-kind selection for this node, override with edge-kind menu
                if let Some(edge_menu) = workspace.edge_menu.clone() {
                    if edge_menu.doc == doc_id && edge_menu.target == *eid {
                        if menu_ui.button("Always").clicked() {
                            workspace.pending_edge_create = Some(crate::editor::workspace::PendingEdgeCreate { doc: doc_id, source: edge_menu.source, target: edge_menu.target, kind: "Always".to_string() });
                            workspace.preview_edges.retain(|pe| !(pe.doc == doc_id && pe.source == edge_menu.source && pe.target == edge_menu.target));
                            workspace.edge_menu = None;
                            workspace.edge_build = None;
                            menu_ui.close();
                        }
                        for label in workspace.available_event_edges.clone().into_iter() {
                            if menu_ui.button(&label).clicked() {
                                workspace.pending_edge_create = Some(crate::editor::workspace::PendingEdgeCreate { doc: doc_id, source: edge_menu.source, target: edge_menu.target, kind: label.clone() });
                                workspace.preview_edges.retain(|pe| !(pe.doc == doc_id && pe.source == edge_menu.source && pe.target == edge_menu.target));
                                workspace.edge_menu = None;
                                workspace.edge_build = None;
                                menu_ui.close();
                            }
                        }
                        menu_ui.separator();
                        if menu_ui.button("Cancel").clicked() {
                            workspace.edge_menu = None;
                            workspace.edge_build = None;
                            menu_ui.close();
                        }
                        return;
                    }
                }
                if doc.scene.states.contains_key(eid) {
                        if let Some(graph) = &doc.graph {
                            let items = build_context_menu(graph, *eid);
                            if items.is_empty() {
                                menu_ui.close();
                                return;
                            }
                            for item in items.into_iter() {
                                if menu_ui.button(item.label).clicked() {
                                    let sel = match item.kind {
                                        MenuItemKind::MakeLeaf => MenuSelection::MakeLeaf { target: *eid },
                                        MenuItemKind::MakeParent => MenuSelection::MakeParent { target: *eid },
                                        MenuItemKind::MakeParallel => MenuSelection::MakeParallel { target: *eid },
                                        MenuItemKind::Save => MenuSelection::SaveStateMachine { target: *eid },
                                        MenuItemKind::SaveSubstates => MenuSelection::SaveSubstates { target: *eid },
                                        MenuItemKind::Rename => MenuSelection::RenameEntity { target: *eid },
                                        MenuItemKind::Delete => MenuSelection::DeleteEntity { target: *eid },
                                        MenuItemKind::MakeInitial { parent } => MenuSelection::MakeInitial { parent, new_initial: *eid },
                                        MenuItemKind::AddChild => MenuSelection::AddChildStateMachine { target: *eid },
                                    };
                                    context_menu_selection = Some(sel);
                                    menu_ui.close();
                                }
                            }
                        } else {
                            menu_ui.close();
                        }
                    } else {
                        if menu_ui.button("Rename").clicked() {
                            context_menu_selection = Some(MenuSelection::RenameEntity { target: *eid });
                            menu_ui.close();
                        }
                        if menu_ui.button("Delete").clicked() {
                            context_menu_selection = Some(MenuSelection::DeleteEntity { target: *eid });
                            menu_ui.close();
                    }
                }
            });
        }
        // Persistent menu rendering handled by egui::Context; nothing to draw here.
    }

    // During drag: move draggable in world coords, with clamping to parent content via NodeLayout
    let delta_screen = response.drag_delta();
    if response.dragged() {
        if let (Some(ent), Some(anchor)) = (doc.dragging, doc.drag_anchor_world) {
            if let Some(cursor) = response.ctx.input(|i| i.pointer.hover_pos()) {
                let pointer_world = doc.transform.to_world(cursor);
                let desired_min = egui::pos2(pointer_world.x - anchor.x, pointer_world.y - anchor.y);
                if !doc.scene.edges.contains_key(&ent) {
                        let _ = layout.move_node_clamped_and_propagate(ent, desired_min, &cfg);
                        // Sync rects back to scene
                        for (id, rect) in layout.node_rects.iter() { doc.set_rect(id, *rect); }
                } else {
                        // Compute pill size in world from cached label size, set desired rect, then clamp via layout
                        let label = doc.scene.edges.get(&ent).map(|v| v.label.clone()).unwrap_or_default();
                        let zoom = doc.transform.zoom;
                        let size_s = doc.cached_label_size_screen(&label, zoom, &painter);
                        let pad_s = egui::vec2(10.0 * zoom, 6.0 * zoom);
                        let size_w = egui::vec2((size_s.x + 2.0 * pad_s.x) / zoom, (size_s.y + 2.0 * pad_s.y) / zoom);
                        let rect = egui::Rect::from_min_size(desired_min, size_w);
                        layout.node_rects.insert(ent, rect);
                        layout.clamp_children_left_top(&cfg);
                        // Sync rects back to scene
                        for (id, rect) in layout.node_rects.iter() { doc.set_rect(id, *rect); }
                }
            }
        } else {
            if delta_screen.length_sq() > 0.0 && response.ctx.input(|i| i.pointer.primary_down()) {
                doc.transform.pan_screen_delta(delta_screen);
            }
        }
    }

    // Auto-pan canvas while dragging near the viewport edges to keep node under cursor
    if doc.dragging.is_some() {
        if let Some(cursor) = response.ctx.input(|i| i.pointer.hover_pos()) {
            let pan = NodeLayout::autopan_suggestion(rect, cursor, 24.0, 10.0);
            if pan != egui::Vec2::ZERO { doc.transform.pan_screen_delta(pan); }
        }
    }

    if response.drag_stopped() { doc.dragging = None; doc.drag_anchor_world = None; }

    // Draw graph if any
    if doc.graph.is_none() { return context_menu_selection; }

    // Layout handled via NodeLayout above; legacy pre-draw sizing/clamp pass removed.

    // Single-pass layered draw using computed order
    let zoom = doc.transform.zoom;
    let font_px = (14.0 * zoom).clamp(6.0, 64.0);
    let font_id = egui::FontId::proportional(font_px);
    let pad = 8.0 * zoom;
    // header height is derived from layout.header_rect; keep constant here only for sizing heuristics elsewhere if needed

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

    // Helper: draw a translucent selection halo ring around a rect (screen space)
    let draw_selection_halo = |rect_screen: egui::Rect, rounding: egui::CornerRadius| {
        let halo_w = (0.75 + 0.5 * zoom.sqrt()).clamp(0.75, 2.0);
        let halo_rect = rect_screen.expand(4.0);
        painter.rect(
            halo_rect,
            rounding,
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(halo_w, egui::Color32::from_rgba_premultiplied(120, 180, 255, 32)),
            egui::StrokeKind::Outside,
        );
    };

    // Helper: see free function `is_direct_substate_of_parallel`

    for id in order.iter() {
        if let Some(sv) = doc.scene.states.get(id) {
            // Debug: print classification and name when drawing as a state
            let is_container = !matches!(sv.kind, StateKind::Leaf);
            if is_container {
                let rect_world = sv.rect;
                let min = doc.transform.to_screen(rect_world.min);
                let max = doc.transform.to_screen(rect_world.max);
                let rect_screen = egui::Rect::from_min_max(min, max);
                let rounding = egui::CornerRadius::same(6);
                // Fill (container body stays gray; header changes color)
                let base_fill = egui::Color32::from_rgb(30, 30, 35);
                let base_yellow = egui::Color32::from_rgb(230, 200, 40);
                let bright_yellow = egui::Color32::from_rgb(255, 240, 0);
                let lerp_color = |a: egui::Color32, b: egui::Color32, t: f32| -> egui::Color32 {
                    let cl = |x: f32| -> u8 { x.clamp(0.0, 255.0) as u8 };
                    let ta = t.clamp(0.0, 1.0);
                    let inv = 1.0 - ta;
                    let r = a.r() as f32 * inv + b.r() as f32 * ta;
                    let g = a.g() as f32 * inv + b.g() as f32 * ta;
                    let bch = a.b() as f32 * inv + b.b() as f32 * ta;
                    egui::Color32::from_rgb(cl(r), cl(g), cl(bch))
                };
                painter.rect_filled(rect_screen, rounding, base_fill);
                let header_rect_world = layout.header_rect(id, &cfg).unwrap_or(rect_world);
                let header_rect = egui::Rect::from_min_max(doc.transform.to_screen(header_rect_world.min), doc.transform.to_screen(header_rect_world.max));
                // Header color: active -> yellow family; deactivated fades from yellow -> gray
                let is_active = doc.active_nodes.contains(id);
                let flash_t = doc.node_flash.get(id).copied().unwrap_or(0.0);
                let fade_t = doc.node_fade.get(id).copied().unwrap_or(0.0);
                let mut header_color = egui::Color32::from_rgb(38, 38, 46);
                if is_active {
                    let alpha = 1.0 - flash_t;
                    header_color = lerp_color(bright_yellow, base_yellow, alpha);
                } else if fade_t > 0.0 {
                    // fade down from base yellow to header gray
                    let alpha = 1.0 - fade_t;
                    header_color = lerp_color(base_yellow, egui::Color32::from_rgb(38, 38, 46), alpha);
                }
                painter.rect_filled(header_rect, egui::CornerRadius::same(6), header_color);
                painter.hline(header_rect.x_range(), header_rect.max.y, egui::Stroke::new(1.0, egui::Color32::from_gray(90)));
                let label_pos = header_rect.min + egui::vec2(pad, pad * 0.5);
                // Text color: black when header is yellow, else white; lerp on fade
                let mut text_col = egui::Color32::WHITE;
                if is_active {
                    text_col = egui::Color32::BLACK;
                } else if fade_t > 0.0 {
                    // approximate: crossfade black->white opposite to header fade
                    let alpha = 1.0 - fade_t;
                    text_col = lerp_color(egui::Color32::BLACK, egui::Color32::WHITE, alpha);
                }
                // Inline rename for container nodes
                let edit_rect = egui::Rect::from_min_max(label_pos, egui::pos2(header_rect.max.x - pad, label_pos.y + 20.0 * zoom));
                draw_label_or_inline_editor(
                    ui,
                    workspace,
                    doc_id,
                    id,
                    edit_rect,
                    &painter,
                    label_pos,
                    egui::Align2::LEFT_TOP,
                    &sv.label,
                    &font_id,
                    text_col,
                );
                // Selection halo (drawn before border so border stays crisp)
                let is_selected = selection.as_ref().map(|s| *s == *id).unwrap_or(false);
                if is_selected { draw_selection_halo(rect_screen, egui::CornerRadius::same(8)); }
                // Arrow-handle to start edge building when selected and not already building
                if is_selected && workspace.edge_build.is_none() {
                    let handle_r = 6.0 * zoom;
                    let handle_center = egui::pos2(rect_screen.max.x + handle_r + 2.0 * zoom, rect_screen.center().y);
                    let handle_rect = egui::Rect::from_center_size(handle_center, egui::vec2(handle_r * 2.0, handle_r * 2.0));
                    let hid = egui::Id::new(("edge_handle", doc_id, "node")).with(*id);
                    let hresp = ui.interact(handle_rect, hid, egui::Sense::click());
                    painter.circle_filled(handle_center, handle_r, egui::Color32::from_rgb(110, 190, 255));
                    painter.circle_stroke(handle_center, handle_r, egui::Stroke::new(1.0, egui::Color32::from_gray(240)));
                    if hresp.clicked() {
                        workspace.edge_build = Some(EdgeBuildState { doc: doc_id, source: *id, just_started: true });
                    }
                }
                // Border: dashed if direct child of a Parallel (draw after header so it stays visible)
                let is_direct_substate_of_parallel = is_direct_substate_of_parallel(doc, id);
                if is_direct_substate_of_parallel {
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
            } else {
                // Debug: also log if this id is unexpectedly present as an edge (should not happen)
                if doc.scene.edges.contains_key(id) {
                    if let Some(ev) = doc.scene.edges.get(id) {
                        println!("[editor][warn] entity id={:?} also classified as EDGE with name={} while drawing as STATE name={}", id, ev.label, sv.label);
                    }
                }
                // Leaf state rendering (see container branch for shared helpers)
                let rect_world = sv.rect;
            let min = doc.transform.to_screen(rect_world.min);
            let max = doc.transform.to_screen(rect_world.max);
            let rect_screen = egui::Rect::from_min_max(min, max);
            let rounding = egui::CornerRadius::same(6);
                // Fill (leaf body changes fully; header rule doesn't apply here)
                let base_fill = egui::Color32::from_rgb(30, 30, 35);
                let base_yellow = egui::Color32::from_rgb(230, 200, 40);
                let bright_yellow = egui::Color32::from_rgb(255, 240, 0);
                let lerp_color = |a: egui::Color32, b: egui::Color32, t: f32| -> egui::Color32 {
                    let cl = |x: f32| -> u8 { x.clamp(0.0, 255.0) as u8 };
                    let ta = t.clamp(0.0, 1.0);
                    let inv = 1.0 - ta;
                    let r = a.r() as f32 * inv + b.r() as f32 * ta;
                    let g = a.g() as f32 * inv + b.g() as f32 * ta;
                    let bch = a.b() as f32 * inv + b.b() as f32 * ta;
                    egui::Color32::from_rgb(cl(r), cl(g), cl(bch))
                };
                let is_active = doc.active_nodes.contains(id);
                let flash_t = doc.node_flash.get(id).copied().unwrap_or(0.0);
                let fade_t = doc.node_fade.get(id).copied().unwrap_or(0.0);
                let fill_color = if is_active {
                    let alpha = 1.0 - flash_t;
                    lerp_color(bright_yellow, base_yellow, alpha)
                } else if fade_t > 0.0 {
                    let alpha = 1.0 - fade_t;
                    lerp_color(base_yellow, base_fill, alpha)
                } else { base_fill };
                painter.rect_filled(rect_screen, rounding, fill_color);
                // Selection halo (drawn before border so border stays crisp)
                let is_selected = selection.as_ref().map(|s| *s == *id).unwrap_or(false);
                if is_selected { draw_selection_halo(rect_screen, egui::CornerRadius::same(8)); }
                // Arrow-handle to start edge building when selected and not already building
                if is_selected && workspace.edge_build.is_none() {
                    let handle_r = 6.0 * zoom;
                    let handle_center = egui::pos2(rect_screen.max.x + handle_r + 2.0 * zoom, rect_screen.center().y);
                    let handle_rect = egui::Rect::from_center_size(handle_center, egui::vec2(handle_r * 2.0, handle_r * 2.0));
                    let hid = egui::Id::new(("edge_handle", doc_id, "node")).with(*id);
                    let hresp = ui.interact(handle_rect, hid, egui::Sense::click());
                    painter.circle_filled(handle_center, handle_r, egui::Color32::from_rgb(110, 190, 255));
                    painter.circle_stroke(handle_center, handle_r, egui::Stroke::new(1.0, egui::Color32::from_gray(240)));
                    if hresp.clicked() {
                        workspace.edge_build = Some(EdgeBuildState { doc: doc_id, source: *id, just_started: true });
                    }
                }
                // Border: dashed if direct child of a Parallel
                let is_direct_substate_of_parallel = is_direct_substate_of_parallel(doc, id);
                if is_direct_substate_of_parallel {
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
                // Text color: black when yellow-ish, else white; also lerp on fade
                let mut text_col = egui::Color32::WHITE;
                if is_active { text_col = egui::Color32::BLACK; }
                else if fade_t > 0.0 {
                    let alpha = 1.0 - fade_t;
                    text_col = lerp_color(egui::Color32::BLACK, egui::Color32::WHITE, alpha);
                }
                // Inline rename for leaf nodes
                let label_top = rect_screen.center_top() + egui::vec2(0.0, 12.0 * zoom);
                let edit_rect = egui::Rect::from_min_max(
                    egui::pos2(rect_screen.min.x + 8.0 * zoom, label_top.y - 2.0 * zoom),
                    egui::pos2(rect_screen.max.x - 8.0 * zoom, label_top.y + 18.0 * zoom),
                );
                draw_label_or_inline_editor(
                    ui,
                    workspace,
                    doc_id,
                    id,
                    edit_rect,
                    &painter,
                    label_top,
                    egui::Align2::CENTER_TOP,
                    &sv.label,
                    &font_id,
                    text_col,
                );
                // Initial indicator for nodes that are the parent's initial child
                if doc.is_initial_child.contains(id) {
                    draw_initial_indicator(rect_screen);
                }
            }
        } else if let Some(ev) = doc.scene.edges.get(id) {
            // Edge rendering
            let source = ev.source;
            let target = ev.target;
            let Some(src_view) = doc.scene.states.get(&source) else { continue };
            let Some(dst_view) = doc.scene.states.get(&target) else { continue };
                let src_rect_w = src_view.rect;
                let dst_rect_w = dst_view.rect;

                let src_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(src_rect_w.min), doc.transform.to_screen(src_rect_w.max));
                let dst_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(dst_rect_w.min), doc.transform.to_screen(dst_rect_w.max));

            let pill_center_s = doc.transform.to_screen(doc.scene.node_rects.get(id).map(|r| r.center()).unwrap_or(ev.rect.center()));
            let text_size_s = doc.cached_label_size_screen(&ev.label, zoom, &painter);
                let pill_pad_x = 10.0 * zoom;
                let pill_pad_y = 6.0 * zoom;
                let pill_size_s = egui::vec2(text_size_s.x + 2.0 * pill_pad_x, text_size_s.y + 2.0 * pill_pad_y);
                let pill_rect_s = egui::Rect::from_center_size(pill_center_s, pill_size_s);

                // Selection halo around pill (drawn before pill border)
                let is_selected = selection.as_ref().map(|s| *s == *id).unwrap_or(false);
                if is_selected {
                    let rounding = egui::CornerRadius::same((pill_size_s.y * 0.5).round() as u8);
                    draw_selection_halo(pill_rect_s, rounding);
                }

                let a_start = rect_from_inside_toward(src_rect_s, pill_center_s);
                let a_end = rect_from_outside_toward_center(pill_rect_s, a_start);
                // If source is an ancestor of target, draw an outward-and-back curve to create a loop illusion
                let mut is_ancestor_edge = false;
                {
                    let mut cur = Some(target);
                    while let Some(cid) = cur {
                        if cid == source { is_ancestor_edge = true; break; }
                    cur = doc.scene.tree.parent_of.get(&cid).and_then(|p| *p);
                    }
                }
                // Edge highlight color (bright yellow -> base gray)
                let bright_yellow = egui::Color32::from_rgb(255, 240, 0);
                let base_gray_line = egui::Color32::from_gray(120);
                let base_gray_edge = egui::Color32::from_gray(160);
                let lerp_color = |a: egui::Color32, b: egui::Color32, t: f32| -> egui::Color32 {
                    let cl = |x: f32| -> u8 { x.clamp(0.0, 255.0) as u8 };
                    let ta = t.clamp(0.0, 1.0);
                    let inv = 1.0 - ta;
                    let r = a.r() as f32 * inv + b.r() as f32 * ta;
                    let g = a.g() as f32 * inv + b.g() as f32 * ta;
                    let bch = a.b() as f32 * inv + b.b() as f32 * ta;
                    egui::Color32::from_rgb(cl(r), cl(g), cl(bch))
                };
                let t_edge = doc.edge_flash.get(id).copied().unwrap_or(0.0);
                let alpha = 1.0 - t_edge;
                let edge_line_col = lerp_color(bright_yellow, base_gray_line, alpha);
                let edge_col = lerp_color(bright_yellow, base_gray_edge, alpha);
                // Edge pill body/text: flash bright yellow then lerp back to base fill & text to white
                let base_fill = egui::Color32::from_rgb(30, 30, 35);
                let pill_fill_col = lerp_color(bright_yellow, base_fill, alpha);
                let pill_text_col = lerp_color(egui::Color32::BLACK, egui::Color32::WHITE, alpha);

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
                        painter.line_segment([prev, p], egui::Stroke::new(2.0, edge_line_col));
                        prev = p;
                    }
                } else {
                painter.line_segment([a_start, a_end], egui::Stroke::new(2.0, edge_line_col));
                }

                let b_end = rect_from_inside_toward(dst_rect_s, pill_center_s);
                let b_start = rect_from_outside_toward_center(pill_rect_s, b_end);
                painter.line_segment([b_start, b_end], egui::Stroke::new(2.0, edge_line_col));

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
                    edge_col,
                egui::Stroke::new(0.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 0)),
                ));

                let rounding = egui::CornerRadius::same((pill_size_s.y * 0.5).round() as u8);
                painter.rect(
                    pill_rect_s,
                    rounding,
                    pill_fill_col,
                    egui::Stroke::new(1.0, edge_col),
                    egui::StrokeKind::Outside,
                );
                // Inline rename for edge pills
                let edit_rect = pill_rect_s.shrink2(egui::vec2(6.0 * zoom, 4.0 * zoom));
                draw_label_or_inline_editor(
                    ui,
                    workspace,
                    doc_id,
                    id,
                    edit_rect,
                    &painter,
                    pill_rect_s.center(),
                    egui::Align2::CENTER_CENTER,
                &ev.label,
                    &font_id,
                    pill_text_col,
                );
        }
    }

    // (old extra sizing pass removed; handled above)
    // (old extra sizing pass removed; handled above)
    // Edge-build interaction and preview rendering
    let mut _edge_cancel = false;
    let mut _open_edge_menu: Option<EdgeMenuState> = None;
    let mut _stop_dashed_build = false;
    if let Some(build) = workspace.edge_build.as_mut() {
        if build.doc == doc_id {
            // Determine preview end point: hovered node center (if valid) or cursor
            let cursor_opt = ui.ctx().input(|i| i.pointer.hover_pos());
            let end_screen = cursor_opt.unwrap_or(rect.center());
            let mut snap_to_target: Option<EntityId> = None;
            if let Some(hid) = hovered_entity {
                if !doc.scene.edges.contains_key(&hid) && hid != build.source {
                        // Do not snap the preview; only record a valid target for click commit
                        snap_to_target = Some(hid);
                }
            }
            // Draw line from source rect toward end
            if let Some(src_view) = doc.scene.states.get(&build.source) {
                let src_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(src_view.rect.min), doc.transform.to_screen(src_view.rect.max));
                let a_start = rect_from_inside_toward(src_rect_s, end_screen);
                let color = egui::Color32::from_rgb(110, 190, 255);
                // Dashed preview line
                let total: f32 = (end_screen - a_start).length();
                if total > 0.0 {
                    let dir = (end_screen - a_start) / total;
                    let dash: f32 = 8.0;
                    let gap: f32 = 6.0;
                    let mut t: f32 = 0.0;
                    while t < total {
                        let seg_len = dash.min(total - t);
                        let p0 = a_start + dir * t;
                        let p1 = a_start + dir * (t + seg_len);
                        ui.painter().line_segment([p0, p1], egui::Stroke::new(2.0, color));
                        t += dash + gap;
                    }
                    // Arrowhead at cursor end
                    let arrow_len = 10.0 * zoom;
                    let arrow_w = 8.0 * zoom;
                    let tip = end_screen;
                    let base = tip - dir * arrow_len;
                    let perp = egui::pos2(-dir.y, dir.x);
                    let left = base + perp.to_vec2() * (arrow_w * 0.5);
                    let right = base - perp.to_vec2() * (arrow_w * 0.5);
                    ui.painter().add(egui::Shape::convex_polygon(
                        vec![tip, left, right],
                        color,
                        egui::Stroke::new(0.0, egui::Color32::TRANSPARENT),
                    ));
                }
            }
            // Cancel on right-click or Esc
            let cancel = ui.input(|i| i.pointer.secondary_clicked() || i.key_pressed(egui::Key::Escape));
            if cancel {
                _edge_cancel = true;
            }
            // On primary click: open menu if snapped to a valid target, else cancel
            let suppress_primary = if build.just_started { build.just_started = false; true } else { false };
            if !suppress_primary && ui.input(|i| i.pointer.primary_clicked()) {
                if let (Some(cursor), Some(target)) = (cursor_opt, snap_to_target) {
                    _open_edge_menu = Some(EdgeMenuState { doc: doc_id, source: build.source, target, pos: cursor, just_opened: true, filter: String::new() });
                    _stop_dashed_build = true;
                } else {
                    _edge_cancel = true;
                }
            }
        }
    }
    if _edge_cancel { workspace.edge_build = None; workspace.edge_menu = None; }
    if let Some(m) = _open_edge_menu.take() { workspace.edge_menu = Some(m); }
    if _stop_dashed_build { workspace.edge_build = None; }
    // When an edge-target is chosen but menu not necessarily open, draw solid preview until selection
    if let Some(menu) = workspace.edge_menu.clone() {
        if menu.doc == doc_id {
            if let (Some(src_view), Some(dst_view)) = (doc.scene.states.get(&menu.source), doc.scene.states.get(&menu.target)) {
                let src_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(src_view.rect.min), doc.transform.to_screen(src_view.rect.max));
                let dst_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(dst_view.rect.min), doc.transform.to_screen(dst_view.rect.max));
                let start = rect_from_inside_toward(src_rect_s, dst_rect_s.center());
                let end = rect_from_inside_toward(dst_rect_s, src_rect_s.center());
                let color = egui::Color32::from_rgb(160, 220, 255);
                ui.painter().line_segment([start, end], egui::Stroke::new(2.0, color));
                let dir = (end - start).normalized();
                let arrow_len = 10.0 * zoom;
                let arrow_w = 8.0 * zoom;
                let tip = end;
                let base = tip - dir * arrow_len;
                let perp = egui::pos2(-dir.y, dir.x);
                let left = base + perp.to_vec2() * (arrow_w * 0.5);
                let right = base - perp.to_vec2() * (arrow_w * 0.5);
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![tip, left, right],
                    color,
                    egui::Stroke::new(0.0, egui::Color32::TRANSPARENT),
                ));
            }
            // Popup edge-kind chooser at the cursor position on left-click
            let popup_id = egui::Id::new(("edge_kind_menu", doc_id)).with(menu.target);
            let mut filter_buf: String = menu.filter.clone();
            let popup = egui::Area::new(popup_id)
                .order(egui::Order::Foreground)
                .fixed_pos(menu.pos)
                .show(ui.ctx(), |menu_ui| {
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgba_premultiplied(30, 30, 35, 230))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(80)))
                        .corner_radius(egui::CornerRadius::same(6))
                        .show(menu_ui, |menu_ui| {
                            let w = 220.0;
                            menu_ui.with_layout(egui::Layout::top_down(egui::Align::Center), |menu_ui| {
                                menu_ui.set_min_width(w);
                                if menu_ui.add_sized(egui::vec2(w, 24.0), egui::Button::new("Always")).clicked() {
                                    workspace.pending_edge_create = Some(crate::editor::workspace::PendingEdgeCreate { doc: doc_id, source: menu.source, target: menu.target, kind: "Always".to_string() });
                                    workspace
                                        .preview_edges
                                        .retain(|pe| !(pe.doc == doc_id && pe.source == menu.source && pe.target == menu.target));
                                    workspace.edge_menu = None;
                                    return;
                                }
                                menu_ui.separator();
                                // Search bar
                                let te = egui::TextEdit::singleline(&mut filter_buf).hint_text("Search events...");
                                let _ = menu_ui.add_sized(egui::vec2(w, 24.0), te);
                                menu_ui.add_space(4.0);
                                // Scrollable list of variants with max height
                                egui::ScrollArea::vertical()
                                    .max_height(220.0)
                                    .show(menu_ui, |menu_ui| {
                                        let mut items: Vec<String> = workspace.available_event_edges.clone();
                                        if !filter_buf.trim().is_empty() {
                                            let q = filter_buf.to_lowercase();
                                            items.retain(|s| s.to_lowercase().contains(&q));
                                        }
                                        for label in items.into_iter() {
                                            if menu_ui.add_sized(egui::vec2(w, 24.0), egui::Button::new(&label)).clicked() {
                                                workspace.pending_edge_create = Some(crate::editor::workspace::PendingEdgeCreate { doc: doc_id, source: menu.source, target: menu.target, kind: label.clone() });
                                                workspace
                                                    .preview_edges
                                                    .retain(|pe| !(pe.doc == doc_id && pe.source == menu.source && pe.target == menu.target));
                                                workspace.edge_menu = None;
                                                return;
                                            }
                                        }
                                    });
                                menu_ui.separator();
                                if menu_ui.add_sized(egui::vec2(w, 24.0), egui::Button::new("Cancel")).clicked() {
                                    workspace.edge_menu = None;
                                    return;
                                }
                            });
                        });
                });
            // Persist search filter across frames
            if let Some(m) = workspace.edge_menu.as_mut() { if m.doc == doc_id && m.target == menu.target { m.filter = filter_buf; } }
            // Close the popup on outside click, with one-frame suppression right after opening
            if ui.input(|i| i.pointer.any_pressed()) {
                if menu.just_opened {
                    if let Some(m) = workspace.edge_menu.as_mut() { m.just_opened = false; }
                } else {
                    let pos_opt = ui.ctx().input(|i| i.pointer.hover_pos());
                    let inside = pos_opt.map(|p| popup.response.rect.contains(p)).unwrap_or(false);
                    if !inside { workspace.edge_menu = None; }
                }
            }
        }
    }

    // Draw persisted preview edges (solid line with arrowhead)
    if doc.graph.is_some() {
        for pe in workspace.preview_edges.iter().filter(|pe| pe.doc == doc_id) {
            let Some(src_view) = doc.scene.states.get(&pe.source) else { continue };
            let Some(dst_view) = doc.scene.states.get(&pe.target) else { continue };
            let src_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(src_view.rect.min), doc.transform.to_screen(src_view.rect.max));
            let dst_rect_s = egui::Rect::from_min_max(doc.transform.to_screen(dst_view.rect.min), doc.transform.to_screen(dst_view.rect.max));
            let start = rect_from_inside_toward(src_rect_s, dst_rect_s.center());
            let end = rect_from_inside_toward(dst_rect_s, src_rect_s.center());
            let color = egui::Color32::from_rgb(160, 220, 255);
            ui.painter().line_segment([start, end], egui::Stroke::new(2.0, color));
            let dir = (end - start).normalized();
            let arrow_len = 10.0 * zoom;
            let arrow_w = 8.0 * zoom;
            let tip = end;
            let base = tip - dir * arrow_len;
            let perp = egui::pos2(-dir.y, dir.x);
            let left = base + perp.to_vec2() * (arrow_w * 0.5);
            let right = base - perp.to_vec2() * (arrow_w * 0.5);
            ui.painter().add(egui::Shape::convex_polygon(
                vec![tip, left, right],
                color,
                egui::Stroke::new(0.0, egui::Color32::TRANSPARENT),
            ));
        }
    }

    // Zoom at cursor with scroll, but suppress when UI is consuming pointer input (e.g., scrolling menus)
    let scroll_y = ui.ctx().input(|i| i.smooth_scroll_delta.y);
    if scroll_y != 0.0 && !ui.ctx().wants_pointer_input() {
        let scroll: f32 = scroll_y;
        if scroll.abs() > 0.0 {
            let factor = 1.0 + (-scroll * 0.001);
            let cursor = ui.ctx().input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
            let cursor = cursor.clamp(rect.min, rect.max);
            doc.transform.zoom_around_screen_point(factor, cursor);
        }
    }

    context_menu_selection
}

/// Helper: draw a label normally, or an inline text editor if this entity is being renamed.
/// Commits on Enter (records pending_rename_commit) and cancels on click outside.
fn draw_label_or_inline_editor(
    ui: &mut egui::Ui,
    workspace: &mut Workspace,
    doc_id: EntityId,
    target_id: &EntityId,
    edit_rect: egui::Rect,
    painter: &egui::Painter,
    text_pos: egui::Pos2,
    align: egui::Align2,
    label: &str,
    font_id: &egui::FontId,
    color: egui::Color32,
) {
    let is_renaming = workspace.rename_inline.as_ref().map(|r| r.doc == doc_id && r.target == *target_id).unwrap_or(false);
    if is_renaming {
        // Work on a local buffer, then write back depending on commit/cancel
        let mut buf = workspace.rename_inline.as_ref().map(|r| r.text.clone()).unwrap_or_else(|| label.to_string());
        let mut edited = false;
        let mut cancelled = false;
        let _resp_te = ui.put(edit_rect, egui::TextEdit::singleline(&mut buf));
        // Commit on Enter while focused
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) { edited = true; }
        // Cancel only if user clicks outside the edit rect
        let clicked_outside = ui.input(|i| i.pointer.any_pressed()) && !ui.rect_contains_pointer(edit_rect);
        if clicked_outside { cancelled = true; }
        if edited {
            workspace.pending_rename_commit = Some(RenameInline { doc: doc_id, target: *target_id, text: buf.clone() });
            workspace.rename_inline = None;
        } else if cancelled {
            workspace.rename_inline = None;
        } else {
            // Persist ongoing edit
            workspace.rename_inline = Some(RenameInline { doc: doc_id, target: *target_id, text: buf });
        }
    } else {
        painter.text(text_pos, align, label, font_id.clone(), color);
    }
}

fn is_direct_substate_of_parallel(doc: &GraphDoc, child_id: &EntityId) -> bool {
    let parent_opt = doc.scene.tree.parent_of.get(child_id).and_then(|p| *p);
    let by_view = parent_opt
        .and_then(|pid| doc.scene.states.get(&pid))
        .map(|pv| matches!(pv.kind, StateKind::Parallel))
        .unwrap_or(false);
    if by_view { return true; }
    if let (Some(graph), Some(pid)) = (&doc.graph, parent_opt) {
        if let Some(parent_node) = graph.nodes.get(&pid) {
            let has_initial = parent_node.components.contains(bevy_gearbox_protocol::components::INITIAL_STATE);
            let has_children = !parent_node.children.is_empty();
            if has_children && !has_initial { return true; }
        }
    }
    false
}



