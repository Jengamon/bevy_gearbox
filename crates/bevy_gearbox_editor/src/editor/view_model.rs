use std::collections::HashMap;
use bevy::prelude::*;
use bevy_egui::egui;
use crate::model::{StateMachineGraph, NodeId, EdgeId};
use super::canvas::CanvasTransform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiNodeKind { Leaf, Parent, Parallel }

#[derive(Debug, Clone)]
pub struct UiNode {
    pub id: NodeId,
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
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
    pub label: String,
    pub pill: EdgePill,
}

#[derive(Debug, Default)]
pub struct GraphDoc {
    pub graph: Option<StateMachineGraph>,
    pub node_views: HashMap<NodeId, UiNode>,
    pub edge_views: HashMap<EdgeId, UiEdge>,
    /// World↔screen mapping for this document
    pub transform: CanvasTransform,
    /// Deterministic draw order for nodes (parents before children)
    pub draw_order_nodes: Vec<NodeId>,
    /// Deterministic draw order for edges
    pub draw_order_edges: Vec<EdgeId>,
}


