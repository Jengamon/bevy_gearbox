use bevy_egui::egui;
use crate::editor::panels;
use crate::editor::model::store::EditorStore;

pub fn draw(ui: &mut egui::Ui, store: &mut EditorStore) {
    ui.horizontal(|ui| {
        // Left column: connection + explorer
        ui.vertical(|ui| {
            panels::connection::draw(ui, store);
            ui.separator();
            panels::explorer::draw(ui, store);
        });
        ui.separator();
        // Center: tabs placeholder
        ui.vertical(|ui| {
            ui.heading("Editor Tabs (placeholder)");
        });
        ui.separator();
        // Right: inspector placeholder
        ui.vertical(|ui| {
            ui.heading("Inspector (placeholder)");
        });
    });
}


