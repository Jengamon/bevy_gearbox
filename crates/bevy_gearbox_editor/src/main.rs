use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin};
mod connection;
use crate::connection::{Command, Event, NetCtx};

#[derive(Resource, Clone)]
struct UiState {
    url: String,
    connecting: bool,
    error: Option<String>,
    machines: Vec<(u32, Option<String>)>,
}

// Network logic lives in `connection` module.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use bevy_egui::EguiPrimaryContextPass;

    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .insert_resource(UiState {
            url: std::env::var("BRP_URL").unwrap_or_else(|_| "http://127.0.0.1:15703".to_string()),
            connecting: false,
            error: None,
            machines: vec![],
        })
        .add_systems(Startup, setup_netctx)
        .add_systems(Update, poll_network)
        .add_systems(EguiPrimaryContextPass, ui_system)
        .run();
}

#[cfg(target_arch = "wasm32")]
fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .insert_resource(UiState {
            url: std::env::var("BRP_URL").unwrap_or_else(|_| "http://127.0.0.1:15703".to_string()),
            connecting: false,
            error: None,
            machines: vec![],
        })
        .add_systems(Startup, setup_netctx)
        .add_systems(Update, poll_network)
        .add_systems(Update, ui_system.in_set(EguiSet::Ui))
        .run();
}

fn setup_netctx(mut commands: Commands) {
    commands.spawn(Camera2d);
    let conn = connection::spawn();
    commands.insert_resource(conn);
}

fn poll_network(mut ui: ResMut<UiState>, net: Res<NetCtx>) {
    // Drain queued events
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
            }
            Ok(Event::RefreshResult(Err(e))) => {
                ui.connecting = false;
                ui.error = Some(e);
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
                // URL row
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
                }
            });
        });
    }
}
