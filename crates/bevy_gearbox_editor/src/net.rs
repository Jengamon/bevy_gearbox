use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, IoTaskPool, Task};
use crate::rpcs::{list_state_machines, fetch_machine_graph_model};
use crate::client::{jsonrpc_save_machine, jsonrpc_select};
use crate::types::{ServerEntity, MachineSummary, NetError};
use crate::model::StateMachineGraph;
use std::sync::Arc;

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum NetSet { Send, Drain }

pub(crate) struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<NetCommand>()
            .add_message::<NetEvent>()
            .insert_resource(PendingTasks::default())
            .insert_resource(TokioRuntime(Arc::new(tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime"))))
            .insert_resource(NetworkConfig { url: std::env::var("BRP_URL").unwrap_or_else(|_| "http://127.0.0.1:15703".to_string()) })
            .insert_resource(ConnectionStatus(ConnectionState::Disconnected))
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
    Select { id: ServerEntity },
    Save { id: ServerEntity },
    /// Save As: logical asset base name (for .scn.ron on app) and local sidecar path
    SaveAs { id: ServerEntity, asset_base: String, sidecar_path: std::path::PathBuf },
    FetchGraph { id: ServerEntity },
}

#[derive(Message)]
pub(crate) enum NetEvent {
    Connected,
    Disconnected { reason: Option<NetError> },
    ConnectionError(NetError),
    RefreshResult(Result<Vec<MachineSummary>, NetError>),
    SelectResult(Result<(), NetError>),
    SaveResult(Result<(), NetError>),
    GraphResult { id: ServerEntity, result: Result<StateMachineGraph, NetError> },
}

#[derive(Resource, Default)]
pub(crate) struct PendingTasks {
    tasks: Vec<Task<NetEvent>>,
}

pub(crate) fn handle_commands(
    mut commands_reader: MessageReader<NetCommand>,
    mut pending: ResMut<PendingTasks>,
    mut cfg: ResMut<NetworkConfig>,
    mut conn: ResMut<ConnectionStatus>,
    rt: Res<TokioRuntime>,
) {
    let pool = IoTaskPool::get();
    for cmd in commands_reader.read().cloned() {
        match cmd {
            NetCommand::SetUrl { url } => {
                cfg.url = url;
            }
            NetCommand::Connect => {
                conn.0 = ConnectionState::Connecting;
                let url = cfg.url.clone();
                let rt_handle = rt.0.clone();
                let task = pool.spawn(async move {
                    rt_handle.block_on(async move {
                        match crate::client::jsonrpc_ping(&url).await {
                            Ok(()) => NetEvent::Connected,
                            Err(e) => NetEvent::ConnectionError(NetError::from(e)),
                        }
                    })
                });
                pending.tasks.push(task);
            }
            NetCommand::Disconnect => {
                conn.0 = ConnectionState::Disconnected;
                let task = pool.spawn(async move { NetEvent::Disconnected { reason: None } });
                pending.tasks.push(task);
            }
            other => {
                let url = cfg.url.clone();
                let rt_handle = rt.0.clone();
                let task = pool.spawn(async move {
                    rt_handle.block_on(async move {
                        match other {
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
                                let result = fetch_machine_graph_model(&url, id.0).await.map_err(NetError::from);
                                NetEvent::GraphResult { id, result }
                            }
                            NetCommand::SaveAs { id, asset_base, sidecar_path } => {
                                let r = (async move {
                                    // 1) Ask app to save scene to <asset_base>.scn.ron
                                    crate::rpcs::save_graph(&url, id.0, &asset_base).await.map_err(NetError::from)?;
                                    // 2) Return Ok; caller will write sidecar synchronously on main thread
                                    Ok(()) as Result<(), NetError>
                                }).await;
                                NetEvent::SaveResult(r)
                            }
                            NetCommand::Connect | NetCommand::Disconnect => unreachable!(),
                            NetCommand::SetUrl { .. } => unreachable!(),
                        }
                    })
                });
                pending.tasks.push(task);
            }
        }
    }
}

pub(crate) fn collect_task_results(
    mut pending: ResMut<PendingTasks>,
    mut events_writer: MessageWriter<NetEvent>,
    mut conn: ResMut<ConnectionStatus>,
) {
    let mut i = 0;
    while i < pending.tasks.len() {
        let is_ready = if let Some(evt) = future::block_on(bevy::tasks::futures_lite::future::poll_once(&mut pending.tasks[i])) {
            match &evt {
                NetEvent::Connected => conn.0 = ConnectionState::Connected,
                NetEvent::Disconnected { .. } | NetEvent::ConnectionError(_) => conn.0 = ConnectionState::Disconnected,
                _ => {}
            }
            events_writer.write(evt);
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


