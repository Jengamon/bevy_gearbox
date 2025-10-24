use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use plugin::EditorPlugin;

mod net;
mod rpcs;
mod client;
mod plugin;
mod component;
mod types;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .add_plugins(EditorPlugin)
        .run();
}

