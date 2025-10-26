use bevy::prelude::*;
use std::net::SocketAddr;
#[cfg(feature = "remote_server")]
use bevy::scene::{DynamicScene, DynamicSceneBuilder, DynamicSceneRoot};
#[cfg(feature = "remote_server")]
use bevy::remote::{BrpError, BrpResult, RemoteMethodSystemId, RemoteMethods, error_codes};
#[cfg(feature = "remote_server")]
use serde::Deserialize;
#[cfg(feature = "remote_server")]
use serde_json::Value;

#[cfg(feature = "remote_server")]
use crate::{StateMachine};

/// Configuration plugin for enabling the Bevy Remote (BRP) server from core.
///
/// Defaults:
/// - bind_address: 127.0.0.1:15703
/// - headers: empty
#[cfg(feature = "remote_server")]
pub struct RemoteServerPlugin {
    pub headers: Vec<(String, String)>,
    pub bind_address: SocketAddr,
}

#[cfg(feature = "remote_server")]
impl Default for RemoteServerPlugin {
    fn default() -> Self {
        Self {
            headers: Vec::new(),
            bind_address: "127.0.0.1:15703".parse().expect("valid default bind address"),
        }
    }
}

#[cfg(feature = "remote_server")]
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

#[cfg(feature = "remote_server")]
impl Plugin for RemoteServerPlugin {
    fn build(&self, app: &mut App) {
        // Register commonly-inspected types
        app.register_type::<Name>();

        // Register Bevy Gearbox types commonly interacted with by the editor
        app
            .register_type::<crate::StateChildOf>()
            .register_type::<crate::StateChildren>()
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

        // Register custom RPC endpoints for saving graphs
        register_editor_file_rpcs(app);
    }
}


// =========================
// Editor-facing tracker types
// =========================
#[cfg(feature = "remote_server")]
#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
pub struct ActiveTracker {
    pub active: Vec<Entity>,
    pub leaves: Vec<Entity>,
}

#[cfg(feature = "remote_server")]
#[derive(Reflect, Clone)]
pub struct TransitionEdge { pub seq: u64, pub edge: Entity }

#[cfg(feature = "remote_server")]
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
#[cfg(feature = "remote_server")]
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

#[cfg(feature = "remote_server")]
fn record_transition_on_actions(
    transition_actions: On<crate::TransitionActions>,
    q_source: Query<&crate::transitions::Source>,
    q_child_of: Query<&crate::StateChildOf>,
    mut q_feed: Query<&mut TransitionFeed>,
    mut commands: Commands,
){
    let edge = transition_actions.target;
    let Ok(crate::transitions::Source(source)) = q_source.get(edge) else { return; };
    let machine = q_child_of.root_ancestor(*source);
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
#[cfg(feature = "remote_server")]
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
        if let Some(children) = world.get::<crate::StateChildren>(e) {
            for &child in children.into_iter() { stack.push(child); }
        }
    }
    entities
}

#[cfg(feature = "remote_server")]
fn build_scene_from_root(world: &mut World, root: Entity) -> DynamicScene {
    let entities = collect_state_machine_entities(world, root);
    let builder = DynamicSceneBuilder::from_world(world);
    builder.extract_entities(entities.into_iter()).allow_all().build()
}

#[cfg(feature = "remote_server")]
fn serialize_scene(world: &World, scene: &DynamicScene) -> Result<String, String> {
    let reg = world.resource::<AppTypeRegistry>();
    let reg = reg.read();
    scene.serialize(&reg).map_err(|e| format!("serialize scene: {e}"))
}

#[cfg(feature = "remote_server")]
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

#[cfg(feature = "remote_server")]
fn save_graph_to_file(world: &mut World, root: Entity, path: &std::path::Path) -> Result<(), String> {
    let scene = build_scene_from_root(world, root);
    let ron = serialize_scene(world, &scene)?;
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| format!("mkdirs: {e}"))?; }
    atomic_write(path, &ron).map_err(|e| format!("write: {e}"))
}

#[cfg(feature = "remote_server")]
#[derive(Deserialize)]
struct SaveGraphParams { entity: Entity, path: String }

#[cfg(feature = "remote_server")]
fn parse_params<T: for<'de> Deserialize<'de>>(params: Option<Value>) -> Result<T, BrpError> {
    serde_json::from_value(params.unwrap_or(Value::Null)).map_err(|e| BrpError {
        code: error_codes::INVALID_PARAMS,
        message: format!("invalid params: {e}"),
        data: None,
    })
}

#[cfg(feature = "remote_server")]
fn save_graph_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: SaveGraphParams = parse_params(params)?;
    let path = std::path::PathBuf::from(p.path);
    save_graph_to_file(world, p.entity, &path)
        .map(|_| serde_json::json!({"ok": true}))
        .map_err(|msg| BrpError { code: error_codes::INTERNAL_ERROR, message: msg, data: None })
}

#[cfg(feature = "remote_server")]
fn register_editor_file_rpcs(app: &mut App) {
    if !app.world().contains_resource::<RemoteMethods>() { return; }
    let world = app.main_mut().world_mut();
    let save_id = world.register_system(save_graph_handler);
    let mut methods = world.resource_mut::<RemoteMethods>();
    methods.insert("editor.save_graph", RemoteMethodSystemId::Instant(save_id));
}

#[cfg(not(feature = "remote_server"))]
fn register_editor_file_rpcs(_app: &mut App) {}



