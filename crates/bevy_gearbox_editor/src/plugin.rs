use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::collections::HashMap;

use crate::net::{NetPlugin, NetCommand, NetEvent, StampedEvent, ActiveSession};
use crate::types::ServerEntity;
use crate::model::StateMachineGraph;
use crate::editor::workspace::Workspace;
use crate::editor::model::store::EditorStore;
use crate::editor::actions::{
    on_connect_requested, on_disconnect_requested, on_reconnect_requested, on_refresh_index_requested, on_open_requested,
};
use crate::editor::adapter::project_graph_into_doc;
use crate::persistence::{apply_sidecar_to_doc, compute_graph_fingerprint, load_sidecar, parse_sidecar_text};
use crate::editor::model::types::ConnectionState as EditorConnectionState;
use crate::editor::model::types::{IndexItem};
use crate::component as c;
use crate::model::EntityId;
use serde_json::Value as JsonValue;

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
                pending_active: HashMap::new(),
                pending_machine_events: HashMap::new(),
            })
            .init_resource::<Workspace>()
            .insert_resource(EditorStore::default())
            .add_systems(Startup, setup_camera)
            .add_systems(Update, (poll_network, sync_snapshots_to_workspace))
            // Register editor observers (events are triggered via commands.trigger(...))
            .add_observer(on_connect_requested)
            .add_observer(on_disconnect_requested)
            .add_observer(on_reconnect_requested)
            .add_observer(on_refresh_index_requested)
            .add_observer(on_open_requested)
            .add_observer(crate::editor::actions::on_save_as_requested);

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
    /// One-shot active state snapshots awaiting application to docs
    pending_active: HashMap<ServerEntity, (Vec<u64>, Vec<u64>)>,
    /// Accumulated machine +watch events awaiting application to docs
    pending_machine_events: HashMap<ServerEntity, Vec<JsonValue>>,
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn poll_network(
    mut ui: ResMut<UiState>,
    mut events: MessageReader<StampedEvent>,
    mut cmd_writer: MessageWriter<NetCommand>,
    mut active_session: ResMut<ActiveSession>,
    mut store: ResMut<EditorStore>,
) {
    let mut processed = 0usize;
    const MAX_PER_FRAME: usize = 64;
    // Keep network session aligned with editor session (placeholder; set on connect)
    if let EditorConnectionState::Connected { session_id, .. } = store.connection.clone() {
        if active_session.0 != session_id { active_session.0 = session_id; }
    }
    let cur_session = active_session.0;
    for stamped in events.read() {
        if processed >= MAX_PER_FRAME { break; }
        // Drop stale results from previous sessions
        if stamped.session != cur_session { continue; }
        match &*stamped.event {
            NetEvent::Connected => {
                // First: reflect connected state and align session ids so stamped events aren't dropped
                if let Some(ep) = store.last_endpoint.clone() {
                    store.session_id = store.session_id.wrapping_add(1);
                    let sid = store.session_id;
                    store.connection = crate::editor::model::types::ConnectionState::Connected { session_id: sid, endpoint: ep };
                    // Ensure network stamps use the new session id before spawning tasks
                    active_session.0 = sid;
                }
                // Then: kick off discovery snapshot + watch
                cmd_writer.write(NetCommand::Refresh);
                cmd_writer.write(NetCommand::StartDiscoveryWatch);
                processed += 1;
            }
            NetEvent::RefreshResult(Ok(machines)) => {
                // Update UI cache and editor index; no autoload
                ui.machines = machines.iter().map(|m| (m.id, m.name.clone())).collect();
                ui.connecting = false;
                ui.error = None;
                store.index.is_loading = false;
                store.index.error = None;
                store.index.items = machines.iter().map(|m| IndexItem { name: m.name.clone(), entity: m.id }).collect();
                processed += 1;
            }
            NetEvent::DiscoveryEvents(Ok(batch)) => {
                // Merge batch into index (simple replace-by-id for now)
                for m in batch {
                    if let Some(name) = m.name.clone() {
                        if let Some(ix) = ui.machines.iter_mut().position(|(id, _)| id.0 == m.id.0) {
                            ui.machines[ix] = (m.id, Some(name));
                        } else {
                            ui.machines.push((m.id, Some(name)));
                        }
                    } else {
                        ui.machines.retain(|(id, _)| id.0 != m.id.0);
                    }
                }
                // Rebuild store index list
                ui.machines.sort_by_key(|(id, _)| id.0);
                store.index.items = ui.machines.iter().map(|(id, name)| IndexItem { name: name.clone(), entity: *id }).collect();
                // Re-arm watcher for next batch
                cmd_writer.write(NetCommand::StartDiscoveryWatch);
                processed += 1;
            }
            NetEvent::RefreshResult(Err(e)) => {
                ui.connecting = false;
                ui.error = Some(e.to_string());
                store.index.is_loading = false;
                store.index.error = Some(e.to_string());
                processed += 1;
            }
            NetEvent::GraphResult { id, result } => {
                match result {
                    Ok(graph) => {
                        // Stash/refresh snapshot
                        ui.graphs.insert(*id, graph.clone());
                        // Seed active states and start machine +watch
                        cmd_writer.write(NetCommand::FetchActive { id: *id });
                        cmd_writer.write(NetCommand::StartMachineWatch { id: *id });
                        // If the root has a StateMachineId, request its sidecar via RPC (derive path: <id>.sm.ron)
                        if let Some(id_text) = graph.nodes.get(&graph.root)
                            .and_then(|n| n.components.get(c::STATE_MACHINE_ID))
                            .and_then(|e| e.value_json.as_str())
                        {
                            let path = format!("{}.sm.ron", id_text);
                            cmd_writer.write(NetCommand::FetchSidecarByPath { path, doc: *id });
                        }
                    }
                    Err(e) => {
                    }
                }
                processed += 1;
            }
            NetEvent::ActiveResult { id, result } => {
                if let Ok((active, leaves)) = result {
                    ui.pending_active.insert(*id, (active.clone(), leaves.clone()));
                }
                processed += 1;
            }
            NetEvent::MachineDeltas { id, result } => {
                if let Ok(events) = result {
                    // Stash for application in workspace sync
                    ui.pending_machine_events.entry(*id).or_default().extend(events.iter().cloned());
                    // For now, immediately re-arm the watch to keep streaming
                    cmd_writer.write(NetCommand::StartMachineWatch { id: *id });
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
    mut store: ResMut<EditorStore>,
    mut commands: Commands,
    mut workspace: ResMut<Workspace>,
) {
    if let Ok(ctx) = egui_ctx.ctx_mut() {
        egui::CentralPanel::default().show(ctx, |ui_egui| {
            crate::editor::shell::layout::draw(ui_egui, &mut store, &mut commands, &mut workspace);
        });
    }
}

fn sync_snapshots_to_workspace(
    mut workspace: ResMut<Workspace>,
    mut ui: ResMut<UiState>,
) {
    let mut consume_sidecar_for: Vec<ServerEntity> = Vec::new();
    // Apply pending active snapshots and machine deltas per-doc before projecting any new graph snapshots
    // 1) Active snapshots
    let pending_active = std::mem::take(&mut ui.pending_active);
    for (id, (active, _leaves)) in pending_active.into_iter() {
        let doc = workspace.docs.entry(id).or_default();
        // Map u64s to EntityId::Server (canonicalize)
        let set: std::collections::HashSet<EntityId> = active
            .into_iter()
            .map(|u| crate::util::canonicalize_entity_u64(u))
            .map(|u| EntityId::Server(ServerEntity(u)))
            .collect();
        let (_new, _deactivated) = doc.set_active_nodes(&set);
    }
    // 2) Machine event batches (canonicalize ids before applying)
    let pending_events = std::mem::take(&mut ui.pending_machine_events);
    for (id, events) in pending_events.into_iter() {
        let doc = workspace.docs.entry(id).or_default();
        for ev in events.into_iter() {
            let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "active_changed" => {
                    let active: Vec<u64> = ev
                        .get("active")
                        .and_then(|a| a.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_u64()).map(|u| crate::util::canonicalize_entity_u64(u)).collect())
                        .unwrap_or_default();
                    let set: std::collections::HashSet<EntityId> = active.into_iter().map(|u| EntityId::Server(ServerEntity(u))).collect();
                    let (new_nodes, deactivated) = doc.set_active_nodes(&set);
                    for nid in new_nodes { doc.node_flash.insert(nid, 1.0); }
                    for nid in deactivated { doc.node_fade.insert(nid, 1.0); }
                }
                "transition_edge" => {
                    if let Some(edge) = ev.get("edge").and_then(|v| v.as_u64()) {
                        let edge = crate::util::canonicalize_entity_u64(edge);
                        let eid = EntityId::Server(ServerEntity(edge));
                        doc.flash_edge(eid);
                    }
                }
                _ => {}
            }
        }
    }
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
                // As a final fallback when no sidecar is found anywhere, ensure a derived default layout
                // is applied so the editor shows states/edges at reasonable default positions.
                if entry.graph.is_some() && entry.views.is_empty() {
                    project_graph_into_doc(entry, graph.clone());
                }
            }
        }
    }
    // Now consume fetched sidecar texts after we're done reading from ui.graphs
    for id in consume_sidecar_for { ui.sidecar_texts.remove(&id); }
}


