use bevy::prelude::*;
use bevy_egui::egui;
use crate::editor::model::store::EditorStore;
use crate::editor::actions::{RefreshIndexRequested, OpenRequested};

pub fn draw(ui: &mut egui::Ui, store: &mut EditorStore, commands: &mut Commands) {
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut store.index.filter.query);
    });
    ui.separator();
    if store.index.is_loading { ui.label("Loading..."); return; }
    if let Some(err) = &store.index.error { ui.colored_label(egui::Color32::RED, err); }
    let mut to_open: Option<crate::types::EntityId> = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        let query = store.index.filter.query.trim().to_lowercase();
        let iter = store.index.items.iter().cloned().filter(|it| {
            if query.is_empty() { return true; }
            let name = it.name.as_deref().unwrap_or("");
            let id_text = it.entity.to_string();
            name.to_lowercase().contains(&query) || id_text.to_lowercase().contains(&query)
        });
        for item in iter {
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


