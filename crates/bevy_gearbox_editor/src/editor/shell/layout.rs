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
                    // Ensure a doc entry exists for drawing
                    let mut sel_local = workspace.selection.clone();
                    let mut menu_local = workspace.menu.clone();
                    if let Some(entry) = workspace.docs.get_mut(&active) {
                        let _ = crate::editor::view::draw_doc(ui, entry, &mut sel_local, active, &mut menu_local);
                    } else {
                        let entry = workspace.docs.entry(active).or_default();
                        let _ = crate::editor::view::draw_doc(ui, entry, &mut sel_local, active, &mut menu_local);
                    }
                    workspace.selection = sel_local;
                    workspace.menu = menu_local;
                } else {
                    ui.label("No document open. Select a state machine from the left.");
                }
            });
        });
    });
}


