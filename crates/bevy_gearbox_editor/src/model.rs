use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::types::ServerEntity;
use crate::component as c;

/// Stable Rust type path of a component (e.g. "bevy_ecs::name::Name").
pub(crate) type TypePathString = String;

/// Locally generated identifier used before an entity exists on the server.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct LocalId(pub u64);

/// An entity identifier that may be a server-backed id or a local temporary id.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum EntityId {
    Server(ServerEntity),
    Local(LocalId),
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityId::Server(e) => write!(f, "{}", e.0),
            EntityId::Local(LocalId(id)) => write!(f, "local:{id}"),
        }
    }
}

/// Node identifier wrapper for clarity at use sites.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct NodeId(pub EntityId);

impl Default for NodeId {
    fn default() -> Self {
        Self(EntityId::Local(LocalId(0)))
    }
}

/// Edge identifier wrapper for clarity at use sites.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct EdgeId(pub EntityId);

/// Tracks structural and data changes in the editor model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DirtyFlags {
    /// True if the set of components or their values changed.
    pub(crate) components: bool,
    /// True if hierarchy or connections changed.
    pub(crate) structure: bool,
}

/// Per-component entry stored in a `ComponentBag`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ComponentEntry {
    /// Stable type path for this component.
    pub(crate) type_path: TypePathString,
    /// Component value as JSON; may be round-tripped even when unknown to the client.
    pub(crate) value_json: JsonValue,
    /// Dirty if value differs from last successful server state.
    pub(crate) dirty: bool,
    /// Optional validation errors for UI presentation.
    pub(crate) validation_errors: Vec<String>,
    /// Opaque server version/epoch for conflict detection (if available).
    pub(crate) server_version: Option<u64>,
}

impl ComponentEntry {
    pub(crate) fn new(type_path: impl Into<TypePathString>, value_json: JsonValue) -> Self {
        Self {
            type_path: type_path.into(),
            value_json,
            dirty: false,
            validation_errors: Vec::new(),
            server_version: None,
        }
    }
}

/// Bag of components for a node or edge, keyed by type path.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct ComponentBag {
    pub(crate) entries: HashMap<TypePathString, ComponentEntry>,
}

impl ComponentBag {
    pub(crate) fn contains(&self, type_path: &str) -> bool { self.entries.contains_key(type_path) }
    pub(crate) fn get(&self, type_path: &str) -> Option<&ComponentEntry> { self.entries.get(type_path) }
    pub(crate) fn get_mut(&mut self, type_path: &str) -> Option<&mut ComponentEntry> { self.entries.get_mut(type_path) }
    pub(crate) fn insert(&mut self, entry: ComponentEntry) { self.entries.insert(entry.type_path.clone(), entry); }
    pub(crate) fn remove(&mut self, type_path: &str) -> Option<ComponentEntry> { self.entries.remove(type_path) }
    pub(crate) fn keys(&self) -> impl Iterator<Item=&str> { self.entries.keys().map(|s| s.as_str()) }
}

/// A state in the machine hierarchy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct StateNode {
    pub(crate) id: NodeId,
    pub(crate) parent: Option<NodeId>,
    /// Child state nodes in display order.
    pub(crate) children: Vec<NodeId>,
    pub(crate) components: ComponentBag,
    /// Derived, non-authoritative label (e.g., from `Name`).
    pub(crate) display_name: Option<String>,
    /// Dirty tracking for this node.
    pub(crate) dirty: DirtyFlags,
    /// Opaque server version for the node entity (not per-component).
    pub(crate) server_version: Option<u64>,
}

impl StateNode {
    pub(crate) fn new(id: NodeId) -> Self {
        Self {
            id,
            parent: None,
            children: Vec::new(),
            components: ComponentBag::default(),
            display_name: None,
            dirty: DirtyFlags::default(),
            server_version: None,
        }
    }
}

impl Default for StateNode {
    fn default() -> Self {
        Self::new(NodeId::default())
    }
}

/// A transition edge between states.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Edge {
    pub(crate) id: EdgeId,
    pub(crate) source: NodeId,
    pub(crate) target: NodeId,
    pub(crate) components: ComponentBag,
    /// Derived label for display (e.g., event name), not authoritative.
    pub(crate) display_label: Option<String>,
    pub(crate) dirty: DirtyFlags,
    pub(crate) server_version: Option<u64>,
}

impl Edge {
    pub(crate) fn new(id: EdgeId, source: NodeId, target: NodeId) -> Self {
        Self {
            id,
            source,
            target,
            components: ComponentBag::default(),
            display_label: None,
            dirty: DirtyFlags::default(),
            server_version: None,
        }
    }
}

/// Graph container with adjacency indices for efficient queries.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct StateMachineGraph {
    /// The root state of the machine. The root is also present in `nodes`.
    pub(crate) root: NodeId,
    pub(crate) nodes: HashMap<NodeId, StateNode>,
    pub(crate) edges: HashMap<EdgeId, Edge>,
    /// Cached adjacency for quick traversal; rebuild or keep in sync on edits.
    pub(crate) adjacency_out: HashMap<NodeId, Vec<EdgeId>>,
    pub(crate) adjacency_in: HashMap<NodeId, Vec<EdgeId>>,
}

impl StateMachineGraph {
    /// Create a new graph with the provided root state. The root is inserted into `nodes`.
    pub(crate) fn new(root: StateNode) -> Self {
        let root_id = root.id;
        let mut nodes = HashMap::new();
        nodes.insert(root_id, root);
        Self {
            root: root_id,
            nodes,
            edges: HashMap::new(),
            adjacency_out: HashMap::new(),
            adjacency_in: HashMap::new(),
        }
    }

    /// Returns all outgoing edges for a node.
    pub(crate) fn outgoing(&self, node: &NodeId) -> &[EdgeId] {
        self.adjacency_out.get(node).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Returns all incoming edges for a node.
    pub(crate) fn incoming(&self, node: &NodeId) -> &[EdgeId] {
        self.adjacency_in.get(node).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// Mapping used to resolve local ids to server entities after creation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct IdMapping {
    pub(crate) local_to_server: HashMap<LocalId, ServerEntity>,
}

impl IdMapping {
    pub(crate) fn resolve_entity(&self, id: &EntityId) -> EntityId {
        match id {
            EntityId::Server(_) => *id,
            EntityId::Local(local) => self
                .local_to_server
                .get(local)
                .copied()
                .map(EntityId::Server)
                .unwrap_or(EntityId::Local(*local)),
        }
    }
}

impl fmt::Display for StateMachineGraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Collect names for nodes lazily
        let mut names: HashMap<NodeId, String> = HashMap::new();

        // Helper to extract a displayable name from a node's components
        let mut get_node_name = |id: &NodeId| -> String {
            if let Some(n) = names.get(id) { return n.clone(); }
            let name = self.nodes.get(id)
                .and_then(|node| node.display_name.clone().or_else(|| extract_name_from_bag(&node.components)))
                .unwrap_or_default();
            names.insert(*id, name.clone());
            name
        };

        // Traverse states from root using children relationships, tracking depth
        let mut ordered: Vec<(NodeId, usize)> = Vec::new();
        let mut stack: Vec<(NodeId, usize)> = Vec::new();
        let mut visited: HashSet<NodeId> = HashSet::new();
        if self.nodes.contains_key(&self.root) { stack.push((self.root, 0)); }
        while let Some((id, depth)) = stack.pop() {
            if !visited.insert(id) { continue; }
            ordered.push((id, depth));
            if let Some(node) = self.nodes.get(&id) {
                // Push children in reverse to preserve original order on pop
                for child in node.children.iter().rev() { stack.push((*child, depth + 1)); }
            }
        }

        // Build edges list in a stable order (by source appearance in states)
        let mut edges_formatted: Vec<String> = Vec::new();
        for (state, _depth) in &ordered {
            // Prefer adjacency if present; otherwise scan all edges
            if let Some(out_ids) = self.adjacency_out.get(state) {
                for e_id in out_ids {
                    if let Some(edge) = self.edges.get(e_id) {
                        let source_name = get_node_name(&edge.source);
                        let mut target_name = get_node_name(&edge.target);
                        if target_name.is_empty() {
                            if let Some(s) = extract_name_from_bag(&edge.components) {
                                if let Some(arrow) = s.find("->") {
                                    let rhs = s[arrow+2..].trim();
                                    let rhs = if let Some(paren) = rhs.find('(') { &rhs[..paren] } else { rhs };
                                    let rhs = rhs.trim();
                                    if !rhs.is_empty() { target_name = rhs.to_string(); }
                                }
                            }
                        }
                        let label = choose_edge_label_bag(&edge.components);
                        edges_formatted.push(format!("{} - {} -> {}", source_name, label, target_name));
                    }
                }
            } else {
                for edge in self.edges.values() {
                    if &edge.source == state {
                        let source_name = get_node_name(&edge.source);
                        let mut target_name = get_node_name(&edge.target);
                        if target_name.is_empty() {
                            if let Some(s) = extract_name_from_bag(&edge.components) {
                                if let Some(arrow) = s.find("->") {
                                    let rhs = s[arrow+2..].trim();
                                    let rhs = if let Some(paren) = rhs.find('(') { &rhs[..paren] } else { rhs };
                                    let rhs = rhs.trim();
                                    if !rhs.is_empty() { target_name = rhs.to_string(); }
                                }
                            }
                        }
                        let label = choose_edge_label_bag(&edge.components);
                        edges_formatted.push(format!("{} - {} -> {}", source_name, label, target_name));
                    }
                }
            }
        }

        // Header: root node name
        let header = get_node_name(&self.root);
        writeln!(f, "{}", header)?;
        // Print states excluding the root; indent by depth (3 spaces per level)
        for (id, depth) in ordered.iter().skip(1) {
            let name = get_node_name(id);
            for _ in 0..*depth { write!(f, "   ")?; }
            writeln!(f, "{}", name)?;
        }
        writeln!(f)?;
        for line in edges_formatted {
            writeln!(f, "{}", line)?;
        }
        Ok(())
    }
}

fn extract_name_from_bag(bag: &ComponentBag) -> Option<String> {
    let val = bag.entries.get(c::NAME)?.value_json.clone();
    if let Some(s) = val.as_str() { return Some(s.to_string()); }
    if let JsonValue::Object(obj) = val {
        for v in obj.values() { if let Some(s) = v.as_str() { return Some(s.to_string()); } }
    }
    None
}

fn choose_edge_label_bag(bag: &ComponentBag) -> String {
    let keys: HashSet<String> = bag.entries.keys().cloned().collect();
    if keys.contains(c::ALWAYS_EDGE) { return "Always".to_string(); }
    if keys.contains(c::AFTER) { return "After".to_string(); }

    // Prefer generic EventEdge types
    let mut event_edge_types: Vec<&String> = keys.iter().filter(|s| s.contains(c::EVENT_EDGE_SUBSTR)).collect();
    event_edge_types.sort();
    if let Some(ty) = event_edge_types.first() {
        let s = ty.as_str();
        if let (Some(start), Some(end)) = (s.find('<'), s.rfind('>')) {
            if end > start + 1 {
                let inner = &s[start + 1..end];
                if let Some(simple) = inner.rsplit("::").next() {
                    return simple.to_string();
                }
                return inner.to_string();
            }
        }
        if let Some(simple) = s.rsplit("::").next() { return simple.to_string(); }
        return (*ty).clone();
    }

    // Fallback: try to extract a label from the edge's Name like "... (OnComplete)"
    if let Some(name_val) = bag.entries.get(c::NAME).map(|e| e.value_json.clone()) {
        if let Some(name) = name_val.as_str().map(|s| s.to_string()).or_else(|| {
            if let JsonValue::Object(obj) = name_val {
                for v in obj.values() {
                    if let Some(s) = v.as_str() { return Some(s.to_string()); }
                }
            }
            None
        }) {
            if let (Some(l), Some(r)) = (name.rfind('('), name.rfind(')')) {
                if r > l + 1 {
                    return name[l+1..r].to_string();
                }
            }
        }
    }

    "Edge".to_string()
}



