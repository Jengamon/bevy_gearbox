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
    /// Transition builder ephemeral state
    pub edge_build: Option<EdgeBuildState>,
    /// Edge kind chooser menu (opened when user picks a valid target during build)
    pub edge_menu: Option<EdgeMenuState>,
    /// Available EventEdge<T> variant display names; editor-only listing for the menu
    pub available_event_edges: Vec<String>,
    /// Persisted UI-only previews of edges committed via the chooser
    pub preview_edges: Vec<PreviewEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextTarget { Node(EntityId), Edge(EntityId), Canvas }

#[derive(Debug, Clone)]
pub struct ContextMenuState { pub doc: ServerEntity, pub target: ContextTarget, pub pos: egui::Pos2, pub just_opened: bool }

#[derive(Debug, Clone)]
pub struct RenameInline { pub doc: ServerEntity, pub target: EntityId, pub text: String }

#[derive(Debug, Clone)]
pub struct EdgeBuildState { pub doc: ServerEntity, pub source: EntityId, pub just_started: bool }

#[derive(Debug, Clone)]
pub struct EdgeMenuState { pub doc: ServerEntity, pub source: EntityId, pub target: EntityId, pub pos: egui::Pos2, pub just_opened: bool, pub filter: String }

#[derive(Debug, Clone)]
pub struct PreviewEdge { pub doc: ServerEntity, pub source: EntityId, pub target: EntityId }
