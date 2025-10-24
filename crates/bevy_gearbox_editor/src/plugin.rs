use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::collections::HashMap;

use crate::connection::{Command, Event, PendingTasks, NetworkConfig, ServerEntity, handle_commands, collect_task_results};

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Command>()
            .add_message::<Event>()
            .insert_resource(PendingTasks::default())
            .insert_resource(NetworkConfig { url: std::env::var("BRP_URL").unwrap_or_else(|_| "http://127.0.0.1:15703".to_string()) })
            .insert_resource(UiState {
                url_edit: String::new(),
                connecting: false,
                error: None,
                machines: vec![],
                graphs: HashMap::new(),
            })
            .add_systems(Startup, setup_camera)
            .add_systems(Update, (handle_commands, collect_task_results, poll_network));

        #[cfg(not(target_arch = "wasm32"))]
        {
            use bevy_egui::EguiPrimaryContextPass;
            app.add_systems(EguiPrimaryContextPass, ui_system);
        }

        #[cfg(target_arch = "wasm32")]
        {
            use bevy_egui::EguiSet;
            app.add_systems(Update, ui_system.in_set(EguiSet::Ui));
        }
    }
}

#[derive(Resource, Clone)]
struct UiState {
    url_edit: String,
    connecting: bool,
    error: Option<String>,
    machines: Vec<(ServerEntity, Option<String>)>,
    graphs: HashMap<ServerEntity, String>,
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn poll_network(
    mut ui: ResMut<UiState>,
    mut events: MessageReader<Event>,
    mut cmd_writer: MessageWriter<Command>,
) {
    let mut processed = 0usize;
    const MAX_PER_FRAME: usize = 64;
    for evt in events.read() {
        if processed >= MAX_PER_FRAME { break; }
        match evt {
            Event::RefreshResult(Ok(machines)) => {
                ui.machines = machines.clone();
                ui.connecting = false;
                ui.error = None;
                for (id, _) in ui.machines.iter() {
                    cmd_writer.write(Command::FetchGraph { id: *id });
                }
                processed += 1;
            }
            Event::RefreshResult(Err(e)) => {
                ui.connecting = false;
                ui.error = Some(e.clone());
                processed += 1;
            }
            Event::GraphResult { id, result } => {
                if let Ok(text) = result {
                    ui.graphs.insert(*id, text.clone());
                }
                processed += 1;
            }
            Event::SelectResult(Err(e)) => {
                ui.error = Some(format!("Select failed: {e}"));
                processed += 1;
            }
            Event::SaveResult(Err(e)) => {
                ui.error = Some(format!("Save failed: {e}"));
                processed += 1;
            }
            _ => {}
        }
    }
}

fn ui_system(
    mut egui_ctx: EguiContexts,
    mut ui: ResMut<UiState>,
    mut cmd_writer: MessageWriter<Command>,
    mut cfg: ResMut<NetworkConfig>,
) {
    if let Ok(ctx) = egui_ctx.ctx_mut() {
        egui::CentralPanel::default().show(ctx, |ui_egui| {
            ui_egui.vertical(|col| {
                col.heading("Bevy Gearbox Remote (minimal)");
                col.add_space(8.0);
                col.horizontal(|row| {
                    if ui.url_edit.is_empty() { ui.url_edit = cfg.url.clone(); }
                    let text = egui::TextEdit::singleline(&mut ui.url_edit).desired_width(380.0);
                    row.add(text);
                    let btn = row.add_enabled(!ui.connecting, egui::Button::new("Connect / Refresh"));
                    if btn.clicked() {
                        ui.connecting = true;
                        ui.error = None;
                        cfg.url = ui.url_edit.clone();
                        cmd_writer.write(Command::Refresh);
                    }
                    if let Some(err) = &ui.error {
                        row.colored_label(egui::Color32::from_rgb(176, 0, 0), err);
                    }
                });
                col.separator();
                col.heading("State Machines");
                col.add_space(4.0);
                for (id, name) in ui.machines.iter() {
                    col.horizontal(|row| {
                        let display = name.clone().unwrap_or_else(|| "<unnamed>".to_string());
                        row.add_sized([260.0, 20.0], egui::Label::new(display));
                        row.label(format!("{}", id.0));
                        if row.button("Select").clicked() {
                            cmd_writer.write(Command::Select { id: *id });
                        }
                        if row.button("Save").clicked() {
                            cmd_writer.write(Command::Save { id: *id });
                        }
                    });
                    if let Some(text) = ui.graphs.get(id) {
                        col.add_space(2.0);
                        col.code(text);
                        col.add_space(6.0);
                    }
                }
            });
        });
    }
}


