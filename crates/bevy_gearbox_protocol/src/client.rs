#![cfg(feature = "client")]
use bevy::prelude::*;
use serde_json::{json, Map, Value};
use std::sync::Arc;

use crate::methods::*;
use crate::components as wire;

#[derive(Resource, Clone)]
pub struct ProtocolClientConfig { pub url: String }

#[derive(Debug)]
pub enum ProtocolError {
    Http(reqwest::Error),
    Json(serde_json::Error),
    Rpc { code: i64, message: String, data: Option<Value> },
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::Http(e) => write!(f, "http: {}", e),
            ProtocolError::Json(e) => write!(f, "json: {}", e),
            ProtocolError::Rpc { code, message, .. } => write!(f, "rpc {}: {}", code, message),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<reqwest::Error> for ProtocolError { fn from(e: reqwest::Error) -> Self { ProtocolError::Http(e) } }
impl From<serde_json::Error> for ProtocolError { fn from(e: serde_json::Error) -> Self { ProtocolError::Json(e) } }

#[derive(Resource, Clone)]
pub struct ProtocolClient {
    pub base_url: String,
    http: reqwest::Client,
}

impl ProtocolClient {
    pub fn new(base_url: String) -> Self {
        Self { base_url, http: reqwest::Client::new() }
    }

    pub async fn registry_schema(&self) -> Result<Value, ProtocolError> {
        let v = self.jsonrpc_call(crate::methods::REGISTRY_SCHEMA, None).await?;
        Ok(v.get("result").cloned().unwrap_or(v))
    }

    async fn jsonrpc_call(&self, method: &str, params: Option<Value>) -> Result<Value, ProtocolError> {
        let id = 1u64;
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let resp = self.http.post(&self.base_url).json(&body).send().await?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if let Some(err) = v.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-32000);
            let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error").to_string();
            let data = err.get("data").cloned();
            return Err(ProtocolError::Rpc { code, message, data });
        }
        if !status.is_success() {
            // best-effort message
            return Err(ProtocolError::Rpc { code: status.as_u16() as i64, message: "http status".to_string(), data: Some(v) });
        }
        Ok(v)
    }

    // ===============
    // Thin world.* helpers (direct wrappers)
    // ===============

    pub async fn get_components(
        &self,
        entity: u64,
        components: Option<&[&str]>,
    ) -> Result<Map<String, Value>, ProtocolError> {
        let params = match components {
            Some(list) => json!({"entity": entity, "components": list}),
            None => json!({"entity": entity}),
        };
        let v = self.jsonrpc_call(WORLD_GET_COMPONENTS, Some(params)).await?;
        // Accept common BRP result shapes: {result:{components:{...}}} or {result:{...}} or direct object
        if let Some(obj) = v.get("result").and_then(|r| r.get("components")).and_then(|c| c.as_object()) {
            return Ok(obj.clone());
        }
        if let Some(obj) = v.get("result").and_then(|r| r.as_object()) {
            return Ok(obj.clone());
        }
        if let Some(obj) = v.as_object() {
            return Ok(obj.clone());
        }
        Err(ProtocolError::Rpc { code: -32603, message: "unexpected response for world.get_components".to_string(), data: Some(v) })
    }

    pub async fn insert_components(
        &self,
        entity: u64,
        components: Map<String, Value>,
    ) -> Result<(), ProtocolError> {
        let params = json!({"entity": entity, "components": components});
        let _ = self.jsonrpc_call(WORLD_INSERT_COMPONENTS, Some(params)).await?;
        Ok(())
    }

    pub async fn remove_components(
        &self,
        entity: u64,
        keys: &[&str],
    ) -> Result<(), ProtocolError> {
        let params = json!({"entity": entity, "components": keys});
        let _ = self.jsonrpc_call(WORLD_REMOVE_COMPONENTS, Some(params)).await?;
        Ok(())
    }

	pub async fn spawn(&self, components: Map<String, Value>) -> Result<u64, ProtocolError> {
		let params = json!({"components": components});
		let v = self.jsonrpc_call(WORLD_SPAWN, Some(params)).await?;
		if let Some(id) = v.get("result").and_then(|r| r.get("entity")).and_then(|n| n.as_u64()) { return Ok(id); }
		if let Some(id) = v.get("result").and_then(|n| n.as_u64()) { return Ok(id); }
		if let Some(id) = v.get("entity").and_then(|n| n.as_u64()) { return Ok(id); }
		if let Some(id) = v.as_u64() { return Ok(id); }
		Err(ProtocolError::Rpc { code: -32603, message: "unexpected response for world.spawn".to_string(), data: Some(v) })
	}

	pub async fn despawn(&self, entity: u64) -> Result<(), ProtocolError> {
		let params = json!({"entity": entity});
		let _ = self.jsonrpc_call(WORLD_DESPAWN, Some(params)).await?;
		Ok(())
	}

	pub async fn reset_region(&self, root: u64) -> Result<(), ProtocolError> {
		let params = json!({"root": root});
		let _ = self.jsonrpc_call(EDITOR_RESET_REGION, Some(params)).await?;
		Ok(())
	}

	// ===============
	// Query and file helpers used by editor flows
	// ===============

	pub async fn world_query(&self, data: Value, filter: Value, strict: bool) -> Result<Vec<Value>, ProtocolError> {
		let params = json!({"data": data, "filter": filter, "strict": strict});
		let v = self.jsonrpc_call(WORLD_QUERY, Some(params)).await?;
		if let Some(arr) = v.get("result").and_then(|r| r.as_array()) { return Ok(arr.clone()); }
		if let Some(arr) = v.as_array() { return Ok(arr.clone()); }
		Err(ProtocolError::Rpc { code: -32603, message: "unexpected response for world.query".to_string(), data: Some(v) })
	}

	pub async fn save_graph(&self, entity: u64, path: &str) -> Result<(), ProtocolError> {
		let params = json!({"entity": entity, "path": path});
		let _ = self.jsonrpc_call(EDITOR_SAVE_GRAPH, Some(params)).await?;
		Ok(())
	}

	pub async fn save_sidecar(&self, path: &str, contents: &str) -> Result<(), ProtocolError> {
		let params = json!({"path": path, "contents": contents});
		let _ = self.jsonrpc_call(EDITOR_SAVE_SIDECAR, Some(params)).await?;
		Ok(())
	}

	pub async fn load_sidecar(&self, path: &str) -> Result<Option<String>, ProtocolError> {
		let params = json!({"path": path});
		let v = self.jsonrpc_call(EDITOR_LOAD_SIDECAR, Some(params)).await?;
		Ok(v.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
	}

	pub async fn find_sidecar_by_fingerprint(&self, fp: &str) -> Result<Option<String>, ProtocolError> {
		let params = json!({"fp": fp});
		let v = self.jsonrpc_call(EDITOR_FIND_SIDECAR_BY_FINGERPRINT, Some(params)).await?;
		Ok(v.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
	}

	pub async fn sidecar_for_machine(&self, entity: u64) -> Result<Option<String>, ProtocolError> {
		let params = json!({"entity": entity});
		let v = self.jsonrpc_call(EDITOR_SIDECAR_FOR_MACHINE, Some(params)).await?;
		// Accept {result:{text}} or top-level {text}
		if let Some(s) = v.get("result").and_then(|r| r.get("text")).and_then(|t| t.as_str()) { return Ok(Some(s.to_string())); }
		Ok(v.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
	}

	pub async fn set_state_machine_id(&self, entity: u64, path: &str) -> Result<(), ProtocolError> {
		let params = json!({"entity": entity, "path": path});
		let _ = self.jsonrpc_call(EDITOR_SET_STATE_MACHINE_ID, Some(params)).await?;
		Ok(())
	}

    pub async fn rename(&self, entity: u64, name: &str) -> Result<(), ProtocolError> {
        let params = json!({ "entity": entity, "components": { wire::NAME_REFLECT: name } });
        let _ = self.jsonrpc_call(WORLD_INSERT_COMPONENTS, Some(params)).await?;
        Ok(())
    }

    pub async fn protocol_version(&self) -> Result<u32, ProtocolError> {
        let v = self.jsonrpc_call(PROTOCOL_VERSION, None).await?;
        Ok(v.get("result").and_then(|r| r.get("version")).and_then(|n| n.as_u64()).unwrap_or(0) as u32)
    }
}

pub fn on_rename(rename: On<crate::events::Rename>, client: Res<ProtocolClient>, rt: Res<TokioRuntime>) {
    // fire-and-forget via Tokio runtime; watches ensure convergence
    let name = rename.name.clone();
    let id = rename.target.to_bits();
    let client_cloned = client.clone();
    let rt = rt.0.clone();
    rt.spawn(async move {
        let _ = client_cloned.rename(id, &name).await;
    });
}

pub fn on_despawn(despawn: On<crate::events::Despawn>, client: Res<ProtocolClient>) {
	let id = despawn.target.to_bits();
	let client_cloned = client.clone();
	bevy::tasks::IoTaskPool::get().spawn(async move {
		let _ = client_cloned.despawn(id).await;
	}).detach();
}

pub fn on_reset_region(reset_region: On<crate::events::ResetRegion>, client: Res<ProtocolClient>) {
	let root = reset_region.target.to_bits();
	let client_cloned = client.clone();
	bevy::tasks::IoTaskPool::get().spawn(async move {
		let _ = client_cloned.reset_region(root).await;
	}).detach();
}

// =========================
// High-level client commands/messages (event-based API)
// =========================

#[derive(Message)]
pub enum ProtocolClientCommand {
    RefreshMachines,
    SetUrl { url: String },
    FetchGraph { id: u64 },
    LoadSidecarByPath { id: u64, path: String },
    SidecarForMachine { id: u64 },
}

#[derive(Message, Clone)]
pub enum ProtocolClientMessage {
    RefreshResult(Result<Vec<ProtocolMachineSummary>, String>),
    GraphResult { id: u64, graph: serde_json::Value },
    SidecarFound { id: u64, text: String },
    SidecarMissing { id: u64 },
    EventEdgeVariants { variants: Vec<String> },
}

async fn list_state_machines(client: &ProtocolClient) -> Result<Vec<ProtocolMachineSummary>, String> {
    let rows = client
        .world_query(serde_json::json!({}), serde_json::json!({"with":[crate::components::STATE_MACHINE]}), false)
        .await
        .map_err(|e| e.to_string())?;
    let mut out: Vec<ProtocolMachineSummary> = Vec::new();
    for row in rows.into_iter() {
        if let Some(id) = row.get("entity").and_then(|e| e.as_u64()) {
            let comps = client
                .get_components(id, Some(&[crate::components::NAME_REFLECT]))
                .await
                .unwrap_or_default();
            let name = comps.get(crate::components::NAME_REFLECT).and_then(|v| v.as_str()).map(|s| s.to_string());
            out.push(ProtocolMachineSummary { id, name });
        }
    }
    Ok(out)
}

fn handle_protocol_client_commands(
    mut reader: MessageReader<ProtocolClientCommand>,
    rt: Res<TokioRuntime>,
    mut client: ResMut<ProtocolClient>,
    mut writer: MessageWriter<ProtocolClientMessage>,
    mut conn: ResMut<ProtocolConnectionState>,
) {
    for cmd in reader.read() {
        match *cmd {
            ProtocolClientCommand::RefreshMachines => {
                let client_cloned = client.clone();
                let r = rt.0.block_on(async move { list_state_machines(&client_cloned).await });
                writer.write(ProtocolClientMessage::RefreshResult(r));
            }
            ProtocolClientCommand::SetUrl { ref url } => {
                client.base_url = url.clone();
                conn.state = ProtocolConnectionPhase::Connecting;
                conn.endpoint = Some(url.clone());
                // Probe connectivity via protocol.version
                let client_cloned = client.clone();
                let ver = rt.0.block_on(async move { client_cloned.protocol_version().await });
                match ver {
                    Ok(_) => {
                        conn.state = ProtocolConnectionPhase::Connected;
                        // Fetch registry schema and publish discovered EventEdge<T> variants
                        let client_cloned2 = client.clone();
                        if let Ok(schema) = rt.0.block_on(async move { client_cloned2.registry_schema().await }) {
                            let variants = extract_event_edge_variants(&schema);
                            if !variants.is_empty() { writer.write(ProtocolClientMessage::EventEdgeVariants { variants }); }
                        }
                    }
                    Err(_) => { conn.state = ProtocolConnectionPhase::Disconnected; }
                }
            }
            ProtocolClientCommand::FetchGraph { id } => {
                let client_cloned = client.clone();
                let v = rt.0.block_on(async move {
                    client_cloned.jsonrpc_call(EDITOR_MACHINE_GRAPH, Some(serde_json::json!({"entity": id}))).await
                });
                if let Ok(v) = v {
                    let graph = v.get("result").cloned().unwrap_or(v);
                    writer.write(ProtocolClientMessage::GraphResult { id, graph });
                } else if let Err(e) = v {
                    writer.write(ProtocolClientMessage::GraphResult { id, graph: serde_json::json!({"error": e.to_string()}) });
                }
            }
            ProtocolClientCommand::LoadSidecarByPath { id, ref path } => {
                let client_cloned = client.clone();
                let path_for_call = path.clone();
                let r = rt.0.block_on(async move { client_cloned.load_sidecar(&path_for_call).await });
                match r {
                    Ok(Some(text)) => { let _ = writer.write(ProtocolClientMessage::SidecarFound { id, text }); }
                    Ok(None) => { let _ = writer.write(ProtocolClientMessage::SidecarMissing { id }); }
                    Err(e) => { let _ = writer.write(ProtocolClientMessage::SidecarMissing { id }); }
                }
            }
            ProtocolClientCommand::SidecarForMachine { id } => {
                let client_cloned = client.clone();
                let r = rt.0.block_on(async move { client_cloned.sidecar_for_machine(id).await });
                match r {
                    Ok(Some(text)) => { let _ = writer.write(ProtocolClientMessage::SidecarFound { id, text }); }
                    Ok(None) => { let _ = writer.write(ProtocolClientMessage::SidecarMissing { id }); }
                    Err(e) => { let _ = writer.write(ProtocolClientMessage::SidecarMissing { id }); }
                }
            }
        }
    }
}

#[derive(Default)]
pub struct GearboxProtocolClientPlugin;

impl Plugin for GearboxProtocolClientPlugin {
    fn build(&self, app: &mut App) {
        if app.world().get_resource::<ProtocolClientConfig>().is_none() {
            let url = std::env::var("GEARBOX_PROTOCOL_URL").or_else(|_| std::env::var("BRP_URL")).unwrap_or_else(|_| "http://127.0.0.1:15703".to_string());
            app.insert_resource(ProtocolClientConfig { url: url.clone() });
            app.insert_resource(ProtocolClient::new(url));
        } else if app.world().get_resource::<ProtocolClient>().is_none() {
            let url = app.world().get_resource::<ProtocolClientConfig>().unwrap().url.clone();
            app.insert_resource(ProtocolClient::new(url));
        }
        if app.world().get_resource::<TokioRuntime>().is_none() {
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
            app.insert_resource(TokioRuntime(Arc::new(rt)));
        }
        app.init_resource::<WatchManager>();
        app.init_resource::<ProtocolCompatibility>();
        app.init_resource::<ProtocolConnectionState>();
        app.add_message::<ProtocolNetMessage>();
        app.add_message::<ProtocolNetCommand>();
        app.add_message::<ProtocolClientCommand>();
        app.add_message::<ProtocolClientMessage>();
        app.add_observer(on_rename);
		app.add_observer(on_despawn);
		app.add_observer(on_reset_region);
        app.add_systems(Startup, protocol_version_check_startup);
        app.add_systems(Update, (handle_protocol_net_commands, drain_protocol_watch_events, handle_protocol_client_commands));
    }
}

// =========================
// Watch Manager (+re-arming)
// =========================

#[derive(Resource, Clone)]
pub struct TokioRuntime(pub Arc<tokio::runtime::Runtime>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolConnectionPhase { Disconnected, Connecting, Connected }

impl Default for ProtocolConnectionPhase { fn default() -> Self { ProtocolConnectionPhase::Disconnected } }

#[derive(Resource, Clone, Default)]
pub struct ProtocolConnectionState {
    pub state: ProtocolConnectionPhase,
    pub endpoint: Option<String>,
}

#[derive(Resource, Default)]
pub struct WatchManager {
    ctl_tx: Option<tokio::sync::mpsc::UnboundedSender<WatchCtl>>,
    evt_rx: Option<tokio::sync::mpsc::UnboundedReceiver<WatchEvt>>,
    cursors: std::collections::HashMap<u64, (u64, u64, u64)>,
}

#[derive(Debug)]
enum WatchCtl {
    StartDiscovery { url: String },
    StopDiscovery,
    StartMachine { url: String, id: u64, last_active_seq: u64, last_transition_seq: u64, last_name_seq: u64 },
    StopMachine { url: String, id: u64 },
    StartComponents { url: String, id: u64, components: Vec<String> },
    StopComponents { url: String, id: u64 },
}

#[derive(Debug, Clone)]
pub struct ProtocolMachineSummary { pub id: u64, pub name: Option<String> }

#[derive(Debug)]
enum WatchEvt {
    Discovery(Vec<ProtocolMachineSummary>),
    Machine { id: u64, events: Vec<Value> },
    Error(String),
    Components { id: u64, components: serde_json::Map<String, Value>, removed: Vec<String> },
}

fn ensure_watch_manager(rt: &tokio::runtime::Runtime, mgr: &mut WatchManager) {
    if mgr.ctl_tx.is_some() { return; }
    let (ctl_tx, mut ctl_rx) = tokio::sync::mpsc::unbounded_channel::<WatchCtl>();
    let (evt_tx, evt_rx) = tokio::sync::mpsc::unbounded_channel::<WatchEvt>();
    mgr.ctl_tx = Some(ctl_tx);
    mgr.evt_rx = Some(evt_rx);

    rt.spawn(async move {
        use futures_util::StreamExt as _;
        use std::collections::HashMap;
        let client = reqwest::Client::new();
        let mut discovery_handle: Option<tokio::task::JoinHandle<()>> = None;
        let mut machine_handles: HashMap<u64, tokio::task::JoinHandle<()>> = HashMap::new();
        let mut component_handles: HashMap<u64, tokio::task::JoinHandle<()>> = HashMap::new();
        while let Some(cmd) = ctl_rx.recv().await {
            match cmd {
                WatchCtl::StartDiscovery { url } => {
                    // Idempotent: if discovery is already running, do nothing
                    if discovery_handle.is_none() {
                        let tx = evt_tx.clone();
                        let client_clone = client.clone();
                        discovery_handle = Some(tokio::spawn(async move {
                            // Track last known snapshot of machines to suppress duplicates
                            let mut known: std::collections::BTreeMap<u64, Option<String>> = std::collections::BTreeMap::new();
                            loop {
                                let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"editor.discovery+watch","params":null});
                                let resp = match client_clone.post(&url).json(&req).send().await { Ok(r) => r, Err(e) => { let _ = tx.send(WatchEvt::Error(format!("watch discovery http: {}", e))); tokio::time::sleep(std::time::Duration::from_millis(300)).await; continue; } };
                                let mut stream = resp.bytes_stream();
                                while let Some(chunk) = stream.next().await {
                                    match chunk {
                                        Ok(bytes) => {
                                            if let Ok(text) = std::str::from_utf8(&bytes) {
                                                // Build a current snapshot map from this chunk's events
                                                let mut current: std::collections::BTreeMap<u64, Option<String>> = std::collections::BTreeMap::new();
                                                for line in text.split('\n') {
                                                    let line = line.trim_start();
                                                    if !line.starts_with("data: ") { continue; }
                                                    let json_str = &line[6..];
                                                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                                                        if let Some(events) = v.get("result").and_then(|r| r.get("events")).and_then(|e| e.as_array()) {
                                                            for ev in events {
                                                                let kind = ev.get("kind").and_then(|s| s.as_str()).unwrap_or("");
                                                                match kind {
                                                                    "machine_created" | "machine_renamed" | "machine_id_set" => {
                                                                        if let Some(raw) = ev.get("machine").and_then(|v| v.as_u64()) {
                                                                            let name = ev.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
                                                                            current.insert(raw, name);
                                                                        }
                                                                    }
                                                                    "machine_removed" => {
                                                                        if let Some(raw) = ev.get("machine").and_then(|v| v.as_u64()) { let _ = current.remove(&raw); }
                                                                    }
                                                                    _ => {}
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                // If server sends a full snapshot each tick, compute a diff vs known
                                                if !current.is_empty() {
                                                    // Compute diffs
                                                    let mut diff: Vec<ProtocolMachineSummary> = Vec::new();
                                                    // Added or changed
                                                    for (id, name) in current.iter() {
                                                        if known.get(id) != Some(name) {
                                                            diff.push(ProtocolMachineSummary { id: *id, name: name.clone() });
                                                        }
                                                    }
                                                    // Removed
                                                    for id in known.keys() {
                                                        if !current.contains_key(id) {
                                                            diff.push(ProtocolMachineSummary { id: *id, name: None });
                                                        }
                                                    }
                                                    if !diff.is_empty() {
                                                        let _ = tx.send(WatchEvt::Discovery(diff));
                                                    }
                                                    known = current;
                                                }
                                            }
                                        }
                                        Err(e) => { let _ = tx.send(WatchEvt::Error(format!("watch discovery stream: {}", e))); break; }
                                    }
                                }
                                // reconnect loop
                            }
                        }));
                    }
                }
                WatchCtl::StopDiscovery => { if let Some(h) = discovery_handle.take() { h.abort(); } }
                WatchCtl::StartMachine { url, id, mut last_active_seq, mut last_transition_seq, mut last_name_seq } => {
                    // Idempotent: if this machine watch is already running, do nothing
                    if machine_handles.contains_key(&id) { continue; }
                    let tx = evt_tx.clone();
                    let client_clone = client.clone();
                    let handle = tokio::spawn(async move {
                        // Subscribe once before starting stream
                        let _ = client_clone.post(&url).json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":EDITOR_MACHINE_SUBSCRIBE,"params":{"entity":id}})).send().await;
                        loop {
                            let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"editor.machine+watch","params":{"entity":id,"last_active_seq":last_active_seq,"last_transition_seq":last_transition_seq, "last_name_seq": last_name_seq}});
                            let resp = match client_clone.post(&url).json(&req).send().await { Ok(r) => r, Err(e) => { let _ = tx.send(WatchEvt::Error(format!("watch http: {}", e))); tokio::time::sleep(std::time::Duration::from_millis(300)).await; continue; } };
                            let mut stream = resp.bytes_stream();
                            while let Some(chunk) = stream.next().await {
                                match chunk {
                                    Ok(bytes) => {
                                        if let Ok(text) = std::str::from_utf8(&bytes) {
                                            let mut out: Vec<Value> = Vec::new();
                                            for line in text.split('\n') {
                                                let line = line.trim_start();
                                                if !line.starts_with("data: ") { continue; }
                                                let json_str = &line[6..];
                                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                                                    if let Some(events) = v.get("result").and_then(|r| r.get("events")).and_then(|e| e.as_array()) {
                                                        if !events.is_empty() {
                                                            for ev in events.iter() {
                                                                if let Some(seq) = ev.get("seq").and_then(|v| v.as_u64()) {
                                                                    match ev.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
                                                                        "active_changed" => if seq > last_active_seq { last_active_seq = seq; },
                                                                        "transition_edge" => if seq > last_transition_seq { last_transition_seq = seq; },
                                                                        "name_changed" => if seq > last_name_seq { last_name_seq = seq; },
                                                                        _ => {}
                                                                    }
                                                                }
                                                            }
                                                            out.extend(events.iter().cloned());
                                                        }
                                                    }
                                                }
                                            }
                                            if !out.is_empty() {
                                                let mut ac = 0usize; let mut tc = 0usize; let mut nc = 0usize; let total = out.len();
                                                for ev in out.iter() {
                                                    match ev.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
                                                        "active_changed" => ac += 1,
                                                        "transition_edge" => tc += 1,
                                                        "name_changed" => nc += 1,
                                                        _ => {}
                                                    }
                                                }
                                            }
                                            if !out.is_empty() { let _ = tx.send(WatchEvt::Machine { id, events: out }); }
                                        }
                                    }
                                    Err(e) => { let _ = tx.send(WatchEvt::Error(format!("watch stream: {}", e))); break; }
                                }
                            }
                        }
                    });
                    machine_handles.insert(id, handle);
                }
                WatchCtl::StopMachine { url, id } => {
                    // Best-effort unsubscribe
                    let _ = client.post(&url).json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":EDITOR_MACHINE_UNSUBSCRIBE,"params":{"entity":id}})).send().await;
                    if let Some(h) = machine_handles.remove(&id) { h.abort(); }
                }
                WatchCtl::StartComponents { url, id, components } => {
                    if component_handles.contains_key(&id) { continue; }
                    let tx = evt_tx.clone();
                    let client_clone = client.clone();
                    let comps = components.clone();
                    let handle = tokio::spawn(async move {
                        loop {
                            let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"world.get_components+watch","params":{"entity":id,"components": comps}});
                            let resp = match client_clone.post(&url).json(&req).send().await { Ok(r) => r, Err(e) => { let _ = tx.send(WatchEvt::Error(format!("components watch http: {}", e))); tokio::time::sleep(std::time::Duration::from_millis(300)).await; continue; } };
                            let mut stream = resp.bytes_stream();
                            while let Some(chunk) = stream.next().await {
                                match chunk {
                                    Ok(bytes) => {
                                        if let Ok(text) = std::str::from_utf8(&bytes) {
                                            for line in text.split('\n') {
                                                let line = line.trim_start();
                                                if !line.starts_with("data: ") { continue; }
                                                let json_str = &line[6..];
                                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                                                    let res = v.get("result").cloned().unwrap_or(v.clone());
                                                    let comps_obj = res.get("components").and_then(|c| c.as_object()).cloned().unwrap_or_default();
                                                    let removed_arr: Vec<String> = res.get("removed").and_then(|a| a.as_array()).map(|arr| arr.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
                                                    if !comps_obj.is_empty() || !removed_arr.is_empty() {
                                                        let _ = tx.send(WatchEvt::Components { id, components: comps_obj, removed: removed_arr });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => { let _ = tx.send(WatchEvt::Error(format!("components watch stream: {}", e))); break; }
                                }
                            }
                            // reconnect
                        }
                    });
                    component_handles.insert(id, handle);
                }
                WatchCtl::StopComponents { url: _url, id } => {
                    if let Some(h) = component_handles.remove(&id) { h.abort(); }
                }
            }
        }
    });
}

#[derive(Message, Clone)]
pub enum ProtocolNetMessage {
    Discovery(Vec<ProtocolMachineSummary>),
    Machine { id: u64, events: Vec<Value> },
    Components { id: u64, components: serde_json::Map<String, Value>, removed: Vec<String> },
}

#[derive(Message)]
pub enum ProtocolNetCommand {
    StartDiscovery,
    StopDiscovery,
    StartMachine { id: u64 },
    StopMachine { id: u64 },
    StartComponents { id: u64, components: Vec<String> },
    StopComponents { id: u64 },
}

fn handle_protocol_net_commands(
    mut reader: MessageReader<ProtocolNetCommand>,
    rt: Res<TokioRuntime>,
    client: Res<ProtocolClient>,
    mut mgr: ResMut<WatchManager>,
) {
    ensure_watch_manager(&rt.0, &mut *mgr);
    for cmd in reader.read() {
        match *cmd {
            ProtocolNetCommand::StartDiscovery => {
                if let Some(tx) = &mgr.ctl_tx { let _ = tx.send(WatchCtl::StartDiscovery { url: client.base_url.clone() }); }
            }
            ProtocolNetCommand::StopDiscovery => {
                if let Some(tx) = &mgr.ctl_tx { let _ = tx.send(WatchCtl::StopDiscovery); }
            }
            ProtocolNetCommand::StartMachine { id } => {
                let (la, lt, ln) = mgr.cursors.get(&id).copied().unwrap_or((0, 0, 0));
                if let Some(tx) = &mgr.ctl_tx { let _ = tx.send(WatchCtl::StartMachine { url: client.base_url.clone(), id, last_active_seq: la, last_transition_seq: lt, last_name_seq: ln }); }
            }
            ProtocolNetCommand::StopMachine { id } => {
                if let Some(tx) = &mgr.ctl_tx { let _ = tx.send(WatchCtl::StopMachine { url: client.base_url.clone(), id }); }
            }
            ProtocolNetCommand::StartComponents { id, ref components } => {
                if let Some(tx) = &mgr.ctl_tx { let _ = tx.send(WatchCtl::StartComponents { url: client.base_url.clone(), id, components: components.clone() }); }
            }
            ProtocolNetCommand::StopComponents { id } => {
                if let Some(tx) = &mgr.ctl_tx { let _ = tx.send(WatchCtl::StopComponents { url: client.base_url.clone(), id }); }
            }
        }
    }
}

fn drain_protocol_watch_events(
    mut mgr: ResMut<WatchManager>,
    mut writer: MessageWriter<ProtocolNetMessage>,
) {
    if mgr.evt_rx.is_some() {
        let mut rx = mgr.evt_rx.take().unwrap();
        let mut drained = 0usize;
        while let Ok(evt) = rx.try_recv() {
            drained += 1;
            match evt {
                WatchEvt::Discovery(batch) => {
                    writer.write(ProtocolNetMessage::Discovery(batch));
                }
                WatchEvt::Machine { id, events } => {
                    // Filter duplicates using stored cursors
                    let (prev_a, prev_t, prev_n) = mgr.cursors.get(&id).copied().unwrap_or((0, 0, 0));
                    let mut max_a = prev_a;
                    let mut max_t = prev_t;
                    let mut max_n = prev_n;
                    let mut filtered: Vec<serde_json::Value> = Vec::with_capacity(events.len());
                    for ev in events.into_iter() {
                        let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                        let seq = ev.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
                        match kind {
                            "active_changed" => {
                                if seq > prev_a { filtered.push(ev.clone()); if seq > max_a { max_a = seq; } }
                            }
                            "transition_edge" => {
                                if seq > prev_t { filtered.push(ev.clone()); if seq > max_t { max_t = seq; } }
                            }
                            "name_changed" => {
                                if seq > prev_n { filtered.push(ev.clone()); if seq > max_n { max_n = seq; } }
                            }
                            _ => {
                                // pass through unknown kinds
                                filtered.push(ev.clone());
                            }
                        }
                    }
                    // Update cursors from filtered
                    mgr.cursors.insert(id, (max_a, max_t, max_n));
                    if !filtered.is_empty() {
                        let mut ac = 0usize; let mut tc = 0usize; let mut nc = 0usize; let total = filtered.len();
                        for ev in filtered.iter() {
                            match ev.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
                                "active_changed" => ac += 1,
                                "transition_edge" => tc += 1,
                                "name_changed" => nc += 1,
                                _ => {}
                            }
                        }
                        writer.write(ProtocolNetMessage::Machine { id, events: filtered });
                    }
                }
                WatchEvt::Error(e) => { println!("[dbg] client: watch error: {}", e); }
                WatchEvt::Components { id, components, removed } => {
                    writer.write(ProtocolNetMessage::Components { id, components, removed });
                }
            }
        }
        if drained > 0 { /* optional log */ }
        mgr.evt_rx = Some(rx);
    }
}

// =========================
// Protocol version check
// =========================

const SUPPORTED_VERSION_MIN: u32 = 1;
const SUPPORTED_VERSION_MAX: u32 = 1;

#[derive(Resource, Clone, Default)]
pub struct ProtocolCompatibility {
    pub server_version: Option<u32>,
    pub compatible: bool,
    pub message: Option<String>,
}

fn protocol_version_check_startup(
    client: Res<ProtocolClient>,
    rt: Res<TokioRuntime>,
    mut compat: ResMut<ProtocolCompatibility>,
) {
    match rt.0.block_on(client.protocol_version()) {
        Ok(ver) => {
            let ok = ver >= SUPPORTED_VERSION_MIN && ver <= SUPPORTED_VERSION_MAX;
            compat.server_version = Some(ver);
            compat.compatible = ok;
            if ok {
                compat.message = Some(format!("Protocol version OK: {} (supported {}..={})", ver, SUPPORTED_VERSION_MIN, SUPPORTED_VERSION_MAX));
                bevy::log::info!("[protocol] version {} compatible", ver);
            } else {
                compat.message = Some(format!("Protocol version {} unsupported (supported {}..={})", ver, SUPPORTED_VERSION_MIN, SUPPORTED_VERSION_MAX));
                bevy::log::error!("[protocol] version {} unsupported (supported {}..={})", ver, SUPPORTED_VERSION_MIN, SUPPORTED_VERSION_MAX);
            }
        }
        Err(e) => {
            compat.server_version = None;
            compat.compatible = false;
            compat.message = Some(format!("Failed to get protocol version: {}", e));
            bevy::log::error!("[protocol] failed to get version: {}", e);
        }
    }
}


// =========================
// Helper: extract EventEdge<T> variants from registry.schema
// =========================
fn extract_event_edge_variants(schema: &serde_json::Value) -> Vec<String> {
    fn collect_strings(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Array(arr) => { for x in arr { collect_strings(x, out); } }
            serde_json::Value::Object(map) => { for (_k, vv) in map { collect_strings(vv, out); } }
            _ => {}
        }
    }
    fn inner_generic(name: &str) -> Option<String> {
        let lb = name.find('<')?;
        let rb = name.rfind('>')?;
        if rb > lb + 1 {
            return Some(name[lb + 1..rb].to_string());
        }
        None
    }
    fn simple_type(name: &str) -> String {
        name.rsplit("::").next().unwrap_or(name).to_string()
    }
    let mut strings: Vec<String> = Vec::new();
    collect_strings(schema, &mut strings);
    let mut out: Vec<String> = Vec::new();
    for s in strings.into_iter() {
        if s.contains(crate::components::EVENT_EDGE_SUBSTR) {
            if let Some(inner) = inner_generic(&s) { out.push(simple_type(&inner)); }
        }
    }
    out.sort();
    out.dedup();
    out
}

