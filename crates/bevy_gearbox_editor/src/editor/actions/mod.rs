use bevy::prelude::*;
use crate::types::ServerEntity;
use super::model::store::EditorStore;
use super::model::types::{ConnectionState, IndexFilter};
use bevy_gearbox_protocol::client::{ClientCommand, NetCommand};
use super::model::store::OpenDocument;
use super::model::types::{DocMode, DocId, TabId};
use crate::editor::workspace::Workspace;
use crate::persistence::extract_sidecar_from_doc;
use rfd::FileDialog;

#[derive(Debug, Clone)]
pub struct EndpointConfig { pub endpoint: String }

pub fn connect(store: &mut EditorStore, endpoint: EndpointConfig) {
    store.connection = ConnectionState::Connecting;
    store.last_endpoint = Some(endpoint.endpoint.clone());
}

pub fn disconnect(store: &mut EditorStore) {
    store.connection = ConnectionState::Disconnected;
    store.clear_session();
}

pub fn reconnect(store: &mut EditorStore) {
    // For now, treat as full disconnect then connect with last endpoint if present
    let endpoint = store.last_endpoint.clone();
    store.clear_session();
    store.connection = ConnectionState::Disconnected;
    if let Some(ep) = endpoint {
        store.connection = ConnectionState::Connecting;
        store.session_id = store.session_id.wrapping_add(1);
        store.connection = ConnectionState::Connected { session_id: store.session_id, endpoint: ep };
    }
}

pub fn refresh_index(store: &mut EditorStore, _filter: IndexFilter) {
    store.index.is_loading = true;
    // RPC list will populate here; leave empty for skeleton
    store.index.is_loading = false;
}

#[allow(dead_code)]
pub fn open_machine(_store: &mut EditorStore, _entity: ServerEntity) {
    // Will perform fresh RPC and create an OpenDocument; skeleton only
}

// Events (request side) for Bevy 0.17 observers (trigger with commands.trigger(...))
#[derive(Debug, Clone, Event)]
pub struct ConnectRequested { pub endpoint: String }

#[derive(Debug, Clone, Event)]
pub struct DisconnectRequested;

#[derive(Debug, Clone, Event)]
pub struct ReconnectRequested;

#[derive(Debug, Clone, Event)]
pub struct RefreshIndexRequested { pub query: String }

#[derive(Debug, Clone, Event)]
pub struct OpenRequested { pub entity: ServerEntity }

// Observers: mutate store
pub fn on_connect_requested(evt: On<ConnectRequested>, mut store: ResMut<EditorStore>, mut proto_cmd: MessageWriter<ClientCommand>, _proto_net: MessageWriter<NetCommand>) {
    // Optimistically bump session and set last endpoint
    connect(&mut store, EndpointConfig { endpoint: evt.endpoint.clone() });
    // Route URL and connection intent to the protocol client
    proto_cmd.write(ClientCommand::SetUrl { url: evt.endpoint.clone() });
    // Kick off initial refresh only; discovery will start after a successful refresh
    proto_cmd.write(ClientCommand::RefreshMachines);
}

pub fn on_disconnect_requested(_evt: On<DisconnectRequested>, mut store: ResMut<EditorStore>, mut proto_net: MessageWriter<NetCommand>) {
    // Stop discovery stream when disconnecting
    proto_net.write(NetCommand::StopDiscovery);
    // Proactively unsubscribe from all open docs before disconnecting
    for (id, _) in store.open_docs.iter() {
        proto_net.write(NetCommand::StopMachine { id: id.0 });
    }
    disconnect(&mut store);
}

pub fn on_reconnect_requested(_evt: On<ReconnectRequested>, mut store: ResMut<EditorStore>, mut proto_cmd: MessageWriter<ClientCommand>, mut proto_net: MessageWriter<NetCommand>) {
    reconnect(&mut store);
    if let Some(ep) = store.last_endpoint.clone() {
        proto_cmd.write(ClientCommand::SetUrl { url: ep });
    }
    proto_net.write(NetCommand::StartDiscovery);
    proto_cmd.write(ClientCommand::RefreshMachines);
}

pub fn on_refresh_index_requested(evt: On<RefreshIndexRequested>, mut store: ResMut<EditorStore>, mut proto_cmd: MessageWriter<ClientCommand>) {
    refresh_index(&mut store, IndexFilter { query: evt.query.clone() });
    proto_cmd.write(ClientCommand::RefreshMachines);
}

pub fn on_open_requested(
    evt: On<OpenRequested>,
    mut store: ResMut<EditorStore>,
    mut workspace: ResMut<Workspace>,
    mut commands: Commands,
    mut proto_net: MessageWriter<NetCommand>,
    mut proto_cmd: MessageWriter<ClientCommand>,
) {
    // Ensure an OpenDocument exists (UI metadata only)
    store.open_docs.entry(evt.entity).or_insert_with(|| OpenDocument {
        doc_id: DocId(evt.entity),
        tab_id: TabId(evt.entity),
        mode: DocMode::Live,
        is_subscribed: false,
        is_dirty: false,
        error: None,
    });
    // Ensure a doc entry exists immediately for drawing feedback
    let _ = workspace.docs.entry(evt.entity).or_default();
    // Promote to active first, then enqueue fetch
    let prev = store.active_doc;
    store.active_doc = Some(evt.entity);
    // Start per-machine +watch stream
    proto_net.write(NetCommand::StartMachine { id: evt.entity.0 });
    // Decoupled unsubscribe and single-doc retention after fetch is enqueued
    if let Some(p) = prev {
        if p != evt.entity {
            commands.trigger(super::actions::UnsubscribeRequested { entity: p });
            workspace.selection = None;
            workspace.docs.retain(|k, _| *k == evt.entity);
        }
    }
    // Request a fresh graph snapshot via protocol (handled asynchronously)
    proto_cmd.write(ClientCommand::FetchGraph { id: evt.entity.0 });
}

#[derive(Debug, Clone, Event)]
pub struct UnsubscribeRequested { pub entity: ServerEntity }

pub fn on_unsubscribe_requested(evt: On<UnsubscribeRequested>, mut proto_net: MessageWriter<NetCommand>) {
    // Decoupled unsubscribe: stop server-side feeds for this machine. Do not couple to new selection.
    proto_net.write(NetCommand::StopMachine { id: evt.entity.0 });
}

#[derive(Debug, Clone, Event)]
pub struct SaveAsRequested { pub entity: ServerEntity }

pub fn on_save_as_requested(
    save_as_requested: On<SaveAsRequested>,
    workspace: Res<Workspace>,
    client: Res<bevy_gearbox_protocol::client::Client>,
    rt: Res<bevy_gearbox_protocol::client::TokioRuntime>,
) {
    // Open native Save dialog for .sm.ron, start in assets/ and suggest a name
    let picked = FileDialog::new()
        .add_filter("State Machine Sidecar", &["sm.ron"])        
        .set_title("Save State Machine (.sm.ron)")
        .set_directory("assets")
        .set_file_name("state_machine.sm.ron")
        .save_file();

    if let Some(path) = picked {
        // Derive logical asset base name (without .sm.ron extension and without directories)
        let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let base = if fname.ends_with(".sm.ron") { &fname[..fname.len()-7] } else { fname };
        let id_text = base.to_string();
        let scn_path = format!("{}.scn.ron", id_text);
        let sm_path = format!("{}.sm.ron", id_text);

        // Extract and serialize current sidecar snapshot once (to upload only on success)
        let sidecar_text: Option<String> = workspace
            .docs
            .get(&save_as_requested.entity)
            .map(|doc| {
                let sc = extract_sidecar_from_doc(doc);
                let pretty = ron::ser::PrettyConfig::new();
                ron::ser::to_string_pretty(&sc, pretty).ok()
            })
            .flatten();

        let entity_bits = save_as_requested.entity.0;
        let client_cloned = client.clone();
        rt.0.spawn(async move {
            // 1) Insert StateMachineId(name) on the root
            if client_cloned.set_state_machine_id(entity_bits, &id_text).await.is_err() { return; }
            // 2) Save scene under assets/<name>.scn.ron
            if client_cloned.save_graph(entity_bits, &scn_path).await.is_err() { return; }
            // 3) Only on success, save editor sidecar next to it as assets/<name>.sm.ron
            if let Some(txt) = sidecar_text {
                let _ = client_cloned.save_sidecar(&sm_path, &txt).await;
            }
        });
    }
}



