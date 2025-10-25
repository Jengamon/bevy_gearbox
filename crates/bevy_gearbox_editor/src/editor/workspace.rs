use bevy::prelude::*;
use std::collections::HashMap;
use crate::model::{NodeId, EdgeId};
use crate::types::ServerEntity;
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
    pub node: Option<NodeId>,
    pub edge: Option<EdgeId>,
}

#[derive(Debug, Clone)]
pub enum ContextTarget { Node(NodeId), Edge(EdgeId), Canvas }

#[derive(Debug, Clone)]
pub struct ContextMenuState { pub target: ContextTarget, pub pos: egui::Pos2 }


