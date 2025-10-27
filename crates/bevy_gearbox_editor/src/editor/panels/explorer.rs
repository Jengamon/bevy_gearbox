use bevy::prelude::*;
use bevy_egui::egui;
use crate::editor::model::store::EditorStore;
use crate::editor::actions::{RefreshIndexRequested, OpenRequested};

pub fn draw(ui: &mut egui::Ui, store: &mut EditorStore, commands: &mut Commands) {
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.text_edit_singleline(&mut store.index.filter.query);
        if ui.button("Refresh").clicked() {
            commands.trigger(RefreshIndexRequested { query: store.index.filter.query.clone() });
            store.index.is_loading = true;
        }
    });
    ui.separator();
    if store.index.is_loading { ui.label("Loading..."); return; }
    if let Some(err) = &store.index.error { ui.colored_label(egui::Color32::RED, err); }
    let mut to_open: Option<crate::types::ServerEntity> = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        for item in store.index.items.clone().into_iter() {
            ui.horizontal(|ui| {
                let title = match &item.name {
                    Some(name) => format!("{}  ({})", name, item.entity),
                    None => format!("{}", item.entity),
                };
                if ui.button(title).clicked() { to_open = Some(item.entity); }
            });
        }
    });
    if let Some(entity) = to_open { commands.trigger(OpenRequested { entity }); }
}


