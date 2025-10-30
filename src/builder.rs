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
pub struct StateMachineBuilder<'w, 's> {
    commands: Commands<'w, 's>,
    root_entity: Entity,
    path_to_entity: HashMap<Vec<String>, Entity>,
    initials: Vec<InitialDef>,
    edges: Vec<EdgeDef>,
    seen_paths: HashSet<Vec<String>>,
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

pub struct StateCommands<'a, 'w, 's> {
    inner: &'a mut StateMachineBuilder<'w, 's>,
    // Full path from the root (excluding root name)
    current_path: Vec<String>,
    // entity for this node
    entity: Entity,
}

pub struct EdgeCommands {
    name: Option<String>,
    kind: EdgeKind,
    after: Option<Duration>,
    extras: Vec<Customizer>,
}

impl EdgeCommands {
    pub fn named(&mut self, name: impl Into<String>) -> &mut Self { self.name = Some(name.into()); self }
    pub fn internal(&mut self) -> &mut Self { self.kind = EdgeKind::Internal; self }
    pub fn external(&mut self) -> &mut Self { self.kind = EdgeKind::External; self }
    pub fn after(&mut self, duration: Duration) -> &mut Self { self.after = Some(duration); self }
    pub fn after_secs(&mut self, secs: f32) -> &mut Self { self.after(Duration::from_secs_f32(secs)); self }
    pub fn after_millis(&mut self, millis: u64) -> &mut Self { self.after(Duration::from_millis(millis)); self }
    pub fn commands(&mut self, f: impl Fn(&mut EntityCommands) + Send + Sync + 'static) -> &mut Self { self.extras.push(Box::new(f)); self }

    /// Insert a bundle onto this edge's entity when the machine is built.
    pub fn insert<T>(&mut self, bundle: T) -> &mut Self
    where
        T: Bundle + Send + Sync + 'static,
    {
        let cell = std::sync::Mutex::new(Some(bundle));
        self.extras.push(Box::new(move |ec| {
            let mut guard = cell.lock().unwrap();
            let value = guard.take().expect("EdgeConfig.insert called more than once");
            ec.insert(value);
        }));
        self
    }
}

/// Typed event-edge configuration that also allows specifying a validator
pub struct EventEdgeCommands<E: crate::TransitionEvent> {
    base: EdgeCommands,
    validator: Option<<E as crate::TransitionEvent>::Validator>,
}

impl<E: crate::TransitionEvent> EventEdgeCommands<E> {
    #[inline] pub fn named(&mut self, name: impl Into<String>) -> &mut Self { self.base.named(name); self }
    #[inline] pub fn internal(&mut self) -> &mut Self { self.base.internal(); self }
    #[inline] pub fn external(&mut self) -> &mut Self { self.base.external(); self }
    #[inline] pub fn after(&mut self, duration: Duration) -> &mut Self { self.base.after(duration); self }
    #[inline] pub fn after_secs(&mut self, secs: f32) -> &mut Self { self.base.after_secs(secs); self }
    #[inline] pub fn after_millis(&mut self, millis: u64) -> &mut Self { self.base.after_millis(millis); self }
    // Immediate builders: use `insert` directly on the config instead of a commands proxy

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

    /// Insert a bundle onto this edge's entity when the machine is built.
    #[inline]
    pub fn insert<T>(&mut self, bundle: T) -> &mut Self
    where
        T: Bundle + Send + Sync + 'static,
    {
        self.base.insert(bundle);
        self
    }
}

impl<'w, 's> StateMachineBuilder<'w, 's> {
    /// Spawn the state machine immediately and return the root entity.
    pub fn spawn(commands: Commands<'w, 's>, root_name: impl Into<String>, f: impl FnOnce(&mut StateCommands<'_, 'w, 's>)) -> Entity
    where
        'w: 's,
    {
        let root_name = root_name.into();
        let mut s = Self {
            commands,
            root_entity: Entity::PLACEHOLDER,
            path_to_entity: HashMap::new(),
            initials: Vec::new(),
            edges: Vec::new(),
            seen_paths: HashSet::new(),
        };
        let root = s.commands.spawn(Name::new(root_name.clone())).id();
        s.root_entity = root;
        s.path_to_entity.insert(vec![], root);
        s.seen_paths.insert(vec![]);

        let mut node = StateCommands { inner: &mut s, current_path: vec![], entity: root };
        f(&mut node);

        // finalize
        s.apply_initials_and_edges();
        s.commands.entity(root).insert(StateMachine::new());
        root
    }

    fn ensure_node(&mut self, path: &[String], parent: Entity, local_name: &str) -> Entity {
        if let Some(&e) = self.path_to_entity.get(path) { return e; }
        let e = self.commands.spawn((Name::new(local_name.to_string()), SubstateOf(parent))).id();
        self.path_to_entity.insert(path.to_vec(), e);
        self.seen_paths.insert(path.to_vec());
        e
    }

    fn apply_initials_and_edges(&mut self) {
        // Apply initials
        for init in &self.initials {
            let parent = *self.path_to_entity.get(&init.parent_path).expect("Initial parent must exist");
            let mut child_path = init.parent_path.clone();
            child_path.push(init.child_name.clone());
            let child = *self.path_to_entity.get(&child_path).unwrap_or_else(|| panic!(
                "Initial child {:?} not found under {:?}", init.child_name, init.parent_path
            ));
            self.commands.entity(parent).insert(InitialState(child));
        }

        // Edges
        let all_paths: HashSet<Vec<String>> = self.path_to_entity.keys().cloned().collect();
        for edge in self.edges.drain(..) {
            let source = *self.path_to_entity.get(&edge.source_path).expect("Edge source must exist");
            let target_path = resolve_target_path(&edge.source_path, &edge.target_raw, &all_paths);
            let target = *self.path_to_entity.get(&target_path).unwrap_or_else(|| panic!(
                "Edge target not found. source={:?} raw='{}'", edge.source_path, edge.target_raw
            ));

            let mut ec = self.commands.spawn_empty();
            if let Some(n) = edge.name.as_ref() { ec.insert(Name::new(n.clone())); }
            ec.insert((Source(source), Target(target), edge.kind));
            match edge.inserter { EdgeInserter::Always => { ec.insert(AlwaysEdge); }, EdgeInserter::Event(f) => { f(&mut ec); } }
            if let Some(dur) = edge.after { ec.insert(After { duration: dur }); }
            for customize in edge.extras { customize(&mut ec); }
        }
    }
}

impl<'a, 'w, 's> StateCommands<'a, 'w, 's> {
    fn ensure_here(&mut self) {
        if self.inner.path_to_entity.contains_key(&self.current_path) { return; }
        let (parent_path, local_name) = split_parent_and_leaf(&self.current_path);
        let parent_entity = *self.inner.path_to_entity.get(parent_path).expect("Parent path must exist");
        let entity = self.inner.ensure_node(&self.current_path, parent_entity, &local_name);
        self.entity = entity;
    }

    /// Add a child state under this node.
    pub fn substate<F>(&mut self, name: impl Into<String>, f: F) -> &mut Self
    where
        F: for<'b> FnOnce(&'b mut StateCommands<'b, 'w, 's>),
    {
        let name = name.into();
        let mut p = self.current_path.clone();
        p.push(name.clone());
        let entity = self.inner.ensure_node(&p, self.entity, &name);
        let mut child = StateCommands { inner: self.inner, current_path: p, entity };
        f(&mut child);
        self
    }

    /// Insert a bundle onto this state's entity when the machine is built.
    pub fn insert<T>(&mut self, bundle: T) -> &mut Self
    where
        T: Bundle + Send + Sync + 'static,
    {
        self.ensure_here();
        self.inner.commands.entity(self.entity).insert(bundle);
        self
    }

    /// Execute arbitrary commands on this state's entity immediately.
    pub fn commands(&mut self, f: impl FnOnce(&mut EntityCommands)) -> &mut Self {
        self.ensure_here();
        let mut ec = self.inner.commands.entity(self.entity);
        f(&mut ec);
        self
    }

    /// Get this state's entity id.
    pub fn id(&self) -> Entity { self.entity }

    /// Get the absolute path of this state as a slice.
    pub fn path(&self) -> &[String] { &self.current_path }

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
        configure: impl FnOnce(&mut EdgeCommands),
    ) -> &mut Self {
        self.add_edge_generic(to, EdgeInserter::Always, configure)
    }

    /// Add an Event edge from this node to a target by name/path with optional validator.
    pub fn on<E>(&mut self, to: impl AsRef<str>, configure: impl FnOnce(&mut EventEdgeCommands<E>)) -> &mut Self
    where
        E: crate::registration::RegisteredTransitionEvent + crate::TransitionEvent + 'static,
    {
        // Start with defaults and let the user configure
        let mut cfg = EventEdgeCommands::<E> { base: EdgeCommands { name: None, kind: EdgeKind::External, after: None, extras: Vec::new() }, validator: None };
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
    pub fn on_from<E>(&mut self, to: impl AsRef<str>, v: impl Into<<E as crate::TransitionEvent>::Validator> + Into<String> + Clone) -> &mut Self
    where
        E: crate::registration::RegisteredTransitionEvent + crate::TransitionEvent + 'static,
    {
        let val = v.clone();
        self.on::<E>(to, move |e| { e.nv(val); });
        self
    }

    fn add_edge_generic(
        &mut self,
        to: impl AsRef<str>,
        inserter: EdgeInserter,
        configure: impl FnOnce(&mut EdgeCommands),
    ) -> &mut Self {
        let mut cfg = EdgeCommands { name: None, kind: EdgeKind::External, after: None, extras: Vec::new() };
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

fn display_path(path: &[String]) -> String {
    if path.is_empty() { "/".to_string() } else { format!("/{}", path.join("/")) }
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

    // Build helpful suggestions for diagnostics
    let siblings: Vec<String> = all_paths
        .iter()
        .filter_map(|p| {
            if p.len() == parent_path.len() + 1 && &p[..parent_path.len()] == parent_path {
                p.last().cloned()
            } else { None }
        })
        .collect();

    let children: Vec<String> = all_paths
        .iter()
        .filter_map(|p| {
            if p.len() == source_path.len() + 1 && &p[..source_path.len()] == source_path {
                p.last().cloned()
            } else { None }
        })
        .collect();

    let top_level: Vec<String> = all_paths
        .iter()
        .filter(|p| p.len() == 1)
        .filter_map(|p| p.last().cloned())
        .collect();

    panic!(
        "Could not resolve target '{}' from source {}.\n  Siblings of {}: [{}]\n  Children of {}: [{}]\n  Top-level nodes: [{}]\n  Tip: Use absolute paths starting with '/' to disambiguate.",
        raw,
        display_path(source_path),
        display_path(parent_path),
        siblings.join(", "),
        display_path(source_path),
        children.join(", "),
        top_level.join(", ")
    );
}


// --------------- command-like customizers ---------------
// Both StateCommands and EdgeCommands offer commands-like ergonomics.

