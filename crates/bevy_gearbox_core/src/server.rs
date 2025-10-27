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
            .register_type::<TransitionFeed>();

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

        // Register custom RPC endpoints for saving graphs and sidecars
        register_editor_file_rpcs(app);

        // Register discovery watcher (+watch SSE)
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
#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
pub struct TransitionFeed {
    pub next_seq: u64,
    pub ring: Vec<TransitionEdge>,
    pub capacity: u16,
}

// =========================
// Tracker updaters
// =========================
#[cfg(feature = "editor")]
fn sync_active_tracker_on_state_changes(
    q_changed: Query<(Entity, &StateMachine), Changed<StateMachine>>,
    mut commands: Commands,
){
    for (root, sm) in q_changed.iter() {
        let mut active: Vec<Entity> = Vec::with_capacity(sm.active.len());
        let mut leaves: Vec<Entity> = Vec::with_capacity(sm.active_leaves.len());
        active.extend(sm.active.iter().copied());
        leaves.extend(sm.active_leaves.iter().copied());

        // Update or insert tracker
        commands.entity(root).insert(ActiveTracker { active, leaves });
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
    } else {
        let mut feed = TransitionFeed { next_seq: 1, ring: Vec::new(), capacity: 64 };
        feed.ring.push(TransitionEdge { seq: 0, edge });
        commands.entity(machine).insert(feed);
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
struct MachineWatchParams { entity: Entity }

// =========================
// Watch (+watch) handlers
// =========================
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
            "machine": e.index() as u64,
            "name": name.map(|n| n.to_string()),
            "id_path": id.map(|p| p.0.clone()),
        }));
    }
    for e in removed_sm.read() {
        events.push(serde_json::json!({
            "kind": "machine_removed",
            "machine": e.index() as u64,
        }));
    }
    for (e, name) in q_name_changed.iter() {
        events.push(serde_json::json!({
            "kind": "machine_renamed",
            "machine": e.index() as u64,
            "name": name.to_string(),
        }));
    }
    for (e, id) in q_id_changed.iter() {
        events.push(serde_json::json!({
            "kind": "machine_id_set",
            "machine": e.index() as u64,
            "id_path": id.0.clone(),
        }));
    }
    if events.is_empty() { Ok(None) } else { Ok(Some(serde_json::json!({ "events": events }))) }
}

#[cfg(feature = "editor")]
fn register_editor_watch_rpcs(app: &mut App) {
    if !app.world().contains_resource::<RemoteMethods>() { return; }
    let world = app.main_mut().world_mut();
    let discovery_watch = world.register_system(discovery_watch_handler);
    let mut methods = world.resource_mut::<RemoteMethods>();
    methods.insert("editor.discovery+watch", RemoteMethodSystemId::Watching(discovery_watch));
}

#[cfg(not(feature = "editor"))]
fn register_editor_watch_rpcs(_app: &mut App) {}

