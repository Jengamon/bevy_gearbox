use bevy::prelude::*;
use bevy_egui::egui;
use crate::editor::model::types::ConnectionState;
use crate::editor::actions::{ConnectRequested, DisconnectRequested, ReconnectRequested};
use crate::editor::model::store::EditorStore;

pub fn draw(ui: &mut egui::Ui, store: &mut EditorStore, commands: &mut Commands) {
    ui.horizontal(|ui| {
        let mut endpoint = match &store.connection {
            ConnectionState::Connected { endpoint, .. } => endpoint.clone(),
            _ => String::from("http://127.0.0.1:15703"),
        };
        ui.label("Endpoint");
        ui.text_edit_singleline(&mut endpoint);
        let is_connected = matches!(store.connection, ConnectionState::Connected { .. });
        if ui.button(if is_connected { "Disconnect" } else { "Connect" }).clicked() {
            if is_connected { commands.trigger(DisconnectRequested); }
            else { commands.trigger(ConnectRequested { endpoint }); }
        }
        if is_connected {
            if ui.button("Reconnect").clicked() {
                commands.trigger(ReconnectRequested);
            }
        }
    });
}


