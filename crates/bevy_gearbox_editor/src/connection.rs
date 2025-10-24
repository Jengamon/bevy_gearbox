use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, IoTaskPool, Task};
use crate::rpcs::{extract_components_map, fetch_machine_graph_text};
use crate::client::{jsonrpc_call, jsonrpc_ping, jsonrpc_save_machine, jsonrpc_select};
use serde_json::{json, Value};
use crate::component as c;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct ServerEntity(pub u64);

#[derive(Resource, Clone)]
pub(crate) struct NetworkConfig {
    pub(crate) url: String,
}

#[derive(Message, Clone)]
pub(crate) enum EditorCommand {
    Refresh,
    Select { id: ServerEntity },
    Save { id: ServerEntity },
    FetchGraph { id: ServerEntity },
}

#[derive(Message)]
pub(crate) enum EditorEvent {
    RefreshResult(Result<Vec<(ServerEntity, Option<String>)>, String>),
    SelectResult(Result<(), String>),
    SaveResult(Result<(), String>),
    GraphResult { id: ServerEntity, result: Result<String, String> },
}

fn extract_result_array(v: Value) -> Result<Vec<Value>, String> {
    match v {
        Value::Array(a) => Ok(a),
        Value::Object(o) => match o.get("result") {
            Some(Value::Array(a)) => Ok(a.clone()),
            Some(other) => Err(format!("expected result array, got {}", other)),
            None => Err("missing result in object".to_string()),
        },
        other => Err(format!("expected array or result object, got {}", other)),
    }
}

#[derive(Resource, Default)]
pub(crate) struct PendingTasks {
    tasks: Vec<Task<EditorEvent>>,
}

pub(crate) fn handle_commands(
    mut commands_reader: MessageReader<EditorCommand>,
    mut pending: ResMut<PendingTasks>,
    cfg: Res<NetworkConfig>,
) {
    let pool = IoTaskPool::get();
    for cmd in commands_reader.read().cloned() {
        let url = cfg.url.clone();
        let task = pool.spawn(async move {
            match cmd {
                EditorCommand::Refresh => {
                    let r = (|| -> Result<Vec<(ServerEntity, Option<String>)>, String> {
                        jsonrpc_ping(&url)?;
                        let list = jsonrpc_call(
                            &url,
                            "world.query",
                            Some(json!({
                                "data": {},
                                "filter": {"with": [c::STATE_MACHINE]},
                                "strict": false
                            })),
                        )?;
                        let mut machines = vec![];
                        for row in extract_result_array(list)? {
                            if let Some(id) = row.get("entity").and_then(|e| e.as_u64()) {
                                let comps = jsonrpc_call(
                                    &url,
                                    "world.get_components",
                                    Some(json!({
                                        "entity": id,
                                        "components": [c::NAME]
                                    })),
                                )
                                .ok()
                                .and_then(|v| extract_components_map(v).ok());
                                let name = comps
                                    .and_then(|m| m.get(c::NAME).cloned())
                                    .and_then(|v| v.as_str().map(|s| s.to_string()));
                                machines.push((ServerEntity(id as u64), name));
                            }
                        }
                        Ok(machines)
                    })();
                    EditorEvent::RefreshResult(r)
                }
                EditorCommand::Select { id } => {
                    let r = (|| -> Result<(), String> { jsonrpc_select(&url, Some(id.0 as u32)) })();
                    EditorEvent::SelectResult(r)
                }
                EditorCommand::Save { id } => {
                    let r = (|| -> Result<(), String> { jsonrpc_save_machine(&url, id.0 as u32) })();
                    EditorEvent::SaveResult(r)
                }
                EditorCommand::FetchGraph { id } => {
                    let result = fetch_machine_graph_text(&url, id.0);
                    EditorEvent::GraphResult { id, result }
                }
            }
        });
        pending.tasks.push(task);
    }
}

pub(crate) fn collect_task_results(
    mut pending: ResMut<PendingTasks>,
    mut events_writer: MessageWriter<EditorEvent>,
) {
    let mut i = 0;
    while i < pending.tasks.len() {
        let is_ready = if let Some(evt) = future::block_on(bevy::tasks::futures_lite::future::poll_once(&mut pending.tasks[i])) {
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


