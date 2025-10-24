use bevy::prelude::*;
use bevy_egui::EguiPlugin;
mod connection;
mod rpcs;
mod client;
mod plugin;
mod component;
use plugin::EditorPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .add_plugins(EditorPlugin)
        .run();
}

