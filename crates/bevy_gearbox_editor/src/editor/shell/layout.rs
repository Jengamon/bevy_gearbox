use bevy::prelude::Commands;
use bevy_egui::egui;
use crate::editor::panels;
use crate::editor::model::store::EditorStore;
use crate::editor::workspace::Workspace;

pub fn draw(ui: &mut egui::Ui, store: &mut EditorStore, commands: &mut Commands, workspace: &mut Workspace) {
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
                panels::explorer::draw(ui, store, commands);
            });

            ui.separator();

            // Canvas: show active document only
            let desired_canvas = ui.available_size_before_wrap();
            let _resp = ui.allocate_ui_with_layout(desired_canvas, egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_size(desired_canvas);

                // Choose an active doc if none set but some are open
                if store.active_doc.is_none() {
                    if let Some(first) = store.open_docs.keys().next().cloned() {
                        store.active_doc = Some(first);
                    }
                }

                if let Some(active) = store.active_doc {
                    // Ensure a doc entry exists, then temporarily remove it to avoid borrow conflicts
                    workspace.docs.entry(active).or_default();
                    let mut sel_local = workspace.selection.clone();
                    let mut menu_local = workspace.menu.clone();
                    let mut entry = workspace.docs.remove(&active).unwrap_or_default();
                    if let Some(selection) = crate::editor::view::draw_doc(ui, &mut entry, &mut sel_local, active, &mut menu_local, workspace) {
                        match selection {
                            crate::editor::context_menu::MenuSelection::SaveStateMachine { .. } => {
                                commands.trigger(crate::editor::actions::SaveAsRequested { entity: active });
                            }
                            crate::editor::context_menu::MenuSelection::RenameEntity { target } => {
                                // Seed inline edit with current display name or current label
                                let mut default_text = String::new();
                                if let Some(g) = &entry.graph { if let Some(n) = g.nodes.get(&target) { if let Some(name) = &n.display_name { default_text = name.clone(); } } }
                                if default_text.is_empty() { if let Some(v) = entry.views.get(&target) { default_text = v.label.clone(); } }
                                workspace.rename_inline = Some(crate::editor::workspace::RenameInline { doc: active, target, text: default_text });
                            }
                            _ => {}
                        }
                    }
                    // Insert the possibly modified entry back
                    workspace.docs.insert(active, entry);
                    // Global commit handler
                    if let Some(commit) = workspace.pending_rename_commit.take() {
                        if let crate::model::EntityId::Server(sid) = commit.target {
                            let e = bevy::prelude::Entity::from_bits(sid.0);
                            commands.trigger(bevy_gearbox_protocol::events::Rename { target: e, name: commit.text.clone() });
                        }
                        // No optimistic UI mutation; wait for watch-driven update
                    }
                    workspace.selection = sel_local;
                    workspace.menu = menu_local;
                } else {
                    // no-op: avoid chatty log
                    ui.label("No document open. Select a state machine from the left.");
                }
            });
        });
    });
}


