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
    /// Global inline rename state (only one rename across app at a time)
    pub rename_inline: Option<RenameInline>,
    /// One-shot commit captured during draw; consumed by shell
    pub pending_rename_commit: Option<RenameInline>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextTarget { Node(EntityId), Edge(EntityId), Canvas }

#[derive(Debug, Clone)]
pub struct ContextMenuState { pub doc: ServerEntity, pub target: ContextTarget, pub pos: egui::Pos2, pub just_opened: bool }

#[derive(Debug, Clone)]
pub struct RenameInline { pub doc: ServerEntity, pub target: EntityId, pub text: String }
