use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, IoTaskPool, Task};
use crate::rpcs::{list_state_machines, fetch_machine_graph_model};
use crate::rpcs as rpc;
use crate::client::{jsonrpc_save_machine, jsonrpc_select};
use crate::types::{ServerEntity, MachineSummary, NetError};
use crate::model::StateMachineGraph;
use std::sync::Arc;
use reqwest::Client;

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum NetSet { Send, Drain }

pub(crate) struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<NetCommand>()
            .add_message::<StampedEvent>()
            .insert_resource(PendingTasks::default())
            .insert_resource(TokioRuntime(Arc::new(tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime"))))
            .insert_resource(NetworkConfig { url: std::env::var("BRP_URL").unwrap_or_else(|_| "http://127.0.0.1:15703".to_string()) })
            .insert_resource(ConnectionStatus(ConnectionState::Disconnected))
            .insert_resource(ActiveSession(0))
            .configure_sets(Update, (NetSet::Send, NetSet::Drain).chain())
            .add_systems(Update, handle_commands.in_set(NetSet::Send))
            .add_systems(Update, collect_task_results.in_set(NetSet::Drain));
    }
}

#[derive(Resource, Clone)]
pub(crate) struct NetworkConfig {
    pub(crate) url: String,
}

#[derive(Resource, Clone)]
pub(crate) struct TokioRuntime(pub Arc<tokio::runtime::Runtime>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Resource, Clone)]
pub(crate) struct ConnectionStatus(pub ConnectionState);

#[derive(Message, Clone)]
pub(crate) enum NetCommand {
    SetUrl { url: String },
    Connect,
    Disconnect,
    Refresh,
    /// Start discovery SSE watcher (BRP +watch)
    StartDiscoveryWatch,
    /// Start per-machine SSE watcher (BRP +watch). Stateless: pass last seen seqs
    StartMachineWatch { id: ServerEntity, last_active_seq: u64, last_transition_seq: u64 },
    /// Stop per-machine SSE watcher (no-op placeholder; task auto-exits on re-arm)
    StopMachineWatch { id: ServerEntity },
    Select { id: ServerEntity },
    Save { id: ServerEntity },
    /// Save As: logical asset base name (for .scn.ron on app) and local sidecar path
    SaveAs { id: ServerEntity, asset_base: String, sidecar_path: std::path::PathBuf },
    FetchSidecarByFingerprint { fingerprint: String },
    FetchSidecarByPath { path: String, doc: ServerEntity },
    FetchGraph { id: ServerEntity },
    /// Server-side subscribe/unsubscribe for gating feeds
    Subscribe { id: ServerEntity },
    Unsubscribe { id: ServerEntity },
    /// One-shot fetch of active states to seed highlights
    FetchActive { id: ServerEntity },
}

#[derive(Message)]
pub(crate) enum NetEvent {
    Connected,
    Disconnected { reason: Option<NetError> },
    ConnectionError(NetError),
    RefreshResult(Result<Vec<MachineSummary>, NetError>),
    DiscoveryEvents(Result<Vec<MachineSummary>, NetError>),
    SelectResult(Result<(), NetError>),
    SaveResult(Result<(), NetError>),
    SidecarResult(Result<Option<String>, NetError>),
    SidecarResultFor { id: ServerEntity, result: Result<Option<String>, NetError> },
    GraphResult { id: ServerEntity, result: Result<StateMachineGraph, NetError> },
    /// Streamed machine deltas from +watch
    MachineDeltas { id: ServerEntity, result: Result<Vec<serde_json::Value>, NetError> },
    /// One-shot active states fetch
    ActiveResult { id: ServerEntity, result: Result<(Vec<u64>, Vec<u64>), NetError> },
}

#[derive(Message, Clone)]
pub(crate) struct StampedEvent {
    pub session: u64,
    pub event: Arc<NetEvent>,
}

#[derive(Resource, Default)]
pub(crate) struct PendingTasks {
    tasks: Vec<Task<StampedEvent>>,
}

#[derive(Resource, Clone, Copy)]
pub(crate) struct ActiveSession(pub u64);

pub(crate) fn handle_commands(
    mut commands_reader: MessageReader<NetCommand>,
    mut pending: ResMut<PendingTasks>,
    mut cfg: ResMut<NetworkConfig>,
    mut conn: ResMut<ConnectionStatus>,
    rt: Res<TokioRuntime>,
    active: Res<ActiveSession>,
) {
    let pool = IoTaskPool::get();
    let cmds: Vec<NetCommand> = commands_reader.read().cloned().collect();
    if !cmds.is_empty() { println!("[net] handle_commands: cmds_this_frame={}", cmds.len()); }
    for cmd in cmds.into_iter() {
        match cmd {
            NetCommand::SetUrl { url } => {
                cfg.url = url;
            }
            NetCommand::Connect => {
                conn.0 = ConnectionState::Connecting;
                let url = cfg.url.clone();
                let rt_handle = rt.0.clone();
                let session = active.0;
                let task = pool.spawn(async move {
                    rt_handle.block_on(async move {
                        let evt = match crate::client::jsonrpc_ping(&url).await {
                            Ok(()) => NetEvent::Connected,
                            Err(e) => NetEvent::ConnectionError(NetError::from(e)),
                        };
                        StampedEvent { session, event: Arc::new(evt) }
                    })
                });
                pending.tasks.push(task);
                // Discovery watch will be triggered by UI upon Connected event
            }
            NetCommand::StartDiscoveryWatch => {
                let url = cfg.url.clone();
                let session = active.0;
                let rt_handle = rt.0.clone();
                let task = pool.spawn(async move {
                    rt_handle.block_on(async move {
                        use futures_util::StreamExt as _;
                        let client = Client::new();
                        // Compose JSON-RPC +watch request
                        let req = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "method": "editor.discovery+watch",
                            "params": null,
                        });
                        let resp = match client.post(&url).json(&req).send().await {
                            Ok(r) => r,
                            Err(e) => {
                                let evt = NetEvent::ConnectionError(NetError::Other(format!("watch http: {}", e)));
                                return StampedEvent { session, event: Arc::new(evt) };
                            }
                        };
                        let mut stream = resp.bytes_stream();
                        let mut summaries: Vec<MachineSummary> = Vec::new();
                        // Time-box a single chunk read to avoid long-lived tasks
                        let next_chunk = tokio::time::timeout(std::time::Duration::from_millis(200), stream.next()).await;
                        match next_chunk {
                            Ok(Some(Ok(bytes))) => {
                                if let Ok(text) = std::str::from_utf8(&bytes) {
                                    for line in text.split('\n') {
                                        let line = line.trim_start();
                                        if !line.starts_with("data: ") { continue; }
                                        let json_str = &line[6..];
                                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                                            let events = v.get("result")
                                                .and_then(|r| r.get("events"))
                                                .and_then(|e| e.as_array())
                                                .cloned()
                                                .unwrap_or_default();
                                            for ev in events {
                                                let kind = ev.get("kind").and_then(|s| s.as_str()).unwrap_or("");
                                                match kind {
                                                    "machine_created" | "machine_renamed" | "machine_id_set" => {
                                                        if let Some(raw) = ev.get("machine").and_then(|v| v.as_u64()) {
                                                            let canon = crate::util::canonicalize_entity_u64(raw);
                                                            let name = ev.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
                                                            summaries.push(MachineSummary { id: ServerEntity(canon), name });
                                                        }
                                                    }
                                                    "machine_removed" => {
                                                        if let Some(raw) = ev.get("machine").and_then(|v| v.as_u64()) {
                                                            let canon = crate::util::canonicalize_entity_u64(raw);
                                                            summaries.push(MachineSummary { id: ServerEntity(canon), name: None });
                                                        }
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                }
                                let evt = NetEvent::DiscoveryEvents(Ok(std::mem::take(&mut summaries)));
                                StampedEvent { session, event: Arc::new(evt) }
                            }
                            Ok(Some(Err(e))) => {
                                let evt = NetEvent::ConnectionError(NetError::Other(format!("watch stream: {}", e)));
                                StampedEvent { session, event: Arc::new(evt) }
                            }
                            // Timeout, end-of-stream, or no chunk => emit benign empty batch
                            _ => {
                                let evt = NetEvent::DiscoveryEvents(Ok(Vec::new()));
                                StampedEvent { session, event: Arc::new(evt) }
                            }
                        }
                    })
                });
                pending.tasks.push(task);
            }
            NetCommand::StartMachineWatch { id, last_active_seq, last_transition_seq } => {
                let url = cfg.url.clone();
                let session = active.0;
                let rt_handle = rt.0.clone();
                let task = pool.spawn(async move {
                    rt_handle.block_on(async move {
                        use futures_util::StreamExt as _;
                        let client = Client::new();
                        let req = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "method": "editor.machine+watch",
                            "params": { "entity": id.0, "last_active_seq": last_active_seq, "last_transition_seq": last_transition_seq },
                        });
                        let resp = match client.post(&url).json(&req).send().await {
                            Ok(r) => r,
                            Err(e) => {
                                let evt = NetEvent::ConnectionError(NetError::Other(format!("watch http: {}", e)));
                                return StampedEvent { session, event: Arc::new(evt) };
                            }
                        };
                        let mut stream = resp.bytes_stream();
                        // Time-box a single chunk read to avoid long-lived tasks
                        let next_chunk = tokio::time::timeout(std::time::Duration::from_millis(200), stream.next()).await;
                        match next_chunk {
                            Ok(Some(Ok(bytes))) => {
                                if let Ok(text) = std::str::from_utf8(&bytes) {
                                    for line in text.split('\n') {
                                        let line = line.trim_start();
                                        if !line.starts_with("data: ") { continue; }
                                        let json_str = &line[6..];
                                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                                            if let Some(events) = v.get("result").and_then(|r| r.get("events")).and_then(|e| e.as_array()) {
                                                let evt = NetEvent::MachineDeltas { id, result: Ok(events.iter().cloned().collect()) };
                                                return StampedEvent { session, event: Arc::new(evt) };
                                            }
                                        }
                                    }
                                }
                                let evt = NetEvent::MachineDeltas { id, result: Ok(Vec::new()) };
                                StampedEvent { session, event: Arc::new(evt) }
                            }
                            Ok(Some(Err(e))) => {
                                let evt = NetEvent::ConnectionError(NetError::Other(format!("watch stream: {}", e)));
                                StampedEvent { session, event: Arc::new(evt) }
                            }
                            // Timeout, end-of-stream, or no chunk => benign empty batch
                            _ => {
                                let evt = NetEvent::MachineDeltas { id, result: Ok(Vec::new()) };
                                StampedEvent { session, event: Arc::new(evt) }
                            }
                        }
                    })
                });
                pending.tasks.push(task);
            }
            NetCommand::StopMachineWatch { .. } => {
                // No persistent handle kept; SSE tasks complete after a batch. This is a no-op placeholder.
            }
            NetCommand::Disconnect => {
                conn.0 = ConnectionState::Disconnected;
                let session = active.0;
                let task = pool.spawn(async move { StampedEvent { session, event: Arc::new(NetEvent::Disconnected { reason: None }) } });
                pending.tasks.push(task);
            }
            other => {
                let url = cfg.url.clone();
                let rt_handle = rt.0.clone();
                let session = active.0;
                let task = pool.spawn(async move {
                    rt_handle.block_on(async move {
                        let evt = match other {
                            NetCommand::Refresh => {
                                let r: Result<Vec<MachineSummary>, NetError> = (async move {
                                    let list = list_state_machines(&url).await.map_err(NetError::from)?;
                                    let machines = list.into_iter().map(|(id, name)| MachineSummary { id: ServerEntity(id), name }).collect();
                                    Ok(machines)
                                }).await;
                                NetEvent::RefreshResult(r)
                            }
                            NetCommand::Select { id } => {
                                let r = jsonrpc_select(&url, Some(id.0 as u32)).await.map_err(NetError::from);
                                NetEvent::SelectResult(r)
                            }
                            NetCommand::Save { id } => {
                                let r = jsonrpc_save_machine(&url, id.0 as u32).await.map_err(NetError::from);
                                NetEvent::SaveResult(r)
                            }
                            NetCommand::FetchGraph { id } => {
                                println!("[net] fetch_graph start: id={:?}", id);
                                let result = fetch_machine_graph_model(&url, id.0).await.map_err(NetError::from);
                                match &result {
                                    Ok(g) => println!("[net] fetch_graph result: id={:?}, nodes={}, edges={}", id, g.nodes.len(), g.edges.len()),
                                    Err(e) => println!("[net] fetch_graph error: id={:?}, err={}", id, e),
                                }
                                NetEvent::GraphResult { id, result }
                            }
                            NetCommand::SaveAs { id, asset_base, sidecar_path } => {
                                let r = (async move {
                                    // 1) Ask app to save scene to <asset_base>.scn.ron
                                    rpc::save_graph(&url, id.0, &asset_base).await.map_err(NetError::from)?;
                                    // 2) Also push sidecar to server assets: read local file and send
                                    if let Ok(txt) = std::fs::read_to_string(&sidecar_path) {
                                        let server_path = format!("{}.sm.ron", asset_base);
                                        let _ = rpc::save_sidecar(&url, &server_path, &txt).await.map_err(NetError::from)?;
                                        // 3) Set the pointer on the root machine
                                        let _ = rpc::set_state_machine_id(&url, id.0, &server_path).await.map_err(NetError::from)?;
                                    }
                                    Ok(()) as Result<(), NetError>
                                }).await;
                                NetEvent::SaveResult(r)
                            }
                            NetCommand::FetchSidecarByFingerprint { fingerprint } => {
                                let r = (async move {
                                    let txt = rpc::find_sidecar_by_fingerprint(&url, &fingerprint).await.map_err(NetError::from)?;
                                    Ok(txt)
                                }).await;
                                NetEvent::SidecarResult(r)
                            }
                            NetCommand::FetchSidecarByPath { path, doc } => {
                                let r = (async move {
                                    let txt = rpc::load_sidecar(&url, &path).await.map_err(NetError::from)?;
                                    Ok(txt)
                                }).await;
                                NetEvent::SidecarResultFor { id: doc, result: r }
                            }
                            NetCommand::FetchActive { id } => {
                                println!("[net] fetch_active start: id={:?}", id);
                                let r = (async move {
                                    let (a, l) = rpc::fetch_active_states(&url, id.0).await.map_err(NetError::from)?;
                                    Ok((a, l))
                                }).await;
                                NetEvent::ActiveResult { id, result: r }
                            }
                            NetCommand::Subscribe { id } => {
                                let r = (async move {
                                    rpc::subscribe_machine(&url, id.0).await.map_err(NetError::from)
                                }).await;
                                match r { Ok(_) => NetEvent::SelectResult(Ok(())), Err(e) => NetEvent::SelectResult(Err(e)) }
                            }
                            NetCommand::Unsubscribe { id } => {
                                let r = (async move {
                                    rpc::unsubscribe_machine(&url, id.0).await.map_err(NetError::from)
                                }).await;
                                match r { Ok(_) => NetEvent::SelectResult(Ok(())), Err(e) => NetEvent::SelectResult(Err(e)) }
                            }
                            NetCommand::StartDiscoveryWatch => unreachable!(),
                            NetCommand::StartMachineWatch { .. } => unreachable!(),
                            NetCommand::StopMachineWatch { .. } => unreachable!(),
                            NetCommand::Connect | NetCommand::Disconnect => unreachable!(),
                            NetCommand::SetUrl { .. } => unreachable!(),
                        };
                        StampedEvent { session, event: Arc::new(evt) }
                    })
                });
                pending.tasks.push(task);
            }
        }
    }
}

pub(crate) fn collect_task_results(
    mut pending: ResMut<PendingTasks>,
    mut events_writer: MessageWriter<StampedEvent>,
    mut conn: ResMut<ConnectionStatus>,
) {
    let mut i = 0;
    while i < pending.tasks.len() {
        let is_ready = if let Some(stamped) = future::block_on(bevy::tasks::futures_lite::future::poll_once(&mut pending.tasks[i])) {
            match &*stamped.event {
                NetEvent::Connected => conn.0 = ConnectionState::Connected,
                NetEvent::Disconnected { .. } | NetEvent::ConnectionError(_) => conn.0 = ConnectionState::Disconnected,
                _ => {}
            }
            events_writer.write(stamped);
            true
        } else {
            false
        };
        if is_ready {
            let task = pending.tasks.swap_remove(i);
            task.detach();
        } else {
            i += 1;
        }
    }
}


