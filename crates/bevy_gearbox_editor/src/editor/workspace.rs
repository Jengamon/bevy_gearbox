use bevy::prelude::*;
use std::collections::HashMap;
use crate::{model::EntityId, types::ServerEntity};
use super::view_model::GraphDoc;
use bevy_egui::egui;

#[derive(Debug, Default, Resource)]
pub struct Workspace {
    pub docs: HashMap<ServerEntity, GraphDoc>,
    pub selection: Option<EntityId>,
    pub menu: Option<ContextMenuState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextTarget { Node(EntityId), Canvas }

#[derive(Debug, Clone)]
pub struct ContextMenuState { pub doc: ServerEntity, pub target: ContextTarget, pub pos: egui::Pos2 }


