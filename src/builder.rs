use bevy::prelude::*;
use std::{collections::{HashMap, HashSet}, time::Duration};

use crate::{
    InitialState,
    StateMachine,
    SubstateOf,
    transitions::{After, AlwaysEdge, EdgeKind, EventEdge, Source, Target},
};

type Customizer = Box<dyn Fn(&mut EntityCommands) + Send + Sync + 'static>;

/// Fluent builder for creating hierarchical state machines with named states and edges.
///
/// - States are referenced by name paths. Sibling names must be unique; global uniqueness is not required.
/// - Paths are relative to the current node, unless prefixed with `/`, which makes them absolute from the machine root.
/// - Transitions (edges) can be Always or Event-driven and support optional `After` delay and `EdgeKind`.
pub struct StateMachineBuilder {
    root_name: String,
    nodes: Vec<NodeDef>,
    initials: Vec<InitialDef>,
    edges: Vec<EdgeDef>,
    // Detect duplicate state paths early
    seen_paths: HashSet<Vec<String>>,
    // Deferred state customizers applied after entity spawn
    state_customizers: Vec<(Vec<String>, Customizer)>,
}

struct NodeDef {
    // Full path from the root (excluding the root's own name), e.g. ["Main Menu", "Buttons"].
    full_path: Vec<String>,
}

struct InitialDef {
    parent_path: Vec<String>,
    child_name: String,
}

enum EdgeInserter {
    Always,
    Event(Box<dyn Fn(&mut EntityCommands) + Send + Sync + 'static>),
}

struct EdgeDef {
    source_path: Vec<String>,
    target_raw: String,
    name: Option<String>,
    kind: EdgeKind,
    after: Option<Duration>,
    inserter: EdgeInserter,
    extras: Vec<Customizer>,
}

pub struct StateNodeBuilder<'a> {
    inner: &'a mut StateMachineBuilder,
    // Full path from the root (excluding root name)
    current_path: Vec<String>,
}

pub struct EdgeConfig {
    name: Option<String>,
    kind: EdgeKind,
    after: Option<Duration>,
    extras: Vec<Customizer>,
}

impl EdgeConfig {
    pub fn named(&mut self, name: impl Into<String>) -> &mut Self { self.name = Some(name.into()); self }
    pub fn internal(&mut self) -> &mut Self { self.kind = EdgeKind::Internal; self }
    pub fn external(&mut self) -> &mut Self { self.kind = EdgeKind::External; self }
    pub fn after(&mut self, duration: Duration) -> &mut Self { self.after = Some(duration); self }
    pub fn after_secs(&mut self, secs: f32) -> &mut Self { self.after(Duration::from_secs_f32(secs)); self }
    pub fn commands(&mut self, f: impl Fn(&mut EntityCommands) + Send + Sync + 'static) -> &mut Self { self.extras.push(Box::new(f)); self }
}

/// Typed event-edge configuration that also allows specifying a validator
pub struct EventEdgeConfig<E: crate::TransitionEvent> {
    base: EdgeConfig,
    validator: Option<<E as crate::TransitionEvent>::Validator>,
}

impl<E: crate::TransitionEvent> EventEdgeConfig<E> {
    #[inline] pub fn named(&mut self, name: impl Into<String>) -> &mut Self { self.base.named(name); self }
    #[inline] pub fn internal(&mut self) -> &mut Self { self.base.internal(); self }
    #[inline] pub fn external(&mut self) -> &mut Self { self.base.external(); self }
    #[inline] pub fn after(&mut self, duration: Duration) -> &mut Self { self.base.after(duration); self }
    #[inline] pub fn after_secs(&mut self, secs: f32) -> &mut Self { self.base.after_secs(secs); self }
    #[inline] pub fn commands(&mut self) -> EdgeEntityCommands<'_> { EdgeEntityCommands { base: &mut self.base } }

    /// Set a per-edge validator that must accept events of this type for the edge to fire
    #[inline]
    pub fn validator(&mut self, validator: <E as crate::TransitionEvent>::Validator) -> &mut Self { self.validator = Some(validator); self }

    /// Set both the name and validator from a single value that can convert into both.
    /// Useful when the validator and display name are the same conceptual value.
    #[inline]
    pub fn nv<V>(&mut self, v: V) -> &mut Self
    where
        V: Into<<E as crate::TransitionEvent>::Validator> + Into<String> + Clone,
    {
        let name: String = v.clone().into();
        let validator: <E as crate::TransitionEvent>::Validator = v.into();
        self.base.named(name);
        self.validator = Some(validator);
        self
    }
}

impl StateMachineBuilder {
    pub fn new<F>(root_name: impl Into<String>, f: F) -> Self
    where
        F: for<'b> FnOnce(&'b mut StateNodeBuilder<'b>),
    {
        let mut s = Self {
            root_name: root_name.into(),
            nodes: Vec::new(),
            initials: Vec::new(),
            edges: Vec::new(),
            seen_paths: HashSet::new(),
            state_customizers: Vec::new(),
        };
        let mut node = StateNodeBuilder { inner: &mut s, current_path: vec![] };
        f(&mut node);
        s
    }

    /// Build the machine and return the root entity with `StateMachine`.
    pub fn build(mut self, commands: &mut Commands) -> Entity {
        // 1) Spawn root
        let root = commands
            .spawn(Name::new(self.root_name.clone()))
            .id();

        // Apply any root-level customizers (path == [])
        for (path, customize) in self.state_customizers.iter() {
            if path.is_empty() {
                let mut ec = commands.entity(root);
                customize(&mut ec);
            }
        }

        // 2) Create states in parent-before-child order
        let mut path_to_entity: HashMap<Vec<String>, Entity> = HashMap::new();
        path_to_entity.insert(vec![], root);

        self.nodes.sort_by_key(|n| n.full_path.len());

        for node in &self.nodes {
            let (parent_path, local_name) = split_parent_and_leaf(&node.full_path);
            let parent_entity = *path_to_entity
                .get(parent_path)
                .expect("Parent path must exist");
            let entity = commands
                .spawn((Name::new(local_name.clone()), SubstateOf(parent_entity)))
                .id();
            path_to_entity.insert(node.full_path.clone(), entity);
            // Apply any deferred customizers for this state
            for (path, customize) in self.state_customizers.iter() {
                if *path == node.full_path {
                    let mut ec = commands.entity(entity);
                    customize(&mut ec);
                }
            }
        }

        // 3) Apply InitialState markers
        for init in &self.initials {
            let parent = *path_to_entity
                .get(&init.parent_path)
                .expect("Initial parent must exist");
            let mut child_path = init.parent_path.clone();
            child_path.push(init.child_name.clone());
            let child = *path_to_entity
                .get(&child_path)
                .unwrap_or_else(|| panic!(
                    "Initial child {:?} not found under {:?}",
                    init.child_name, init.parent_path
                ));
            commands.entity(parent).insert(InitialState(child));
        }

        // 4) Create edges
        let all_paths: HashSet<Vec<String>> = self.nodes.iter().map(|n| n.full_path.clone()).collect();
        for edge in self.edges {
            let source = *path_to_entity
                .get(&edge.source_path)
                .expect("Edge source must exist");
            let target_path = resolve_target_path(&edge.source_path, &edge.target_raw, &all_paths);
            let target = *path_to_entity
                .get(&target_path)
                .unwrap_or_else(|| panic!("Edge target not found. source={:?} raw='{}'", edge.source_path, edge.target_raw));

            let mut ec = commands.spawn_empty();
            if let Some(n) = edge.name.as_ref() {
                ec.insert(Name::new(n.clone()));
            }
            ec.insert((Source(source), Target(target), edge.kind));

            match edge.inserter {
                EdgeInserter::Always => {
                    ec.insert(AlwaysEdge);
                }
                EdgeInserter::Event(insert_fn) => {
                    insert_fn(&mut ec);
                }
            }

            if let Some(dur) = edge.after {
                ec.insert(After { duration: dur });
            }
            // Apply extra edge customizers
            for customize in edge.extras {
                customize(&mut ec);
            }
        }

        commands.entity(root).insert(StateMachine::new());

        root
    }
}

impl<'a> StateNodeBuilder<'a> {
    fn push_node_here(&mut self) {
        // Idempotent: defining the same node twice is allowed; only create once
        if self.inner.seen_paths.insert(self.current_path.clone()) {
            self.inner.nodes.push(NodeDef { full_path: self.current_path.clone() });
        }
    }

    /// Add a child state under this node.
    pub fn substate<F>(&mut self, name: impl Into<String>, f: F) -> &mut Self
    where
        F: for<'b> FnOnce(&'b mut StateNodeBuilder<'b>),
    {
        let name = name.into();
        let mut child = StateNodeBuilder {
            inner: self.inner,
            current_path: {
                let mut p = self.current_path.clone();
                p.push(name);
                p
            },
        };
        child.push_node_here();
        f(&mut child);
        self
    }

    pub fn commands(&mut self) -> StateEntityCommands<'_> {
        StateEntityCommands { inner: self.inner, path: self.current_path.clone() }
    }

    /// Mark one of this node's direct children as its initial substate by name.
    pub fn initial(&mut self, child_name: impl Into<String>) -> &mut Self {
        let child_name = child_name.into();
        self.inner.initials.push(InitialDef {
            parent_path: self.current_path.clone(),
            child_name,
        });
        self
    }

    /// Add an Always edge from this node to a target by name/path.
    pub fn always(
        &mut self,
        to: impl AsRef<str>,
        configure: impl FnOnce(&mut EdgeConfig),
    ) -> &mut Self {
        self.add_edge_generic(to, EdgeInserter::Always, configure)
    }

    /// Add an Event edge from this node to a target by name/path with optional validator.
    pub fn edge<E>(&mut self, to: impl AsRef<str>, configure: impl FnOnce(&mut EventEdgeConfig<E>)) -> &mut Self
    where
        E: crate::registration::RegisteredTransitionEvent + crate::TransitionEvent + 'static,
    {
        // Start with defaults and let the user configure
        let mut cfg = EventEdgeConfig::<E> { base: EdgeConfig { name: None, kind: EdgeKind::External, after: None, extras: Vec::new() }, validator: None };
        configure(&mut cfg);

        let validator = cfg.validator;
        self.inner.edges.push(EdgeDef {
            source_path: self.current_path.clone(),
            target_raw: to.as_ref().to_string(),
            name: cfg.base.name,
            kind: cfg.base.kind,
            after: cfg.base.after,
            inserter: EdgeInserter::Event(Box::new(move |ec| {
                let mut edge = EventEdge::<E>::default();
                edge.validator = validator.clone();
                ec.insert(edge);
            })),
            extras: cfg.base.extras,
        });
        self
    }

    /// Sugar: add an Event edge and set both its name and validator from a single value
    /// that can convert into both a `String` (for the name) and `E::Validator`.
    pub fn edge_from<E>(&mut self, to: impl AsRef<str>, v: impl Into<<E as crate::TransitionEvent>::Validator> + Into<String> + Clone) -> &mut Self
    where
        E: crate::registration::RegisteredTransitionEvent + crate::TransitionEvent + 'static,
    {
        let val = v.clone();
        self.edge::<E>(to, move |e| { e.nv(val); });
        self
    }

    fn add_edge_generic(
        &mut self,
        to: impl AsRef<str>,
        inserter: EdgeInserter,
        configure: impl FnOnce(&mut EdgeConfig),
    ) -> &mut Self {
        let mut cfg = EdgeConfig { name: None, kind: EdgeKind::External, after: None, extras: Vec::new() };
        configure(&mut cfg);
        self.inner.edges.push(EdgeDef {
            source_path: self.current_path.clone(),
            target_raw: to.as_ref().to_string(),
            name: cfg.name,
            kind: cfg.kind,
            after: cfg.after,
            inserter,
            extras: cfg.extras,
        });
        self
    }
}

// ---------------- helpers ----------------

fn split_parent_and_leaf(path: &[String]) -> (&[String], String) {
    assert!(!path.is_empty(), "leaf must have at least one segment");
    let parent = &path[..path.len() - 1];
    let leaf = path.last().unwrap().clone();
    (parent, leaf)
}

// Convert "/A/B/C" or "A/B/C" into ["A","B","C"]. Trims whitespace and drops empties.
fn split_path(input: &str) -> Vec<String> {
    input
        .trim()
        .trim_start_matches('/')
        .split('/')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn canonicalize_relative(base: &[String], segments: &[String]) -> Vec<String> {
    let mut out = base.to_vec();
    for seg in segments {
        if seg == "." { continue; }
        if seg == ".." { out.pop(); continue; }
        out.push(seg.clone());
    }
    out
}

// Resolve target with sibling-first semantics and support for ../ and ./
fn resolve_target_path(source_path: &[String], raw: &str, all_paths: &HashSet<Vec<String>>) -> Vec<String> {
    if raw.starts_with('/') {
        return split_path(raw);
    }

    let segs = split_path(raw);
    let has_dots = segs.iter().any(|s| s == "." || s == "..");
    let parent_path: &[String] = if source_path.is_empty() { &[] } else { &source_path[..source_path.len()-1] };

    if has_dots {
        let cand = canonicalize_relative(source_path, &segs);
        if all_paths.contains(&cand) { return cand; }
    } else {
        // 1) siblings of current: parent + segs
        let mut cand = parent_path.to_vec();
        cand.extend(segs.iter().cloned());
        if all_paths.contains(&cand) { return cand; }

        // 2) children of current: current + segs
        let mut cand2 = source_path.to_vec();
        cand2.extend(segs.iter().cloned());
        if all_paths.contains(&cand2) { return cand2; }

        // 3) climb ancestors: ancestor + segs
        let mut depth = parent_path.len();
        while depth > 0 {
            let mut cand3 = parent_path[..depth].to_vec();
            cand3.extend(segs.iter().cloned());
            if all_paths.contains(&cand3) { return cand3; }
            depth -= 1;
        }

        // 4) root + segs
        if all_paths.contains(&segs) { return segs; }
    }

    panic!("Could not resolve target '{}'", raw);
}


// --------------- command-like customizers ---------------

/// A lightweight, deferred wrapper that records `EntityCommands` operations
/// to be applied to the state's entity when the machine is built.
pub struct StateEntityCommands<'a> {
    inner: &'a mut StateMachineBuilder,
    path: Vec<String>,
}

impl<'a> StateEntityCommands<'a> {
    /// Queue an `insert` on the target entity.
    pub fn insert<T>(&mut self, bundle: T) -> &mut Self
    where
        T: Bundle + Send + Sync + 'static,
    {
        let path = self.path.clone();
        let cell = std::sync::Mutex::new(Some(bundle));
        self.inner.state_customizers.push((path, Box::new(move |ec| {
            let mut guard = cell.lock().unwrap();
            let value = guard.take().expect("StateEntityCommands.insert called more than once");
            ec.insert(value);
        })));
        self
    }
}


/// A lightweight, deferred wrapper that records `EntityCommands` operations
/// to be applied to the edge's entity when the machine is built.
pub struct EdgeEntityCommands<'a> {
    base: &'a mut EdgeConfig,
}

impl<'a> EdgeEntityCommands<'a> {
    /// Queue an `insert` on the edge entity.
    pub fn insert<T>(&mut self, bundle: T) -> &mut Self
    where
        T: Bundle + Send + Sync + 'static,
    {
        let cell = std::sync::Mutex::new(Some(bundle));
        self.base.extras.push(Box::new(move |ec| {
            let mut guard = cell.lock().unwrap();
            let value = guard.take().expect("EdgeEntityCommands.insert called more than once");
            ec.insert(value);
        }));
        self
    }
}

