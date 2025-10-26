use crate::types::ServerEntity;
use super::model::store::EditorStore;
use super::model::types::{ConnectionState, IndexFilter};

#[derive(Debug, Clone)]
pub struct EndpointConfig { pub endpoint: String }

pub fn connect(store: &mut EditorStore, endpoint: EndpointConfig) {
    store.connection = ConnectionState::Connecting;
    // RPC connect will be wired later; for now, we only set state.
    store.connection = ConnectionState::Connected { session_id: 0, endpoint: endpoint.endpoint };
}

pub fn disconnect(store: &mut EditorStore) {
    store.connection = ConnectionState::Disconnected;
    store.clear_session();
}

pub fn reconnect(store: &mut EditorStore) {
    // For now, treat as full disconnect then connect with last endpoint if present
    let endpoint = match &store.connection {
        ConnectionState::Connected { endpoint, .. } => Some(endpoint.clone()),
        _ => None,
    };
    store.clear_session();
    store.connection = ConnectionState::Disconnected;
    if let Some(ep) = endpoint { store.connection = ConnectionState::Connecting; store.connection = ConnectionState::Connected { session_id: 0, endpoint: ep }; }
}

pub fn refresh_index(store: &mut EditorStore, _filter: IndexFilter) {
    store.index.is_loading = true;
    // RPC list will populate here; leave empty for skeleton
    store.index.is_loading = false;
}

pub fn open_machine(_store: &mut EditorStore, _entity: ServerEntity) {
    // Will perform fresh RPC and create an OpenDocument; skeleton only
}


