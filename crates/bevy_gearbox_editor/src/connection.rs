use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, IoTaskPool, Task};
use crate::rpcs::fetch_machine_graph_text;
use crate::client::{jsonrpc_call, jsonrpc_ping, jsonrpc_save_machine, jsonrpc_select};
use serde_json::{json, Value};

#[derive(Message, Clone)]
pub enum Command {
    Refresh(String),
    Select { url: String, id: u32 },
    Save { url: String, id: u32 },
    FetchGraph { url: String, id: u32 },
}

#[derive(Message)]
pub enum Event {
    RefreshResult(Result<Vec<(u32, Option<String>)>, String>),
    SelectResult(Result<(), String>),
    SaveResult(Result<(), String>),
    GraphResult { id: u32, result: Result<String, String> },
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

fn extract_components_map(v: Value) -> Result<serde_json::Map<String, Value>, String> {
    match v {
        Value::Object(o) => {
            if let Some(Value::Object(components)) = o.get("components") { Ok(components.clone()) } else { Ok(o) }
        }
        Value::Array(_) => Err("unexpected array for components response".to_string()),
        other => {
            if let Some(obj) = other.get("result") {
                if let Value::Object(o) = obj {
                    if let Some(Value::Object(components)) = o.get("components") { return Ok(components.clone()); }
                    return Ok(o.clone());
                }
            }
            Err(format!("expected object or result object, got {}", other))
        }
    }
}

#[derive(Resource, Default)]
pub struct PendingTasks {
    pub tasks: Vec<Task<Event>>,
}

pub fn handle_commands(
    mut commands_reader: MessageReader<Command>,
    mut pending: ResMut<PendingTasks>,
) {
    let pool = IoTaskPool::get();
    for cmd in commands_reader.read().cloned() {
        let task = pool.spawn(async move {
            match cmd {
                Command::Refresh(url) => {
                    let r = (|| -> Result<Vec<(u32, Option<String>)>, String> {
                        jsonrpc_ping(&url)?;
                        let list = jsonrpc_call(
                            &url,
                            "world.query",
                            Some(json!({
                                "data": {},
                                "filter": {"with": ["bevy_gearbox_core::StateMachine"]},
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
                                        "components": ["bevy_ecs::name::Name"]
                                    })),
                                )
                                .ok()
                                .and_then(|v| extract_components_map(v).ok());
                                let name = comps
                                    .and_then(|m| m.get("bevy_ecs::name::Name").cloned())
                                    .and_then(|v| v.as_str().map(|s| s.to_string()));
                                machines.push((id as u32, name));
                            }
                        }
                        Ok(machines)
                    })();
                    Event::RefreshResult(r)
                }
                Command::Select { url, id } => {
                    let r = (|| -> Result<(), String> { jsonrpc_select(&url, Some(id)) })();
                    Event::SelectResult(r)
                }
                Command::Save { url, id } => {
                    let r = (|| -> Result<(), String> { jsonrpc_save_machine(&url, id) })();
                    Event::SaveResult(r)
                }
                Command::FetchGraph { url, id } => {
                    let result = fetch_machine_graph_text(&url, id as u64);
                    Event::GraphResult { id, result }
                }
            }
        });
        pending.tasks.push(task);
    }
}

pub fn collect_task_results(
    mut pending: ResMut<PendingTasks>,
    mut events_writer: MessageWriter<Event>,
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


