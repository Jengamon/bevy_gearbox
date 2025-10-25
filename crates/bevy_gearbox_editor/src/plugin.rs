use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::collections::HashMap;

use crate::net::{NetPlugin, NetCommand, NetEvent, NetworkConfig};
use crate::types::ServerEntity;
use crate::model::StateMachineGraph;

pub(crate) struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(NetPlugin)
            .insert_resource(UiState {
                url_edit: String::new(),
                connecting: false,
                error: None,
                machines: vec![],
                graphs: HashMap::new(),
            })
            .add_systems(Startup, setup_camera)
            .add_systems(Update, poll_network);

        use bevy_egui::EguiPrimaryContextPass;
        app.add_systems(EguiPrimaryContextPass, ui_system);
    }
}

#[derive(Resource, Clone)]
struct UiState {
    url_edit: String,
    connecting: bool,
    error: Option<String>,
    machines: Vec<(ServerEntity, Option<String>)>,
    graphs: HashMap<ServerEntity, StateMachineGraph>,
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn poll_network(
    mut ui: ResMut<UiState>,
    mut events: MessageReader<NetEvent>,
    mut cmd_writer: MessageWriter<NetCommand>,
) {
    let mut processed = 0usize;
    const MAX_PER_FRAME: usize = 64;
    for evt in events.read() {
        if processed >= MAX_PER_FRAME { break; }
        match evt {
            NetEvent::RefreshResult(Ok(machines)) => {
                ui.machines = machines.iter().map(|m| (m.id, m.name.clone())).collect();
                ui.connecting = false;
                ui.error = None;
                for (id, _) in ui.machines.iter() {
                    cmd_writer.write(NetCommand::FetchGraph { id: *id });
                }
                processed += 1;
            }
            NetEvent::RefreshResult(Err(e)) => {
                ui.connecting = false;
                ui.error = Some(e.to_string());
                processed += 1;
            }
            NetEvent::GraphResult { id, result } => {
                if let Ok(graph) = result {
                    ui.graphs.insert(*id, graph.clone());
                }
                processed += 1;
            }
            NetEvent::SelectResult(Err(e)) => {
                ui.error = Some(format!("Select failed: {}", e));
                processed += 1;
            }
            NetEvent::SaveResult(Err(e)) => {
                ui.error = Some(format!("Save failed: {}", e));
                processed += 1;
            }
            _ => {}
        }
    }
}

fn ui_system(
    mut egui_ctx: EguiContexts,
    mut ui: ResMut<UiState>,
    mut cmd_writer: MessageWriter<NetCommand>,
    cfg: Res<NetworkConfig>,
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
                        cmd_writer.write(NetCommand::SetUrl { url: ui.url_edit.clone() });
                        cmd_writer.write(NetCommand::Refresh);
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
                            cmd_writer.write(NetCommand::Select { id: *id });
                        }
                        if row.button("Save").clicked() {
                            cmd_writer.write(NetCommand::Save { id: *id });
                        }
                    });
                    if let Some(graph) = ui.graphs.get(id) {
                        col.add_space(2.0);
                        col.code(format!("{}", graph));
                        col.add_space(6.0);
                    }
                }
            });
        });
    }
}


