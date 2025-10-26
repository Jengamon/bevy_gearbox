use bevy::prelude::*;
use crate::types::ServerEntity;
use super::model::store::EditorStore;
use super::model::types::{ConnectionState, IndexFilter};

#[derive(Debug, Clone)]
pub struct EndpointConfig { pub endpoint: String }

pub fn connect(store: &mut EditorStore, endpoint: EndpointConfig) {
    store.connection = ConnectionState::Connecting;
    store.last_endpoint = Some(endpoint.endpoint.clone());
    // RPC connect will be wired later; for now, simulate success and bump session id.
    store.session_id = store.session_id.wrapping_add(1);
    store.connection = ConnectionState::Connected { session_id: store.session_id, endpoint: endpoint.endpoint };
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
pub fn on_connect_requested(evt: On<ConnectRequested>, mut store: ResMut<EditorStore>) {
    connect(&mut store, EndpointConfig { endpoint: evt.endpoint.clone() });
}

pub fn on_disconnect_requested(_evt: On<DisconnectRequested>, mut store: ResMut<EditorStore>) {
    disconnect(&mut store);
}

pub fn on_reconnect_requested(_evt: On<ReconnectRequested>, mut store: ResMut<EditorStore>) {
    reconnect(&mut store);
}

pub fn on_refresh_index_requested(evt: On<RefreshIndexRequested>, mut store: ResMut<EditorStore>) {
    refresh_index(&mut store, IndexFilter { query: evt.query.clone() });
}

pub fn on_open_requested(_evt: On<OpenRequested>, _store: ResMut<EditorStore>) {
    // Placeholder observer; will trigger RPC open and doc creation later
}


