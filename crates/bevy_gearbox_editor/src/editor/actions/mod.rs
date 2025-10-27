use bevy::prelude::*;
use crate::types::ServerEntity;
use super::model::store::EditorStore;
use super::model::types::{ConnectionState, IndexFilter};
use crate::net::{NetCommand, ActiveSession};
use super::model::store::OpenDocument;
use super::model::types::{DocMode, DocId, TabId};
use crate::editor::view_model::GraphDoc;
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

// Observers: mutate store; RPC wiring will be added later
pub fn on_connect_requested(evt: On<ConnectRequested>, mut store: ResMut<EditorStore>, mut net: MessageWriter<NetCommand>, mut active_session: ResMut<ActiveSession>) {
    // Optimistically bump session and set last endpoint
    connect(&mut store, EndpointConfig { endpoint: evt.endpoint.clone() });
    // Ensure network stamps use the new session id before any tasks are spawned
    active_session.0 = store.session_id;
    net.write(NetCommand::SetUrl { url: evt.endpoint.clone() });
    net.write(NetCommand::Connect);
}

pub fn on_disconnect_requested(_evt: On<DisconnectRequested>, mut store: ResMut<EditorStore>, mut net: MessageWriter<NetCommand>) {
    disconnect(&mut store);
    net.write(NetCommand::Disconnect);
}

pub fn on_reconnect_requested(_evt: On<ReconnectRequested>, mut store: ResMut<EditorStore>, mut net: MessageWriter<NetCommand>) {
    reconnect(&mut store);
    if let Some(ep) = store.last_endpoint.clone() {
        net.write(NetCommand::SetUrl { url: ep });
    }
    net.write(NetCommand::Connect);
}

pub fn on_refresh_index_requested(evt: On<RefreshIndexRequested>, mut store: ResMut<EditorStore>, mut net: MessageWriter<NetCommand>) {
    refresh_index(&mut store, IndexFilter { query: evt.query.clone() });
    net.write(NetCommand::Refresh);
}

pub fn on_open_requested(evt: On<OpenRequested>, mut store: ResMut<EditorStore>, mut net: MessageWriter<NetCommand>) {
    // Ensure an OpenDocument exists so the UI can render immediately (snapshot will fill via workspace sync)
    store.open_docs.entry(evt.entity).or_insert_with(|| OpenDocument {
        doc_id: DocId(evt.entity),
        tab_id: TabId(evt.entity),
        mode: DocMode::Live,
        is_subscribed: false,
        is_dirty: false,
        error: None,
        graph: GraphDoc::default(),
    });
    store.active_doc = Some(evt.entity);
    net.write(NetCommand::FetchGraph { id: evt.entity });
}

#[derive(Debug, Clone, Event)]
pub struct SaveAsRequested { pub entity: ServerEntity }

pub fn on_save_as_requested(save_as_requested: On<SaveAsRequested>, workspace: Res<Workspace>, mut net: MessageWriter<NetCommand>) {
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
        let asset_base = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "state_machine".to_string());
        net.write(NetCommand::SaveAs { id: save_as_requested.entity, asset_base, sidecar_path: path });
    }
}


