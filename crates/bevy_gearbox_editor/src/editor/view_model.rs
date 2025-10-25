use std::collections::HashMap;
use bevy::prelude::*;
use bevy_egui::egui;
use crate::model::{StateMachineGraph, EntityId};
use super::canvas::CanvasTransform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiNodeKind { Leaf, Parent, Parallel }

#[derive(Debug, Clone)]
pub struct UiNode {
    pub id: EntityId,
    pub rect: egui::Rect,
    pub kind: UiNodeKind,
    pub label: String,
    pub is_container: bool,
}

#[derive(Debug, Clone)]
pub struct EdgePill {
    pub pos: egui::Pos2,
    pub offset_from_midpoint: egui::Vec2,
    pub dragging: bool,
}

#[derive(Debug, Clone)]
pub struct UiEdge {
    pub id: EntityId,
    pub source: EntityId,
    pub target: EntityId,
    pub label: String,
    pub pill: EdgePill,
    /// Parent container that governs pill transform/clamping (sibling of target)
    pub pill_parent: Option<EntityId>,
}

#[derive(Debug, Default)]
pub struct GraphDoc {
    pub graph: Option<StateMachineGraph>,
    pub node_views: HashMap<EntityId, UiNode>,
    pub edge_views: HashMap<EntityId, UiEdge>,
    /// World↔screen mapping for this document
    pub transform: CanvasTransform,
    /// Deterministic draw order for nodes (parents before children)
    pub draw_order_nodes: Vec<EntityId>,
    /// Deterministic draw order for edges
    pub draw_order_edges: Vec<EntityId>,
    /// Unified drag state for either a node or a pill (by EntityId)
    pub dragging: Option<EntityId>,
    pub drag_anchor_world: Option<egui::Vec2>,
}


