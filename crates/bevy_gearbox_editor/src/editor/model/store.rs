use bevy::prelude::*;
use crate::types::ServerEntity;
use super::types::{ConnectionState, StateMachineIndex, DocMode, DocId, TabId};

#[derive(Debug)]
pub struct OpenDocument {
    pub doc_id: DocId,
    pub tab_id: TabId,
    pub mode: DocMode,
    pub is_subscribed: bool,
    pub is_dirty: bool,
    pub error: Option<String>,
}

#[derive(Debug, Default, Resource)]
pub struct EditorStore {
    pub connection: ConnectionState,
    /// Last endpoint used for connection (if any)
    pub last_endpoint: Option<String>,
    /// Monotonically increasing session identifier; increment on each successful connect/reconnect
    pub session_id: u64,
    pub index: StateMachineIndex,
    pub open_docs: std::collections::HashMap<ServerEntity, OpenDocument>,
    /// Currently active/open document shown in the canvas
    pub active_doc: Option<ServerEntity>,
}

impl EditorStore {
    pub fn clear_session(&mut self) {
        self.index = StateMachineIndex::default();
        self.open_docs.clear();
        self.active_doc = None;
    }
}


