use std::collections::HashMap;
use bevy::prelude::*;
use bevy_egui::egui;
use crate::model::{StateMachineGraph, EntityId};
use super::canvas::CanvasTransform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiNodeKind { Leaf, Parent, Parallel }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiViewKind {
    Leaf,
    Parent,
    Parallel,
    Edge { source: EntityId, target: EntityId },
}

#[derive(Debug, Clone)]
pub struct PillData {
    pub center: egui::Pos2,
    pub offset_from_midpoint: egui::Vec2,
    pub dragging: bool,
}

#[derive(Debug, Clone)]
pub struct UiView {
    pub id: EntityId,
    /// For states, this is the state rect; for edges, this is the pill rect in world space.
    pub rect: egui::Rect,
    pub kind: UiViewKind,
    pub label: String,
    /// Edge-only: pill info (None for state views)
    pub pill: Option<PillData>,
}

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
    /// World↔screen mapping for this document
    pub transform: CanvasTransform,
    /// Unified drag state for either a node or a pill (by EntityId)
    pub dragging: Option<EntityId>,
    pub drag_anchor_world: Option<egui::Vec2>,
    /// Unified view map (nodes and edges)
    pub views: HashMap<EntityId, UiView>,
    /// Unified draw order (parents, edges, leaves)
    pub draw_order: Vec<EntityId>,
    /// Transform children: node -> [child nodes and edge pills under this container]
    pub transform_children: HashMap<EntityId, Vec<EntityId>>,
    /// Transform parent for each view: node -> parent node; edge pill -> pill parent
    pub transform_parent: HashMap<EntityId, Option<EntityId>>,
    /// Mapping of parent -> initial child state (if any)
    pub initial_child_of: HashMap<EntityId, EntityId>,
    /// Set of nodes that are the initial child of their parent
    pub is_initial_child: std::collections::HashSet<EntityId>,
}


impl GraphDoc {
    /// Returns the rect for a unified view entity (if present).
    pub fn get_rect(&self, id: &EntityId) -> Option<egui::Rect> {
        self.views.get(id).map(|v| v.rect)
    }

    /// Sets the rect for a unified view entity (no-ops if not present).
    pub fn set_rect(&mut self, id: &EntityId, rect: egui::Rect) {
        if let Some(v) = self.views.get_mut(id) { v.rect = rect; }
    }

    /// Returns the transform parent for an entity (node parent or pill parent).
    pub fn parent_of(&self, id: &EntityId) -> Option<EntityId> {
        self.transform_parent.get(id).and_then(|p| *p)
    }
}


