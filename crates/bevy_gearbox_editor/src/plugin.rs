use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::collections::HashMap;

use bevy_gearbox_protocol::client::{GearboxProtocolClientPlugin, ProtocolNetMessage, ProtocolClientMessage, ProtocolNetCommand, ProtocolClientCommand};
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
use bevy_gearbox_protocol::components as c;
use crate::model::EntityId;
use serde_json::Value as JsonValue;

pub(crate) struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(GearboxProtocolClientPlugin)
            .insert_resource(UiState {
                url_edit: String::new(),
                connecting: false,
                error: None,
                machines: vec![],
                graphs: HashMap::new(),
                sidecar_texts: HashMap::new(),
                pending_active: HashMap::new(),
                pending_machine_events: HashMap::new(),
                last_active_seq: HashMap::new(),
                last_transition_seq: HashMap::new(),
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
            .add_observer(crate::editor::actions::on_unsubscribe_requested)
            .add_observer(crate::editor::actions::on_save_as_requested)
            ;

        use bevy_egui::EguiPrimaryContextPass;
        app.add_systems(EguiPrimaryContextPass, ui_system);
    }
}

#[derive(Resource, Clone)]
pub(crate) struct UiState {
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
    /// Per-machine cursors for stateless +watch
    last_active_seq: HashMap<ServerEntity, u64>,
    last_transition_seq: HashMap<ServerEntity, u64>,
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn poll_network(
    mut ui: ResMut<UiState>,
    mut client_msgs: MessageReader<ProtocolClientMessage>,
    mut net_msgs: MessageReader<ProtocolNetMessage>,
    mut net_cmd: MessageWriter<ProtocolNetCommand>,
    mut client_cmd: MessageWriter<ProtocolClientCommand>,
    mut store: ResMut<EditorStore>,
    mut workspace: ResMut<Workspace>,
) {
    let mut processed = 0usize;
    const MAX_PER_FRAME: usize = 64;
    // Handle client responses (e.g., RefreshMachines)
    for msg in client_msgs.read() {
        if processed >= MAX_PER_FRAME { break; }
        match msg {
            ProtocolClientMessage::RefreshResult(Ok(list)) => {
                // Update UI cache and editor index
                ui.machines = list.iter().map(|m| (ServerEntity(m.id), m.name.clone())).collect();
                ui.connecting = false;
                ui.error = None;
                store.index.is_loading = false;
                store.index.error = None;
                store.index.items = list.iter().map(|m| IndexItem { name: m.name.clone(), entity: ServerEntity(m.id) }).collect();
                // Mark connected for UI button logic
                let ep = store.last_endpoint.clone().unwrap_or_else(|| "http://127.0.0.1:15703".to_string());
                store.connection = EditorConnectionState::Connected { session_id: store.session_id, endpoint: ep };
                // Now that the refresh succeeded, start discovery watch
                net_cmd.write(ProtocolNetCommand::StartDiscovery);
                processed += 1;
            }
            ProtocolClientMessage::RefreshResult(Err(e)) => {
                ui.connecting = false;
                ui.error = Some(e.clone());
                store.index.is_loading = false;
                store.index.error = Some(e.clone());
                store.connection = EditorConnectionState::Disconnected;
                processed += 1;
            }
            ProtocolClientMessage::GraphResult { id, graph } => {
                if let Some(sm_graph) = convert_wire_graph_to_state_machine_graph(graph.clone()) {
                    let fp = compute_graph_fingerprint(&sm_graph);
                    let doc_id = ServerEntity(*id);
                    ui.graphs.insert(doc_id, sm_graph);
                    // Prefer server-resolved sidecar lookup by machine
                    client_cmd.write(ProtocolClientCommand::SidecarForMachine { id: *id });
                }
            }
            ProtocolClientMessage::SidecarFound { id, text } => {
                let doc_id = ServerEntity(*id);
                ui.sidecar_texts.insert(doc_id, text.clone());
                processed += 1;
            }
            ProtocolClientMessage::SidecarMissing { .. } => {
                // No-op; fallback to local disk/default layout in sync pass
                processed += 1;
            }
            ProtocolClientMessage::EventEdgeVariants { variants } => {
                workspace.available_event_edges = variants.clone();
                processed += 1;
            }
        }
    }
    // Handle net watch messages (discovery, machine deltas)
    for msg in net_msgs.read() {
        if processed >= MAX_PER_FRAME { break; }
        match msg.clone() {
            ProtocolNetMessage::Discovery(batch) => {
                for m in batch.into_iter() {
                    if let Some(name) = m.name.clone() {
                        if let Some(ix) = ui.machines.iter_mut().position(|(id, _)| id.0 == m.id) {
                            ui.machines[ix] = (ServerEntity(m.id), Some(name));
                        } else {
                            ui.machines.push((ServerEntity(m.id), Some(name)));
                        }
                    } else {
                        ui.machines.retain(|(id, _)| id.0 != m.id);
                    }
                }
                ui.machines.sort_by_key(|(id, _)| id.0);
                store.index.items = ui.machines.iter().map(|(id, name)| IndexItem { name: name.clone(), entity: *id }).collect();
                processed += 1;
            }
            ProtocolNetMessage::Machine { id, events } => {
                let doc_id = ServerEntity(id);
                // Update last seqs and stash events
                let mut max_a = ui.last_active_seq.get(&doc_id).copied().unwrap_or(0);
                let mut max_t = ui.last_transition_seq.get(&doc_id).copied().unwrap_or(0);
                for ev in events.iter() {
                    let seq = ev.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
                    match ev.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
                        "active_changed" => { if seq > max_a { max_a = seq; } }
                        "transition_edge" => { if seq > max_t { max_t = seq; } }
                        _ => {}
                    }
                }
                ui.last_active_seq.insert(doc_id, max_a);
                ui.last_transition_seq.insert(doc_id, max_t);
                ui.pending_machine_events.entry(doc_id).or_default().extend(events.into_iter());
                processed += 1;
            }
            ProtocolNetMessage::Components { id, components, removed } => {
                // Apply Name changes to any open doc containing this entity
                let target = crate::model::EntityId::Server(ServerEntity(id));
                let name_key = bevy_gearbox_protocol::components::NAME_REFLECT;
                let name_opt = components.get(name_key).and_then(|v| v.as_str()).map(|s| s.to_string());
                for (_doc_id, doc) in workspace.docs.iter_mut() {
                    if let Some(v) = doc.views.get_mut(&target) {
                        if let Some(ref name) = name_opt { v.label = name.clone(); }
                    }
                    if let Some(g) = doc.graph.as_mut() {
                        if let Some(n) = g.nodes.get_mut(&target) {
                            if let Some(ref name) = name_opt { n.display_name = Some(name.clone()); }
                        }
                        if let Some(e) = g.edges.get_mut(&target) {
                            if let Some(ref name) = name_opt { e.display_label = Some(name.clone()); }
                        }
                    }
                }
                processed += 1;
            }
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
                "name_changed" => {
                    if let (Some(ent_u), Some(name_s)) = (
                        ev.get("entity").and_then(|v| v.as_u64()),
                        ev.get("name").and_then(|v| v.as_str()),
                    ) {
                        let ent_u = crate::util::canonicalize_entity_u64(ent_u);
                        let eid = EntityId::Server(ServerEntity(ent_u));
                        let name = name_s.to_string();
                        if let Some(v) = doc.views.get_mut(&eid) { v.label = name.clone(); }
                        if let Some(g) = doc.graph.as_mut() {
                            if let Some(n) = g.nodes.get_mut(&eid) { n.display_name = Some(name.clone()); }
                            if let Some(e) = g.edges.get_mut(&eid) { e.display_label = Some(name.clone()); }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // Drain snapshot inbox: apply once, then clear from UiState
    let mut to_remove: Vec<ServerEntity> = Vec::new();
    for (id, graph) in ui.graphs.iter() {
        // Capture metrics before taking a mutable borrow of workspace.docs entry
        let was_empty = workspace.docs.get(id).and_then(|d| d.graph.as_ref()).is_none();
        let entry = workspace.docs.entry(*id).or_default();
        project_graph_into_doc(entry, graph.clone());
        // After mutation, avoid borrowing workspace again; use the entry we have
        to_remove.push(*id);
        // Try applying sidecar when: (a) first load, or (b) new sidecar text arrived
        let fp = compute_graph_fingerprint(&graph);
        let mut applied = false;
        if let Some(text) = ui.sidecar_texts.get(id) {
            match parse_sidecar_text(text) {
                Ok(sc) => {
                    apply_sidecar_to_doc(entry, &sc);
                    applied = true;
                }
                Err(e) => (),
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
                // As a final fallback when no sidecar is found anywhere, ensure a derived default layout
                // is applied so the editor shows states/edges at reasonable default positions.
                if entry.graph.is_some() && entry.views.is_empty() {
                    project_graph_into_doc(entry, graph.clone());
                }
            }
        }
    }
    // Remove consumed snapshots
    for id in to_remove { ui.graphs.remove(&id); }
    // Apply any sidecars that arrived independently of new snapshots (decoupled from inbox)
    // Only apply if the doc already has a graph to target
    let extra_sidecars: Vec<(ServerEntity, String)> = ui.sidecar_texts.iter().map(|(k, v)| (*k, v.clone())).collect();
    for (id, text) in extra_sidecars.iter() {
        if let Some(doc) = workspace.docs.get_mut(id) {
            if let Some(_graph) = &doc.graph {
                match parse_sidecar_text(text) {
                    Ok(sc) => {
                        apply_sidecar_to_doc(doc, &sc);
                        consume_sidecar_for.push(*id);
                    }
                    Err(e) => (),
                }
            }
        }
    }
    // Now consume fetched sidecar texts
    for id in consume_sidecar_for { ui.sidecar_texts.remove(&id); }
}

fn convert_wire_graph_to_state_machine_graph(graph: serde_json::Value) -> Option<StateMachineGraph> {
    use crate::model as m;
    use crate::types::ServerEntity;
    let root_s = graph.get("root").and_then(|v| v.as_str())?;
    let root_u = root_s.parse::<u64>().ok()?;
    let mut out = m::StateMachineGraph::new(m::StateNode::new(m::EntityId::Server(ServerEntity(root_u))));
    // Nodes
    if let Some(nodes) = graph.get("nodes").and_then(|v| v.as_array()) {
        for n in nodes.iter() {
            let id_u = n.get("id").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok())?;
            let id = m::EntityId::Server(ServerEntity(id_u));
            let mut node = out.nodes.remove(&id).unwrap_or_else(|| m::StateNode::new(id));
            if let Some(parent_s) = n.get("parent").and_then(|v| v.as_str()) { if let Ok(pu) = parent_s.parse::<u64>() { node.parent = Some(m::EntityId::Server(ServerEntity(pu))); } }
            if let Some(comps) = n.get("components").and_then(|v| v.as_object()) {
                for (k, v) in comps.iter() { node.components.insert(m::ComponentEntry::new(k.clone(), v.clone())); }
                if let Some(name_v) = comps.get("Name") { if let Some(s) = name_v.as_str() { node.display_name = Some(s.to_string()); } }
                if let Some(children_v) = comps.get("bevy_gearbox::Substates").and_then(|v| v.as_array()) {
                    node.children = children_v.iter().filter_map(|vv| vv.as_str()).filter_map(|s| s.parse::<u64>().ok()).map(|u| m::EntityId::Server(ServerEntity(u))).collect();
                }
            }
            out.nodes.insert(id, node);
        }
    }
    // Edges
    if let Some(edges) = graph.get("edges").and_then(|v| v.as_array()) {
        for e in edges.iter() {
            let id_u = e.get("id").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok())?;
            let src_u = e.get("source").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok())?;
            let tgt_u = e.get("target").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            if tgt_u == 0 { continue; }
            let id = m::EntityId::Server(ServerEntity(id_u));
            let src = m::EntityId::Server(ServerEntity(src_u));
            let tgt = m::EntityId::Server(ServerEntity(tgt_u));
            let mut edge = m::Edge::new(id, src, tgt);
            if let Some(comps) = e.get("components").and_then(|v| v.as_object()) {
                for (k, v) in comps.iter() { edge.components.insert(m::ComponentEntry::new(k.clone(), v.clone())); }
                // Derive and store a stable display label so sidecar edge keys can match
                edge.display_label = Some(crate::model::choose_edge_label_bag(&edge.components));
            }
            out.adjacency_out.entry(src).or_default().push(id);
            out.adjacency_in.entry(tgt).or_default().push(id);
            out.edges.insert(id, edge);
        }
    }
    Some(out)
}


