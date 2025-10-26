use crate::model::EntityId;
use super::types::TabId;

#[derive(Debug, Default, Clone)]
pub struct SelectionState {
    pub active_tab: Option<TabId>,
    pub per_tab_selected: std::collections::HashMap<TabId, Option<EntityId>>,
}

