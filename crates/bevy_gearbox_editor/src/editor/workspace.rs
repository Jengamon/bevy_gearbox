use bevy::prelude::*;
use std::collections::HashMap;
use crate::{model::EntityId, types::ServerEntity};
use super::view_model::GraphDoc;
use bevy_egui::egui;

#[derive(Debug, Default, Resource)]
pub struct Workspace {
    pub docs: HashMap<ServerEntity, GraphDoc>,
    pub selection: Selection,
    pub menu: Option<ContextMenuState>,
}

#[derive(Debug, Default)]
pub struct Selection {
    pub node: Option<EntityId>,
    pub edge: Option<EntityId>,
}

#[derive(Debug, Clone)]
pub enum ContextTarget { Node(EntityId), Edge(EntityId), Canvas }

#[derive(Debug, Clone)]
pub struct ContextMenuState { pub target: ContextTarget, pub pos: egui::Pos2 }


