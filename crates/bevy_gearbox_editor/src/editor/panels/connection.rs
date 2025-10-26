use bevy_egui::egui;
use crate::editor::model::types::ConnectionState;
use crate::editor::actions::{self, EndpointConfig};
use crate::editor::model::store::EditorStore;

pub fn draw(ui: &mut egui::Ui, store: &mut EditorStore) {
    ui.horizontal(|ui| {
        let mut endpoint = match &store.connection {
            ConnectionState::Connected { endpoint, .. } => endpoint.clone(),
            _ => String::from("ws://127.0.0.1:9000"),
        };
        ui.label("Endpoint");
        ui.text_edit_singleline(&mut endpoint);
        let is_connected = matches!(store.connection, ConnectionState::Connected { .. });
        if ui.button(if is_connected { "Disconnect" } else { "Connect" }).clicked() {
            if is_connected { actions::disconnect(store); }
            else { actions::connect(store, EndpointConfig { endpoint }); }
        }
        if ui.button("Reconnect").clicked() {
            actions::reconnect(store);
        }
    });
}


