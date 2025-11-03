use bevy::prelude::Commands;
use bevy_egui::egui;
use crate::editor::panels;
use crate::editor::model::store::EditorStore;
use crate::editor::workspace::Workspace;
use crate::editor::docs::Docs;

pub fn draw(ui: &mut egui::Ui, store: &mut EditorStore, commands: &mut Commands, workspace: &mut Workspace, docs: &mut Docs) {
    ui.vertical(|ui| {
        // Top bar: connection controls across entire app
        ui.horizontal(|ui| {
            panels::connection::draw(ui, store, commands);
        });
        ui.add_space(4.0);

        ui.separator();

        // Below: left panel + canvas, sized to fill remaining height
        let available_size = egui::vec2(ui.available_width(), ui.available_height());
        let _resp = ui.allocate_ui_with_layout(available_size, egui::Layout::left_to_right(egui::Align::Min), |ui| {
            // Left panel: fixed width; search + remote state machines
            let left_width = 260.0;
            let desired_left = egui::vec2(left_width, ui.available_height());
            let _resp = ui.allocate_ui_with_layout(desired_left, egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(left_width);
                ui.heading("Explorer");
                panels::explorer::draw(ui, store, commands, workspace, docs);
            });

            ui.separator();

            // Canvas: single shared board where all open documents are drawn together
            let desired_canvas = ui.available_size_before_wrap();
            let _resp = ui.allocate_ui_with_layout(desired_canvas, egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_size(desired_canvas);
                let board_size = ui.available_size_before_wrap();
                let (board_rect, board_resp) = ui.allocate_exact_size(board_size, egui::Sense::click_and_drag());
                let board_painter = ui.painter_at(board_rect);
                board_painter.rect_filled(board_rect, 0.0, egui::Color32::from_gray(20));

                // Reset board-level once-per-frame guards
                workspace.board_pan_applied = false;

                // Board-level zoom: apply wheel zoom to all documents once per frame and persist to workspace.board_transform
                let scroll_y = ui.ctx().input(|i| i.smooth_scroll_delta.y);
                if scroll_y != 0.0 && !ui.ctx().wants_pointer_input() {
                    let scroll: f32 = scroll_y;
                    if scroll.abs() > 0.0 {
                        let factor = 1.0 + (-scroll * 0.001);
                        let cursor = ui.ctx().input(|i| i.pointer.hover_pos()).unwrap_or(board_rect.center());
                        let cursor = cursor.clamp(board_rect.min, board_rect.max);
                        for (_id, doc) in docs.map.iter_mut() {
                            doc.transform.zoom_around_screen_point(factor, cursor);
                        }
                        workspace.board_transform.zoom_around_screen_point(factor, cursor);
                    }
                }

                if docs.map.is_empty() {
                    let hint = "No document open. Select a state machine from the left.";
                    board_painter.text(board_rect.center(), egui::Align2::CENTER_CENTER, hint, egui::FontId::proportional(14.0), egui::Color32::from_gray(160));
                } else {
                    let mut ids: Vec<crate::types::EntityId> = docs.map.keys().copied().collect();
                    ids.sort_by_key(|e| e.0);
                    for doc_id in ids {
                        let mut entry = docs.map.remove(&doc_id).unwrap_or_default();
                        // Local view of global selection for this doc
                        let prev_global = workspace.global_selection;
                        let mut sel_local = prev_global.and_then(|(d, t)| if d == doc_id { Some(t) } else { None });
                        if let Some(selection) = crate::editor::view::draw_doc_on_board(ui, board_rect, &board_resp, &mut entry, &mut sel_local, doc_id, workspace, docs, false) {
                            match selection {
                                crate::editor::context_menu::MenuSelection::SaveStateMachine { target } => {
                                    commands.trigger(crate::editor::actions::SaveAsRequested { doc: doc_id, target });
                                }
                                crate::editor::context_menu::MenuSelection::SaveSubstates { target } => {
                                    commands.trigger(crate::editor::actions::SaveSubstatesRequested { target });
                                }
                                crate::editor::context_menu::MenuSelection::RenameEntity { target } => {
                                    let mut default_text = String::new();
                                    if let Some(g) = &entry.graph { default_text = g.get_display_name(&target); }
                                    if default_text.is_empty() {
                                        if let Some(v) = entry.scene.states.get(&target) { default_text = v.label.clone(); }
                                        else if let Some(v) = entry.scene.edges.get(&target) { default_text = v.label.clone(); }
                                    }
                                    workspace.rename_inline = Some(crate::editor::workspace::RenameInline { doc: doc_id, target, text: default_text });
                                }
                                crate::editor::context_menu::MenuSelection::DeleteEntity { target } => {
                                    let e = bevy::prelude::Entity::from_bits(target.0);
                                    commands.trigger(bevy_gearbox_protocol::events::Despawn { target: e });
                                    workspace.pending_fetch_docs.push(doc_id);
                                }
                                _ => {}
                            }
                        }
                        // Reconcile global selection based on this doc's local selection delta
                        match (prev_global, sel_local) {
                            (_, Some(t)) => workspace.global_selection = Some((doc_id, t)),
                            (Some((d, _)), None) if d == doc_id => workspace.global_selection = None,
                            _ => {}
                        }
                        docs.map.insert(doc_id, entry);
                    }

                    if let Some(commit) = workspace.pending_rename_commit.take() {
                        let e = bevy::prelude::Entity::from_bits(commit.target.0);
                        commands.trigger(bevy_gearbox_protocol::events::Rename { target: e, name: commit.text.clone() });
                    }
                    if let Some(req) = workspace.pending_edge_create.take() {
                        let m = bevy::prelude::Entity::from_bits(req.doc.0);
                        let s = bevy::prelude::Entity::from_bits(req.source.0);
                        let t = bevy::prelude::Entity::from_bits(req.target.0);
                        commands.trigger(bevy_gearbox_protocol::events::CreateTransition { machine: m, source: s, target: t, kind: req.kind.clone() });
                        workspace.pending_fetch_docs.push(req.doc);
                    }
                }
            });
        });
    });
}


