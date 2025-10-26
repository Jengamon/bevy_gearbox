use bevy_egui::egui;
use crate::editor::model::store::EditorStore;
use crate::editor::actions;

pub fn draw(ui: &mut egui::Ui, store: &mut EditorStore) {
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.text_edit_singleline(&mut store.index.filter.query);
        if ui.button("Refresh").clicked() {
            let filter = store.index.filter.clone();
            actions::refresh_index(store, filter);
        }
    });
    ui.separator();
    if store.index.is_loading { ui.label("Loading..."); return; }
    if let Some(err) = &store.index.error { ui.colored_label(egui::Color32::RED, err); }
    let mut to_open: Option<crate::types::ServerEntity> = None;
    for item in store.index.items.clone().into_iter() {
        ui.horizontal(|ui| {
            let title = item.name.clone().unwrap_or_else(|| format!("{}", item.entity.0));
            if ui.button(title).clicked() { to_open = Some(item.entity); }
        });
    }
    if let Some(entity) = to_open { actions::open_machine(store, entity); }
}


