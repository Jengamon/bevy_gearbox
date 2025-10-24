use bevy::prelude::*;
use bevy_egui::EguiPlugin;
mod connection;
mod rpcs;
mod plugin;
use plugin::EditorPlugin;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .add_plugins(EditorPlugin)
        .run();
}

#[cfg(target_arch = "wasm32")]
fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .add_plugins(EditorPlugin)
        .run();
}
 
