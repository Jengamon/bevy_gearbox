use bevy::prelude::*;
use std::net::SocketAddr;
#[cfg(feature = "editor")]
use bevy::scene::{DynamicScene, DynamicSceneBuilder};
#[cfg(feature = "editor")]
use bevy::remote::{BrpError, BrpResult, RemoteMethodSystemId, RemoteMethods, error_codes};
#[cfg(feature = "editor")]
use serde::Deserialize;
#[cfg(feature = "editor")]
use serde_json::Value;
#[cfg(feature = "editor")]
use std::path::PathBuf;
#[cfg(feature = "editor")]
use std::collections::HashMap;

/*
================================================================================
Editor <-> App stack overview (farm-to-table flows)
================================================================================

This file implements the app-side of the JSON-RPC (+watch SSE) surface that the
editor consumes. Below is a practical walkthrough of how data and commands flow
end-to-end for the main editor features. The client/editor pieces referenced
live in crates/bevy_gearbox_editor (not in this crate), but are summarized here
so you can reason about the whole stack from user input to UI update.

-------------------------------------------------------------------------------
1) State machine polling (listing machines for the Explorer)
-------------------------------------------------------------------------------
User → UI
- User clicks connect. The editor triggers
  a RefreshIndexRequested event.

Editor (client) → Network
- The observer writes NetCommand::Refresh.
- net.rs handles it by calling rpcs::list_state_machines() which issues a
  JSON-RPC call (world.query with a filter for StateMachine) against this app.
- The query returns entity ids and optional Name components.
- net.rs emits NetEvent::RefreshResult(Vec<MachineSummary>).

Editor (UI update)
- plugin.rs poll_network receives RefreshResult and updates the editor’s index
  (UiState.machines and EditorStore.index). The Explorer panel renders the list.

App (server)
- This file does not implement the polling RPC directly; it relies on Bevy
  Remote (BRP) world.query. We do ensure the StateMachine type is registered so
  it can be filtered/queried.

-------------------------------------------------------------------------------
2) State machine loading (opening a machine onto the canvas)
-------------------------------------------------------------------------------
User → UI
- User clicks a machine in the Explorer. The editor triggers OpenRequested.

Editor (client) → Network
- The observer sets the newly selected doc as active, ensures an empty document
  is present in Workspace, and writes NetCommand::FetchGraph for that entity.
- net.rs calls rpcs::fetch_machine_graph_model(), which:
  - Performs structured world.get_components calls via JSON-RPC to read the
    root and all descendant states, plus transitions, and constructs a
    StateMachineGraph (nodes, edges, adjacency, components).
- net.rs emits NetEvent::GraphResult { id, graph }.

Editor (projection + sidecar)
- plugin.rs poll_network stashes the snapshot briefly. In
  sync_snapshots_to_workspace the snapshot is projected once into the active
  Workspace document via project_graph_into_doc (computes initial layout,
  pill positions, transform parents/children, etc.).
- If a sidecar (.sm.ron) is available (either fetched over RPC using the
  root’s StateMachineId pointer or discovered on disk), it is parsed and
  applied (positions, view data) to the document.

UI render
- The canvas draws from Workspace.docs[active].graph + views; once projected,
  the graph appears immediately. Subsequent draws only animate highlights.

App (server)
- This file doesn’t have a dedicated “load graph” RPC; the client constructs a
  graph by reading components with world.get_components. We do register all the
  relevant types/components so they can be read.

-------------------------------------------------------------------------------
3) Active/transition live updates (stateless +watch streams)
-------------------------------------------------------------------------------
High-level
- Live updates are delivered over BRP “+watch” SSE endpoints defined here:
  - editor.discovery+watch → creation/rename/remove events for machines
  - editor.machine+watch → per-machine deltas for active changes and fired
    transition edges
- The watches are stateless: the editor passes last_active_seq /
  last_transition_seq cursors so the server can emit only newer entries.

User → UI
- After a machine loads, the editor subscribes the app-side feeds and starts a
  watch for the active document.

Editor (client) → Network
- The editor writes NetCommand::Subscribe { id } to gate feeds server-side
  (see register_editor_subscription_rpcs below). Then it requests
  NetCommand::StartMachineWatch { id, cursors }.
- A single Tokio “Watch Manager” task runs in the editor. It receives start/
  stop requests, owns the long-lived SSE connections, and pushes parsed events
  back to Bevy via a channel. This avoids starving Bevy’s IoTaskPool.
- Incoming batches from the manager are forwarded as NetEvent::MachineDeltas.

Editor (UI update)
- poll_network coalesces cursors and stashes batches. In
  sync_snapshots_to_workspace the editor applies:
  - ActiveChanged: replaces the document’s active set (flashes/fades nodes)
  - TransitionEdge: flashes the edge pill
  The canvas then reflects these changes on the next frame.
- Discovery watch (editor.discovery+watch) similarly feeds index updates.

App (server)
- Subscriptions: editor.machine_subscribe/editor.machine_unsubscribe bump a
  per-machine subscriber count. When the first subscriber arrives, we insert
  TransitionFeed and ActiveChangedFeed components and seed an initial active
  snapshot. When the last subscriber leaves, we remove these feeds.
- Watch endpoints: discovery_watch_handler and machine_watch_handler serialize
  recent events into JSON (using entity.to_bits() to identify entities) and
  BRP turns these into SSE lines. Because the editor provides cursors, the
  server can emit only new entries, keeping streams small.

-------------------------------------------------------------------------------
4) Command RPCs (e.g., rename)
-------------------------------------------------------------------------------
User → UI
- User invokes a command in the editor (e.g., “Rename”). The UI triggers an
  observer/event which then writes a NetCommand to the network layer.

Editor (client) → Network
- net.rs issues a JSON-RPC call for the command. For rename this would likely
  be either a generic world.set_component (setting Name on the root entity) or
  a custom editor.* RPC if there is server-side validation.
- On success, the server mutates the world state. If the mutation changes any
  feed-relevant data, the next discovery +watch batch will include a
  machine_renamed event (or similar), and the per-machine +watch will continue
  with updated state as needed.
- net.rs emits NetEvent::Select/Save/… result for immediate feedback; the
  watches deliver the eventual state convergence.

Editor (UI update)
- poll_network handles the immediate result (errors surfaced to UI). The
  discovery +watch delivers the rename event which updates the index and the
  document header (since the root Name is part of the projected graph).

App (server)
- This file demonstrates how file save commands are implemented as editor.*
  RPCs (save_graph, save_sidecar, set_state_machine_id). A future rename RPC
  would follow the same pattern: parse params, mutate world, return a JSON
  result and let +watch streams disseminate the change.

-------------------------------------------------------------------------------
Notes and future server-side multiplex
-------------------------------------------------------------------------------
- Today each watched machine uses one HTTP SSE stream. This is fine for
  dozens of machines and typical editor usage (often only the active doc is
  watched). If you need to watch many (50–100+) concurrently, consider a
  server-side multiplexed watch that aggregates multiple machines into a single
  SSE stream. The client’s Watch Manager can continue to fan-out events to the
  appropriate documents, but you reduce total connections.

-------------------------------------------------------------------------------
Runtime and threading choices (Bevy IoTaskPool vs Tokio)
-------------------------------------------------------------------------------
Where we run code and why:

- Bevy IoTaskPool (short-lived work)
  - Used for one-shot network RPCs that return quickly (e.g., list machines,
    fetch graph snapshot, fetch one active snapshot, save graph/sidecar).
  - We spawn a Task<StampedEvent> and drain it in collect_task_results during
    NetSet::Drain. That integrates cleanly with Bevy’s schedule and frame loop.
  - These tasks may internally call into the Tokio runtime (rt.block_on) to
    perform the HTTP request, but the task lifetime is bounded to a single
    response and then the thread returns to the pool.
  - Do NOT keep long-lived or idle waits here; they can starve the pool.

- Tokio runtime (long-lived work)
  - Used for sustained SSE “+watch” streams via a single Watch Manager task.
  - Reasons:
    - Long-lived, mostly-idle tasks (waiting on bytes) should not occupy Bevy’s
      IoTaskPool threads. Tokio’s cooperative scheduler handles many such tasks
      without starving other work.
    - Clean cancellation: per-watch JoinHandle aborts and central supervision
      (start/stop/reconnect/backoff) are straightforward in a single async task
      with channels.
    - Backpressure and coalescing: events are sent on an mpsc channel back to
      Bevy; Bevy drains at its cadence.
  - Only the active (or MRU) machine(s) are typically watched; Unsubscribe
    aborts the per-machine task immediately.

Rule of thumb:
- Short, bounded operations that produce one result → IoTaskPool task.
- Long-lived streams, reconnection loops, or anything that could block/wait →
  Tokio (Watch Manager), then bridge back into Bevy via a channel and
  translate into NetEvents in NetSet::Drain.

================================================================================
*/

#[cfg(feature = "editor")]
use crate::{StateMachine};

/// Configuration plugin for enabling the Bevy Remote (BRP) server from core.
///
/// Defaults:
/// - bind_address: 127.0.0.1:15703
/// - headers: empty
#[cfg(feature = "editor")]
pub struct RemoteServerPlugin {
    pub headers: Vec<(String, String)>,
    pub bind_address: SocketAddr,
}

#[cfg(feature = "editor")]
impl Default for RemoteServerPlugin {
    fn default() -> Self {
        Self {
            headers: Vec::new(),
            bind_address: "127.0.0.1:15703".parse().expect("valid default bind address"),
        }
    }
}

#[cfg(feature = "editor")]
impl RemoteServerPlugin {
    pub fn new() -> Self { Self::default() }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }

    pub fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = headers;
        self
    }

    pub fn with_bind_address(mut self, addr: SocketAddr) -> Self {
        self.bind_address = addr;
        self
    }
}

#[cfg(feature = "editor")]
impl Plugin for RemoteServerPlugin {
    fn build(&self, app: &mut App) {
        // Register commonly-inspected types
        app.register_type::<Name>();

        // Register Bevy Gearbox types commonly interacted with by the editor
        app
            .register_type::<crate::SubstateOf>()
            .register_type::<crate::Substates>()
            .register_type::<crate::StateMachine>()
            .register_type::<crate::InitialState>()
            .register_type::<crate::Parallel>()
            .register_type::<crate::transitions::Source>()
            .register_type::<crate::transitions::Target>()
            .register_type::<crate::transitions::EdgeKind>()
            .register_type::<crate::transitions::AlwaysEdge>();

        // Editor transport helpers: reflectable trackers the editor can watch via BRP
        app
            .register_type::<ActiveTracker>()
            .register_type::<TransitionEdge>()
            .register_type::<TransitionFeed>()
            .register_type::<ActiveChangedFeed>()
            .register_type::<ActiveChangedEntry>();

        // Configure HTTP transport for BRP
        let mut http = {
            let addr = self.bind_address;
            bevy::remote::http::RemoteHttpPlugin::default()
                .with_address(addr.ip())
                .with_port(addr.port())
        };

        if !self.headers.is_empty() {
            let mut headers = bevy::remote::http::Headers::new();
            for (k, v) in &self.headers {
                headers = headers.insert(k.clone(), v.clone());
            }
            http = http.with_headers(headers);
        }

        app.add_plugins(bevy::remote::RemotePlugin::default());
        app.add_plugins(http);

        // Systems/observers to keep trackers updated
        app.add_systems(Update, sync_active_tracker_on_state_changes);
        app.add_observer(record_transition_on_actions);
        // Event-driven active feed updates using component add/remove triggers
        app.add_observer(on_active_added);
        app.add_observer(on_active_removed);

        // Register custom RPC endpoints for saving graphs and sidecars
        register_editor_file_rpcs(app);

        // Register discovery watcher (+watch SSE) and machine +watch
        register_editor_subscription_rpcs(app);
        register_editor_watch_rpcs(app);
    }
}


// =========================
// Editor-facing tracker types
// =========================
#[cfg(feature = "editor")]
#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
pub struct ActiveTracker {
    pub active: Vec<Entity>,
    pub leaves: Vec<Entity>,
}

#[cfg(feature = "editor")]
#[derive(Reflect, Clone)]
pub struct TransitionEdge { pub seq: u64, pub edge: Entity }

#[cfg(feature = "editor")]
#[derive(Reflect, Clone)]
pub struct ActiveChangedEntry { pub seq: u64, pub active: Vec<Entity>, pub leaves: Vec<Entity> }

#[cfg(feature = "editor")]
#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
pub struct TransitionFeed {
    pub next_seq: u64,
    pub ring: Vec<TransitionEdge>,
    pub capacity: u16,
}

#[cfg(feature = "editor")]
#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
pub struct ActiveChangedFeed {
    pub next_seq: u64,
    pub ring: Vec<ActiveChangedEntry>,
    pub capacity: u16,
}

// (Removed) Internal state for machine +watch de-duplication. The +watch endpoint
// is now stateless and relies on client-provided cursors (last seqs) instead.

// =========================
// Tracker updaters
// =========================
#[cfg(feature = "editor")]
fn sync_active_tracker_on_state_changes(
    q_changed: Query<(Entity, &StateMachine), Changed<StateMachine>>,
    mut commands: Commands,
    mut q_active_feed: Query<&mut ActiveChangedFeed>,
){
    for (root, sm) in q_changed.iter() {
        let mut active: Vec<Entity> = Vec::with_capacity(sm.active.len());
        let mut leaves: Vec<Entity> = Vec::with_capacity(sm.active_leaves.len());
        active.extend(sm.active.iter().copied());
        leaves.extend(sm.active_leaves.iter().copied());

        // Update or insert tracker
        // Clone for feed before moving into the component
        let active_for_feed = active.clone();
        let leaves_for_feed = leaves.clone();
        commands.entity(root).insert(ActiveTracker { active, leaves });

        // Also append to ActiveChangedFeed ring for reliable delivery via +watch
        if let Ok(mut feed) = q_active_feed.get_mut(root) {
            let seq = feed.next_seq;
            feed.next_seq = feed.next_seq.saturating_add(1);
            feed.ring.push(ActiveChangedEntry { seq, active: active_for_feed.clone(), leaves: leaves_for_feed.clone() });
            let cap = feed.capacity.max(1) as usize;
            if feed.ring.len() > cap { let _ = feed.ring.remove(0); }
        }
    }
}

#[cfg(feature = "editor")]
fn record_transition_on_actions(
    transition_actions: On<crate::TransitionActions>,
    q_source: Query<&crate::transitions::Source>,
    q_substate_of: Query<&crate::SubstateOf>,
    mut q_feed: Query<&mut TransitionFeed>,
    mut commands: Commands,
){
    let edge = transition_actions.target;
    let Ok(crate::transitions::Source(source)) = q_source.get(edge) else { return; };
    let machine = q_substate_of.root_ancestor(*source);
    if let Ok(mut feed) = q_feed.get_mut(machine) {
        let seq = feed.next_seq;
        feed.next_seq = feed.next_seq.saturating_add(1);
        feed.ring.push(TransitionEdge { seq, edge });
        let cap = feed.capacity.max(1) as usize;
        if feed.ring.len() > cap { let _ = feed.ring.remove(0); }
    }
}

// Append to the ActiveChangedFeed when Active is added to a state
#[cfg(feature = "editor")]
fn on_active_added(
    add: On<Add, crate::active::Active>,
    q_substate_of: Query<&crate::SubstateOf>,
    q_sm: Query<&crate::StateMachine>,
    mut q_feed: Query<&mut ActiveChangedFeed>,
    mut commands: Commands,
){
    let state = add.event().entity;
    let root = q_substate_of.root_ancestor(state);
    // Snapshot current active/leaves from authoritative StateMachine
    if let Ok(sm) = q_sm.get(root) {
        let mut active: Vec<Entity> = sm.active.iter().copied().collect();
        let mut leaves: Vec<Entity> = sm.active_leaves.iter().copied().collect();
        // Keep stable-ish order
        active.sort_by_key(|e| e.index());
        leaves.sort_by_key(|e| e.index());
        if let Ok(mut feed) = q_feed.get_mut(root) {
            let seq = feed.next_seq;
            feed.next_seq = feed.next_seq.saturating_add(1);
            feed.ring.push(ActiveChangedEntry { seq, active, leaves });
            let cap = feed.capacity.max(1) as usize;
            if feed.ring.len() > cap { let _ = feed.ring.remove(0); }
        }
    }
}

// Append to the ActiveChangedFeed when Active is removed from a state
#[cfg(feature = "editor")]
fn on_active_removed(
    rem: On<Remove, crate::active::Active>,
    q_substate_of: Query<&crate::SubstateOf>,
    q_sm: Query<&crate::StateMachine>,
    mut q_feed: Query<&mut ActiveChangedFeed>,
    mut commands: Commands,
){
    let state = rem.event().entity;
    let root = q_substate_of.root_ancestor(state);
    if let Ok(sm) = q_sm.get(root) {
        let mut active: Vec<Entity> = sm.active.iter().copied().collect();
        let mut leaves: Vec<Entity> = sm.active_leaves.iter().copied().collect();
        active.sort_by_key(|e| e.index());
        leaves.sort_by_key(|e| e.index());
        if let Ok(mut feed) = q_feed.get_mut(root) {
            let seq = feed.next_seq;
            feed.next_seq = feed.next_seq.saturating_add(1);
            feed.ring.push(ActiveChangedEntry { seq, active, leaves });
            let cap = feed.capacity.max(1) as usize;
            if feed.ring.len() > cap { let _ = feed.ring.remove(0); }
        }
    }
}
// =========================
// Graph save RPCs (server-side)
// =========================
#[cfg(feature = "editor")]
fn collect_state_machine_entities(world: &World, root: Entity) -> Vec<Entity> {
    use crate::transitions::Transitions as EdgeTransitions;
    let mut entities: Vec<Entity> = Vec::new();
    let mut stack: Vec<Entity> = vec![root];
    while let Some(e) = stack.pop() {
        if !world.entities().contains(e) { continue; }
        if !entities.contains(&e) { entities.push(e); }
        if let Some(transitions) = world.get::<EdgeTransitions>(e) {
            for &edge in transitions.into_iter() {
                if world.entities().contains(edge) && !entities.contains(&edge) { entities.push(edge); }
            }
        }
        if let Some(children) = world.get::<crate::Substates>(e) {
            for &child in children.into_iter() { stack.push(child); }
        }
    }
    entities
}

#[cfg(feature = "editor")]
fn build_scene_from_root(world: &mut World, root: Entity) -> DynamicScene {
    let entities = collect_state_machine_entities(world, root);
    let mut builder = DynamicSceneBuilder::from_world(world);
    // Configure filter first, then extract target entities
    builder = builder.allow_all();
    builder = builder.deny_component::<ActiveTracker>();
    builder = builder.deny_component::<TransitionFeed>();
    builder = builder.extract_entities(entities.into_iter());
    builder.build()
}

#[cfg(feature = "editor")]
fn serialize_scene(world: &World, scene: &DynamicScene) -> Result<String, String> {
    let reg = world.resource::<AppTypeRegistry>();
    let reg = reg.read();
    scene.serialize(&reg).map_err(|e| format!("serialize scene: {e}"))
}

#[cfg(feature = "editor")]
fn atomic_write(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::fs;
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.flush()?;
    }
    #[cfg(target_os = "windows")]
    {
        fs::rename(&tmp, path)?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::fs::remove_file(path);
        std::fs::rename(&tmp, path)
    }
}

#[cfg(feature = "editor")]
fn save_graph_to_file(world: &mut World, root: Entity, path: &std::path::Path) -> Result<(), String> {
    let scene = build_scene_from_root(world, root);
    let ron = serialize_scene(world, &scene)?;
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| format!("mkdirs: {e}"))?; }
    atomic_write(path, &ron).map_err(|e| format!("write: {e}"))
}

#[cfg(feature = "editor")]
#[derive(Deserialize)]
struct SaveGraphParams { entity: Entity, path: String }
#[cfg(feature = "editor")]
#[derive(Deserialize)]
struct SetStateMachineId { entity: Entity, path: String }

#[cfg(feature = "editor")]
fn set_state_machine_id_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: SetStateMachineId = parse_params(params)?;
    if !world.entities().contains(p.entity) {
        return Err(BrpError { code: error_codes::INVALID_PARAMS, message: "invalid entity".to_string(), data: None });
    }
    let mut e = world.entity_mut(p.entity);
    e.insert(crate::StateMachineId(p.path));
    Ok(serde_json::json!({"ok": true}))
}

#[cfg(feature = "editor")]
fn parse_params<T: for<'de> Deserialize<'de>>(params: Option<Value>) -> Result<T, BrpError> {
    serde_json::from_value(params.unwrap_or(Value::Null)).map_err(|e| BrpError {
        code: error_codes::INVALID_PARAMS,
        message: format!("invalid params: {e}"),
        data: None,
    })
}

#[cfg(feature = "editor")]
fn save_graph_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: SaveGraphParams = parse_params(params)?;
    let mut path = PathBuf::from(p.path);
    if !path.is_absolute() { path = PathBuf::from("assets").join(path); }
    save_graph_to_file(world, p.entity, &path)
        .map(|_| serde_json::json!({"ok": true}))
        .map_err(|msg| BrpError { code: error_codes::INTERNAL_ERROR, message: msg, data: None })
}

#[cfg(feature = "editor")]
#[derive(Deserialize)]
struct SaveSidecarParams { path: String, contents: String }

#[cfg(feature = "editor")]
fn save_sidecar_handler(In(params): In<Option<Value>>, _world: &mut World) -> BrpResult {
    let p: SaveSidecarParams = parse_params(params)?;
    let mut path = PathBuf::from(p.path);
    if !path.is_absolute() { path = PathBuf::from("assets").join(path); }
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| BrpError { code: error_codes::INTERNAL_ERROR, message: format!("mkdirs: {e}"), data: None })?; }
    atomic_write(&path, &p.contents).map_err(|e| BrpError { code: error_codes::INTERNAL_ERROR, message: format!("write: {e}"), data: None })?;
    Ok(serde_json::json!({"ok": true}))
}

#[cfg(feature = "editor")]
#[derive(Deserialize)]
struct LoadSidecarParams { path: String }

#[cfg(feature = "editor")]
fn load_sidecar_handler(In(params): In<Option<Value>>, _world: &mut World) -> BrpResult {
    let p: LoadSidecarParams = parse_params(params)?;
    let mut path = PathBuf::from(p.path);
    if !path.is_absolute() { path = PathBuf::from("assets").join(path); }
    let txt = std::fs::read_to_string(&path).map_err(|e| BrpError { code: error_codes::INTERNAL_ERROR, message: format!("read: {e}"), data: None })?;
    Ok(serde_json::json!({"text": txt}))
}

#[cfg(feature = "editor")]
#[derive(Deserialize)]
struct FindByFingerprintParams { fp: String }

#[cfg(feature = "editor")]
fn find_sidecar_by_fingerprint_handler(In(params): In<Option<Value>>, _world: &mut World) -> BrpResult {
    let p: FindByFingerprintParams = parse_params(params)?;
    // Simple scan: current dir and ./assets for *.sm.ron
    let mut roots: Vec<std::path::PathBuf> = vec![std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))];
    let assets = std::path::PathBuf::from("assets");
    if assets.exists() { roots.push(assets); }
    for root in roots.into_iter() {
        let walker = walkdir::WalkDir::new(&root).max_depth(6);
        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let path = entry.path().to_path_buf();
            if !path.is_file() { continue; }
            if let Some(ext) = path.extension().and_then(|s| s.to_str()) { if ext != "ron" { continue; } }
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) { if !name.ends_with(".sm.ron") { continue; } }
            if let Ok(txt) = std::fs::read_to_string(&path) {
                // Lightweight parse: look for graph_fingerprint: Some("...")
                if txt.contains(&p.fp) {
                    return Ok(serde_json::json!({"text": txt}));
                }
            }
        }
    }
    Ok(serde_json::json!({"text": null}))
}

#[cfg(feature = "editor")]
fn register_editor_file_rpcs(app: &mut App) {
    if !app.world().contains_resource::<RemoteMethods>() { return; }
    let world = app.main_mut().world_mut();
    let save_id = world.register_system(save_graph_handler);
    let save_sc_id = world.register_system(save_sidecar_handler);
    let load_sc_id = world.register_system(load_sidecar_handler);
    let find_sc_id = world.register_system(find_sidecar_by_fingerprint_handler);
    let set_state_machine_id = world.register_system(set_state_machine_id_handler);
    let mut methods = world.resource_mut::<RemoteMethods>();
    methods.insert("editor.save_graph", RemoteMethodSystemId::Instant(save_id));
    methods.insert("editor.save_sidecar", RemoteMethodSystemId::Instant(save_sc_id));
    methods.insert("editor.load_sidecar", RemoteMethodSystemId::Instant(load_sc_id));
    methods.insert("editor.find_sidecar_by_fingerprint", RemoteMethodSystemId::Instant(find_sc_id));
    methods.insert("editor.set_state_machine_id", RemoteMethodSystemId::Instant(set_state_machine_id));
}

#[cfg(not(feature = "editor"))]
fn register_editor_file_rpcs(_app: &mut App) {}



#[cfg(feature = "editor")]
#[derive(Deserialize)]
struct MachineWatchParams {
    entity: Entity,
    #[serde(default)]
    last_active_seq: u64,
    #[serde(default)]
    last_transition_seq: u64,
}

// =========================
// Watch (+watch) handlers
// =========================
#[cfg(feature = "editor")]
fn entity_to_bits(e: Entity) -> u64 {
    e.to_bits()
}

#[cfg(feature = "editor")]
fn discovery_watch_handler(
    _in: In<Option<Value>>,
    q_added_sm: Query<(Entity, Option<&Name>, Option<&crate::StateMachineId>), Added<StateMachine>>,
    mut removed_sm: RemovedComponents<StateMachine>,
    q_name_changed: Query<(Entity, &Name), (With<StateMachine>, Changed<Name>)>,
    q_id_changed: Query<(Entity, &crate::StateMachineId), (With<StateMachine>, Changed<crate::StateMachineId>)>,
) -> bevy::remote::BrpResult<Option<Value>> {
    let mut events: Vec<Value> = Vec::new();
    for (e, name, id) in q_added_sm.iter() {
        events.push(serde_json::json!({
            "kind": "machine_created",
            "machine": entity_to_bits(e),
            "name": name.map(|n| n.to_string()),
            "id_path": id.map(|p| p.0.clone()),
        }));
    }
    for e in removed_sm.read() {
        events.push(serde_json::json!({
            "kind": "machine_removed",
            "machine": entity_to_bits(e),
        }));
    }
    for (e, name) in q_name_changed.iter() {
        events.push(serde_json::json!({
            "kind": "machine_renamed",
            "machine": entity_to_bits(e),
            "name": name.to_string(),
        }));
    }
    for (e, id) in q_id_changed.iter() {
        events.push(serde_json::json!({
            "kind": "machine_id_set",
            "machine": entity_to_bits(e),
            "id_path": id.0.clone(),
        }));
    }
    if events.is_empty() { Ok(None) } else { Ok(Some(serde_json::json!({ "events": events }))) }
}

#[cfg(feature = "editor")]
fn machine_watch_handler(
    In(params): In<Option<Value>>,
    q_active_feed: Query<&ActiveChangedFeed>,
    q_feed: Query<&TransitionFeed>,
    q_source: Query<&crate::transitions::Source>,
    q_target: Query<&crate::transitions::Target>,
) -> bevy::remote::BrpResult<Option<Value>> {
    // Expect a target machine entity
    let p: MachineWatchParams = parse_params(params)?;
    let mut events: Vec<Value> = Vec::new();

    // Active changes from ring: emit entries with seq greater than provided cursor
    if let Ok(feed) = q_active_feed.get(p.entity) {
        let last = p.last_active_seq;
        for item in feed.ring.iter() {
            if item.seq <= last { continue; }
            let active: Vec<u64> = item.active.iter().map(|e| entity_to_bits(*e)).collect();
            let leaves: Vec<u64> = item.leaves.iter().map(|e| entity_to_bits(*e)).collect();
            events.push(serde_json::json!({
                "seq": item.seq,
                "kind": "active_changed",
                "machine": entity_to_bits(p.entity),
                "active": active,
                "leaves": leaves,
            }));
        }
    }

    // Transition feed deltas: emit entries with seq greater than provided cursor
    if let Ok(feed) = q_feed.get(p.entity) {
        let last = p.last_transition_seq;
        for item in feed.ring.iter() {
            if item.seq <= last { continue; }
            let edge = item.edge;
            let source_u64 = q_source.get(edge).ok().map(|s| entity_to_bits(s.0));
            let target_u64 = q_target.get(edge).ok().map(|t| entity_to_bits(t.0));
            events.push(serde_json::json!({
                "seq": item.seq,
                "kind": "transition_edge",
                "machine": entity_to_bits(p.entity),
                "edge": entity_to_bits(edge),
                "source": source_u64,
                "target": target_u64,
            }));
        }
    }

    if events.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::json!({ "events": events })))
    }
}

#[cfg(feature = "editor")]
fn register_editor_watch_rpcs(app: &mut App) {
    if !app.world().contains_resource::<RemoteMethods>() { return; }
    let world = app.main_mut().world_mut();
    let discovery_watch = world.register_system(discovery_watch_handler);
    let machine_watch = world.register_system(machine_watch_handler);
    let mut methods = world.resource_mut::<RemoteMethods>();
    methods.insert("editor.discovery+watch", RemoteMethodSystemId::Watching(discovery_watch));
    methods.insert("editor.machine+watch", RemoteMethodSystemId::Watching(machine_watch));
}

#[cfg(not(feature = "editor"))]
fn register_editor_watch_rpcs(_app: &mut App) {}

// =========================
// Subscriptions (server-side gating of feeds)
// =========================
#[cfg(feature = "editor")]
#[derive(Resource, Default)]
struct Subscriptions { counts: HashMap<Entity, u32> }

#[cfg(feature = "editor")]
#[derive(Deserialize)]
struct SubscribeParams { entity: Entity }

#[cfg(feature = "editor")]
fn subscribe_machine_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: SubscribeParams = parse_params(params)?;
    if !world.entities().contains(p.entity) {
        return Err(BrpError { code: error_codes::INVALID_PARAMS, message: "invalid entity".to_string(), data: None });
    }
    // Bump count and create feeds if first subscriber
    let mut counts = world.resource_mut::<Subscriptions>();
    let c = counts.counts.entry(p.entity).or_insert(0);
    *c = c.saturating_add(1);
    if *c == 1 {
        // Insert feeds and seed active snapshot
        let mut insert_transition = true;
        let mut insert_active = true;
        if world.get::<TransitionFeed>(p.entity).is_some() { insert_transition = false; }
        if world.get::<ActiveChangedFeed>(p.entity).is_some() { insert_active = false; }
        if insert_transition {
            world.entity_mut(p.entity).insert(TransitionFeed { next_seq: 1, ring: Vec::new(), capacity: 64 });
        }
        if insert_active {
            let mut feed = ActiveChangedFeed { next_seq: 2, ring: Vec::new(), capacity: 64 };
            if let Some(sm) = world.get::<StateMachine>(p.entity) {
                let mut active: Vec<Entity> = sm.active.iter().copied().collect();
                let mut leaves: Vec<Entity> = sm.active_leaves.iter().copied().collect();
                active.sort_by_key(|e| e.index());
                leaves.sort_by_key(|e| e.index());
                feed.ring.push(ActiveChangedEntry { seq: 1, active, leaves });
            }
            world.entity_mut(p.entity).insert(feed);
        }
    }
    Ok(serde_json::json!({"ok": true}))
}

#[cfg(feature = "editor")]
fn unsubscribe_machine_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: SubscribeParams = parse_params(params)?;
    if !world.entities().contains(p.entity) {
        return Err(BrpError { code: error_codes::INVALID_PARAMS, message: "invalid entity".to_string(), data: None });
    }
    let mut counts = world.resource_mut::<Subscriptions>();
    if let Some(c) = counts.counts.get_mut(&p.entity) {
        *c = c.saturating_sub(1);
        if *c == 0 {
            // Remove feeds entirely when last subscriber leaves
            counts.counts.remove(&p.entity);
            let mut e = world.entity_mut(p.entity);
            if e.get::<TransitionFeed>().is_some() { e.remove::<TransitionFeed>(); }
            if e.get::<ActiveChangedFeed>().is_some() { e.remove::<ActiveChangedFeed>(); }
        }
    }
    Ok(serde_json::json!({"ok": true}))
}

#[cfg(feature = "editor")]
fn register_editor_subscription_rpcs(app: &mut App) {
    if !app.world().contains_resource::<RemoteMethods>() { return; }
    if !app.world().contains_resource::<Subscriptions>() { app.insert_resource(Subscriptions::default()); }
    let world = app.main_mut().world_mut();
    let sub_id = world.register_system(subscribe_machine_handler);
    let unsub_id = world.register_system(unsubscribe_machine_handler);
    let mut methods = world.resource_mut::<RemoteMethods>();
    methods.insert("editor.machine_subscribe", RemoteMethodSystemId::Instant(sub_id));
    methods.insert("editor.machine_unsubscribe", RemoteMethodSystemId::Instant(unsub_id));
}

#[cfg(not(feature = "editor"))]
fn register_editor_subscription_rpcs(_app: &mut App) {}

