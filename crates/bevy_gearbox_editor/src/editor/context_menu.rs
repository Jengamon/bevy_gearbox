use crate::component as c;
use crate::model::{EntityId, StateMachineGraph};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItemKind {
    MakeLeaf,
    MakeParent,
    MakeParallel,
    Save,
    Delete,
    /// Parent is the owner of InitialState; this node becomes the new initial
    MakeInitial { parent: EntityId },
    AddChild,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub label: &'static str,
    pub kind: MenuItemKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuSelection {
    MakeLeaf { target: EntityId },
    MakeParent { target: EntityId },
    MakeParallel { target: EntityId },
    SaveStateMachine { target: EntityId },
    DeleteEntity { target: EntityId },
    MakeInitial { parent: EntityId, new_initial: EntityId },
    AddChildStateMachine { target: EntityId },
}

/// Build context menu items for a right-clicked node using only the cached model.
/// No side effects; the returned items should be used to emit selection events.
pub fn build_context_menu(graph: &StateMachineGraph, id: EntityId) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = Vec::new();

    let Some(node) = graph.nodes.get(&id) else { return items; };

    let has_children = !node.children.is_empty();
    let has_initial_state = node.components.contains(c::INITIAL_STATE);
    let has_parallel = node.components.contains(c::PARALLEL);
    let has_state_machine = node.components.contains(c::STATE_MACHINE);
    let has_state_children_capability = node.components.contains(c::STATE_CHILDREN);

    let parent_and_lacks_initial = node.parent.and_then(|pid| {
        graph
            .nodes
            .get(&pid)
            .map(|p| (!p.components.contains(c::INITIAL_STATE)).then_some(pid))
            .flatten()
    });

    // Make Leaf (only when there are children)
    if has_children {
        items.push(MenuItem { label: "Make Leaf", kind: MenuItemKind::MakeLeaf });
    }

    // Make Parent (when this node does not have InitialState)
    if !has_initial_state {
        items.push(MenuItem { label: "Make Parent", kind: MenuItemKind::MakeParent });
    }

    // Make Parallel (when this node does not have Parallel)
    if !has_parallel {
        items.push(MenuItem { label: "Make Parallel", kind: MenuItemKind::MakeParallel });
    }

    // Save As (when this node has StateMachine)
    if has_state_machine {
        items.push(MenuItem { label: "Save As", kind: MenuItemKind::Save });
    }

    // Delete (always)
    items.push(MenuItem { label: "Delete", kind: MenuItemKind::Delete });

    // Make Initial (when node has a parent and the parent lacks InitialState)
    if let Some(parent) = parent_and_lacks_initial {
        items.push(MenuItem { label: "Make Initial", kind: MenuItemKind::MakeInitial { parent } });
    }

    // Add Child (when node has StateChildren capability)
    if has_state_children_capability {
        items.push(MenuItem { label: "Add Child", kind: MenuItemKind::AddChild });
    }

    items
}


