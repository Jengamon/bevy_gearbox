use bevy::prelude::*;
use crate::types::ServerEntity;
use super::model::store::EditorStore;
use super::model::types::{ConnectionState, IndexFilter};
use bevy_gearbox_protocol::client::{ProtocolClientCommand, ProtocolNetCommand};
use super::model::store::OpenDocument;
use super::model::types::{DocMode, DocId, TabId};
use crate::editor::workspace::Workspace;
use crate::persistence::{extract_sidecar_from_doc, save_sidecar};
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
pub fn on_connect_requested(evt: On<ConnectRequested>, mut store: ResMut<EditorStore>, mut proto_cmd: MessageWriter<ProtocolClientCommand>, _proto_net: MessageWriter<ProtocolNetCommand>) {
    // Optimistically bump session and set last endpoint
    connect(&mut store, EndpointConfig { endpoint: evt.endpoint.clone() });
    // Route URL and connection intent to the protocol client
    proto_cmd.write(ProtocolClientCommand::SetUrl { url: evt.endpoint.clone() });
    // Kick off initial refresh only; discovery will start after a successful refresh
    proto_cmd.write(ProtocolClientCommand::RefreshMachines);
}

pub fn on_disconnect_requested(_evt: On<DisconnectRequested>, mut store: ResMut<EditorStore>, mut proto_net: MessageWriter<ProtocolNetCommand>) {
    // Stop discovery stream when disconnecting
    proto_net.write(ProtocolNetCommand::StopDiscovery);
    // Proactively unsubscribe from all open docs before disconnecting
    for (id, _) in store.open_docs.iter() {
        proto_net.write(ProtocolNetCommand::StopMachine { id: id.0 });
    }
    disconnect(&mut store);
}

pub fn on_reconnect_requested(_evt: On<ReconnectRequested>, mut store: ResMut<EditorStore>, mut proto_cmd: MessageWriter<ProtocolClientCommand>, mut proto_net: MessageWriter<ProtocolNetCommand>) {
    reconnect(&mut store);
    if let Some(ep) = store.last_endpoint.clone() {
        proto_cmd.write(ProtocolClientCommand::SetUrl { url: ep });
    }
    proto_net.write(ProtocolNetCommand::StartDiscovery);
    proto_cmd.write(ProtocolClientCommand::RefreshMachines);
}

pub fn on_refresh_index_requested(evt: On<RefreshIndexRequested>, mut store: ResMut<EditorStore>, mut proto_cmd: MessageWriter<ProtocolClientCommand>) {
    refresh_index(&mut store, IndexFilter { query: evt.query.clone() });
    proto_cmd.write(ProtocolClientCommand::RefreshMachines);
}

pub fn on_open_requested(
    evt: On<OpenRequested>,
    mut store: ResMut<EditorStore>,
    mut workspace: ResMut<Workspace>,
    mut commands: Commands,
    mut proto_net: MessageWriter<ProtocolNetCommand>,
    mut proto_cmd: MessageWriter<ProtocolClientCommand>,
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
    proto_net.write(ProtocolNetCommand::StartMachine { id: evt.entity.0 });
    // Decoupled unsubscribe and single-doc retention after fetch is enqueued
    if let Some(p) = prev {
        if p != evt.entity {
            commands.trigger(super::actions::UnsubscribeRequested { entity: p });
            workspace.selection = None;
            workspace.docs.retain(|k, _| *k == evt.entity);
        }
    }
    // Request a fresh graph snapshot via protocol (handled asynchronously)
    proto_cmd.write(ProtocolClientCommand::FetchGraph { id: evt.entity.0 });
}

#[derive(Debug, Clone, Event)]
pub struct UnsubscribeRequested { pub entity: ServerEntity }

pub fn on_unsubscribe_requested(evt: On<UnsubscribeRequested>, mut proto_net: MessageWriter<ProtocolNetCommand>) {
    // Decoupled unsubscribe: stop server-side feeds for this machine. Do not couple to new selection.
    proto_net.write(ProtocolNetCommand::StopMachine { id: evt.entity.0 });
}

#[derive(Debug, Clone, Event)]
pub struct SaveAsRequested { pub entity: ServerEntity }

pub fn on_save_as_requested(save_as_requested: On<SaveAsRequested>, workspace: Res<Workspace>) {
    // Open native Save dialog for .sm.ron
    let picked = FileDialog::new()
        .add_filter("State Machine Sidecar", &["sm.ron"])
        .set_title("Save State Machine Sidecar (.sm.ron)")
        .save_file();
    if let Some(path) = picked {
        // Persist current sidecar snapshot to chosen path if we have the doc
        if let Some(doc) = workspace.docs.get(&save_as_requested.entity) {
            let sc = extract_sidecar_from_doc(doc);
            let _ = save_sidecar(&path, &sc);
        }
        // Derive logical asset base name (without .sm.ron extension and without directories)
        let _ = path; // Protocol-side Save/SaveAs will be wired once available
    }
}


