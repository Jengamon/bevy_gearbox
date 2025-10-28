use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use plugin::EditorPlugin;

mod persistence;
mod plugin;
mod types;
mod util;
mod model;
mod editor;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .add_plugins(EditorPlugin)
        .run();
}

