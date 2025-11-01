use bevy::prelude::*;
use std::collections::HashMap;
use crate::types::EntityId;
use super::view_model::GraphDoc;
use bevy_egui::egui;

#[derive(Debug, Default, Resource)]
pub struct Workspace {
    pub docs: HashMap<EntityId, GraphDoc>,
    pub selection: Option<EntityId>,
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
    /// One-shot commit for creating a transition edge (doc-local)
    pub pending_edge_create: Option<PendingEdgeCreate>,
    /// Pending machine graph refreshes to request over the network
    pub pending_fetch_docs: Vec<EntityId>,
}

#[derive(Debug, Clone)]
pub struct RenameInline { pub doc: EntityId, pub target: EntityId, pub text: String }

#[derive(Debug, Clone)]
pub struct EdgeBuildState { pub doc: EntityId, pub source: EntityId, pub just_started: bool }

#[derive(Debug, Clone)]
pub struct EdgeMenuState { pub doc: EntityId, pub source: EntityId, pub target: EntityId, pub pos: egui::Pos2, pub just_opened: bool, pub filter: String }

#[derive(Debug, Clone)]
pub struct PreviewEdge { pub doc: EntityId, pub source: EntityId, pub target: EntityId }

#[derive(Debug, Clone)]
pub struct PendingEdgeCreate { pub doc: EntityId, pub source: EntityId, pub target: EntityId, pub kind: String }
