use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::collections::HashMap;

use crate::connection::{Command, Event, NetCtx};

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(UiState {
                url: std::env::var("BRP_URL").unwrap_or_else(|_| "http://127.0.0.1:15703".to_string()),
                connecting: false,
                error: None,
                machines: vec![],
                graphs: HashMap::new(),
            })
            .add_systems(Startup, setup_netctx)
            .add_systems(Update, poll_network);

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
    url: String,
    connecting: bool,
    error: Option<String>,
    machines: Vec<(u32, Option<String>)>,
    graphs: HashMap<u32, String>,
}

fn setup_netctx(mut commands: Commands) {
    commands.spawn(Camera2d);
    let conn = crate::connection::spawn();
    commands.insert_resource(conn);
}

fn poll_network(mut ui: ResMut<UiState>, net: Res<NetCtx>) {
    loop {
        let evt = {
            let guard = net.rx.lock().unwrap();
            guard.try_recv()
        };
        match evt {
            Ok(Event::RefreshResult(Ok(machines))) => {
                ui.machines = machines;
                ui.connecting = false;
                ui.error = None;
                for (id, _) in ui.machines.clone() {
                    let _ = net
                        .tx
                        .lock()
                        .unwrap()
                        .send(Command::FetchGraph { url: ui.url.clone(), id });
                }
            }
            Ok(Event::RefreshResult(Err(e))) => {
                ui.connecting = false;
                ui.error = Some(e);
            }
            Ok(Event::GraphResult { id, result }) => {
                if let Ok(text) = result {
                    ui.graphs.insert(id, text);
                }
            }
            Ok(Event::SelectResult(Err(e))) => {
                ui.error = Some(format!("Select failed: {e}"));
            }
            Ok(Event::SaveResult(Err(e))) => {
                ui.error = Some(format!("Save failed: {e}"));
            }
            Ok(_) => {}
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        }
    }
}

fn ui_system(mut egui_ctx: EguiContexts, mut ui: ResMut<UiState>, net: Res<NetCtx>) {
    if let Ok(ctx) = egui_ctx.ctx_mut() {
        egui::CentralPanel::default().show(ctx, |ui_egui| {
            ui_egui.vertical(|col| {
                col.heading("Bevy Gearbox Remote (minimal)");
                col.add_space(8.0);
                col.horizontal(|row| {
                    let text = egui::TextEdit::singleline(&mut ui.url).desired_width(380.0);
                    row.add(text);
                    let btn = row.add_enabled(!ui.connecting, egui::Button::new("Connect / Refresh"));
                    if btn.clicked() {
                        ui.connecting = true;
                        ui.error = None;
                        let _ = net.tx.lock().unwrap().send(Command::Refresh(ui.url.clone()));
                    }
                    if let Some(err) = &ui.error {
                        row.colored_label(egui::Color32::from_rgb(176, 0, 0), err);
                    }
                });
                col.separator();
                col.heading("State Machines");
                col.add_space(4.0);
                for (id, name) in ui.machines.clone() {
                    col.horizontal(|row| {
                        let display = name.unwrap_or_else(|| "<unnamed>".to_string());
                        row.add_sized([260.0, 20.0], egui::Label::new(display));
                        row.label(format!("{}", id));
                        if row.button("Select").clicked() {
                            let _ = net.tx.lock().unwrap().send(Command::Select { url: ui.url.clone(), id });
                        }
                        if row.button("Save").clicked() {
                            let _ = net.tx.lock().unwrap().send(Command::Save { url: ui.url.clone(), id });
                        }
                    });
                    if let Some(text) = ui.graphs.get(&id) {
                        col.add_space(2.0);
                        col.code(text);
                        col.add_space(6.0);
                    }
                }
            });
        });
    }
}


