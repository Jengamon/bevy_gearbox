use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::collections::HashMap;

use crate::net::{NetPlugin, NetCommand, NetEvent, NetworkConfig};
use crate::types::ServerEntity;
use crate::model::StateMachineGraph;
use crate::editor::workspace::Workspace;
use crate::editor::adapter::project_graph_into_doc;
use crate::editor::view::draw_doc;
use crate::editor::context_menu::MenuSelection;
use crate::persistence::{extract_sidecar_from_doc, save_sidecar, apply_sidecar_to_doc, compute_graph_fingerprint, load_sidecar, parse_sidecar_text};
use crate::component as c;

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
                sidecar_texts: HashMap::new(),
            })
            .init_resource::<Workspace>()
            .add_systems(Startup, setup_camera)
            .add_systems(Update, (poll_network, sync_snapshots_to_workspace));

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
    /// Latest sidecar text fetched over RPC per machine (if any)
    sidecar_texts: HashMap<ServerEntity, String>,
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
                    // Stash/refresh snapshot
                    ui.graphs.insert(*id, graph.clone());
                    // If the root has a StateMachineId, request its sidecar via RPC (derive path: <id>.sm.ron)
                    if let Some(id_text) = graph.nodes.get(&graph.root)
                        .and_then(|n| n.components.get(c::STATE_MACHINE_ID))
                        .and_then(|e| e.value_json.as_str())
                    {
                        let path = format!("{}.sm.ron", id_text);
                        cmd_writer.write(NetCommand::FetchSidecarByPath { path, doc: *id });
                    }
                }
                processed += 1;
            }
            NetEvent::SidecarResult(r) => { let _ = r; processed += 1; }
            NetEvent::SidecarResultFor { id, result } => {
                // Cache sidecar text for application during workspace sync
                if let Ok(Some(text)) = result { ui.sidecar_texts.insert(*id, text.clone()); }
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
    mut workspace: ResMut<Workspace>,
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
                    {
                        // Split borrows: copy menu handle separately
                        let mut selected_tmp = workspace.selection.take();
                        let mut menu_handle = workspace.menu.take();
                        if let Some(doc) = workspace.docs.get_mut(id) {
                            col.add_space(4.0);
                            let selection_evt_inner: Option<MenuSelection>;
                            {
                                let response = egui::Frame::canvas(col.style()).show(col, |canvas| {
                                    draw_doc(canvas, doc, &mut selected_tmp, *id, &mut menu_handle)
                                });
                                selection_evt_inner = response.inner;
                            }
                            if let Some(selection_evt) = selection_evt_inner {
                                match selection_evt {
                                    MenuSelection::SaveStateMachine { target: _ } => {
                                        // Prompt for a file path; we'll derive both .scn.ron (remote) and .sm.ron (local)
                                        let default_name = name.clone().unwrap_or_else(|| "state_machine".to_string()).replace('.', "_");
                                        if let Some(chosen) = rfd::FileDialog::new()
                                            .set_title("Save State Machine As")
                                            // Only show scene assets in the picker
                                            .add_filter("Bevy Scene (.scn.ron)", &["scn.ron"])
                                            // Pre-fill name without extension; we'll append .scn.ron on save
                                            .set_file_name(&default_name)
                                            .save_file() {
                                            let stem = chosen.file_stem().and_then(|s| s.to_str()).unwrap_or(&default_name);
                                            // Sanitize base: strip trailing .sm or .scn if present (handles names like app_state.scn.sm)
                                            let mut base = stem.to_string();
                                            if base.ends_with(".sm") { base = base.trim_end_matches(".sm").to_string(); }
                                            if base.ends_with(".scn") { base = base.trim_end_matches(".scn").to_string(); }
                                            let dir = chosen.parent().map(|p| p.to_path_buf()).unwrap_or(std::path::PathBuf::from("."));
                                            let sidecar_path = dir.join(format!("{}.sm.ron", base));
                                            let asset_base = base.clone();
                                            // Kick off remote scene save
                                            cmd_writer.write(NetCommand::SaveAs { id: *id, asset_base: asset_base.clone(), sidecar_path: sidecar_path.clone() });
                                            // Save sidecar immediately from current layout
                                            if let Some(doc_ref) = workspace.docs.get(id) {
                                                let mut sidecar = extract_sidecar_from_doc(doc_ref);
                                                sidecar.scene_basename = Some(format!("{}.scn.ron", asset_base));
                                                let _ = save_sidecar(&sidecar_path, &sidecar);
                                            }
                                        }
                                    }
                                    _ => {
                                        col.label(format!("Menu: {:?}", selection_evt));
                                    }
                                }
                            }
                            col.add_space(6.0);
                        }
                        workspace.selection = selected_tmp;
                        workspace.menu = menu_handle;
                    }
                }
            });
        });
    }
}

fn sync_snapshots_to_workspace(
    mut workspace: ResMut<Workspace>,
    mut ui: ResMut<UiState>,
) {
    let mut consume_sidecar_for: Vec<ServerEntity> = Vec::new();
    for (id, graph) in ui.graphs.iter() {
        let entry = workspace.docs.entry(*id).or_default();
        let was_empty = entry.graph.is_none();
        project_graph_into_doc(entry, graph.clone());
        // Try applying sidecar when: (a) first load, or (b) new sidecar text arrived
        let fp = compute_graph_fingerprint(&graph);
        let mut applied = false;
        if let Some(text) = ui.sidecar_texts.get(id) {
            if let Ok(sc) = parse_sidecar_text(text) {
                if sc.graph_fingerprint.as_deref() == Some(&fp) || sc.graph_fingerprint.is_none() { apply_sidecar_to_doc(entry, &sc); applied = true; }
            }
            // mark for single-consume once attempted (avoid re-applying every frame)
            consume_sidecar_for.push(*id);
        }
        if !applied && was_empty {
            // Fallbacks for local disk resolution for convenience when app and editor share filesystem
            if let Some(id_text) = graph.nodes.get(&graph.root).and_then(|n| n.components.get(c::STATE_MACHINE_ID)).and_then(|e| e.value_json.as_str()) {
                // Derive file name from id
                let ptr_str = format!("{}.sm.ron", id_text);
                let mut tried: Vec<std::path::PathBuf> = Vec::new();
                let candidate_direct = std::path::PathBuf::from(&ptr_str);
                tried.push(candidate_direct.clone());
                let candidate_assets = std::path::PathBuf::from("assets").join(&ptr_str);
                tried.push(candidate_assets.clone());
                for p in tried {
                    if p.exists() {
                        if let Ok(sc) = load_sidecar(&p) { apply_sidecar_to_doc(entry, &sc); applied = true; break; }
                    }
                }
            }
            if !applied {
                if let Some(name) = graph.nodes.get(&graph.root).and_then(|n| n.display_name.clone()) {
                    let stem = name.replace('.', "_");
                    for p in [std::path::PathBuf::from(format!("{}.sm.ron", stem)), std::path::PathBuf::from("assets").join(format!("{}.sm.ron", stem))] {
                        if p.exists() {
                            if let Ok(sc) = load_sidecar(&p) {
                                if sc.graph_fingerprint.as_deref() == Some(&fp) || sc.graph_fingerprint.is_none() { apply_sidecar_to_doc(entry, &sc); }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    // Now consume fetched sidecar texts after we're done reading from ui.graphs
    for id in consume_sidecar_for { ui.sidecar_texts.remove(&id); }
}


