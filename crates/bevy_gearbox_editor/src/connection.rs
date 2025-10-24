use bevy::prelude::Resource;
use crate::rpcs::fetch_machine_graph_text;
use serde_json::{json, Value};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Resource)]
pub struct Connection {
    pub tx: Arc<Mutex<Sender<Command>>>,
    pub rx: Arc<Mutex<Receiver<Event>>>,
}

pub type NetCtx = Connection;

pub fn spawn() -> Connection {
    let (tx_cmd, rx_cmd) = mpsc::channel();
    let (tx_evt, rx_evt) = mpsc::channel();
    start_worker(rx_cmd, tx_evt);
    Connection { tx: Arc::new(Mutex::new(tx_cmd)), rx: Arc::new(Mutex::new(rx_evt)) }
}

use crate::client::{jsonrpc_call, jsonrpc_ping, jsonrpc_save_machine, jsonrpc_select};

pub enum Command {
    Refresh(String),
    Select { url: String, id: u32 },
    Save { url: String, id: u32 },
    FetchGraph { url: String, id: u32 },
}

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

fn start_worker(rx: Receiver<Command>, tx: Sender<Event>) {
    thread::spawn(move || {
        while let Ok(cmd) = rx.recv() {
            match cmd {
                Command::Refresh(url) => {
                    let r = || -> Result<Vec<(u32, Option<String>)>, String> {
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
                    }();
                    let _ = tx.send(Event::RefreshResult(r));
                }
                Command::Select { url, id } => {
                    let r = || -> Result<(), String> { jsonrpc_select(&url, Some(id)) }();
                    let _ = tx.send(Event::SelectResult(r));
                }
                Command::Save { url, id } => {
                    let r = || -> Result<(), String> { jsonrpc_save_machine(&url, id) }();
                    let _ = tx.send(Event::SaveResult(r));
                }
                Command::FetchGraph { url, id } => {
                    let result = fetch_machine_graph_text(&url, id as u64);
                    let _ = tx.send(Event::GraphResult { id, result });
                }
            }
        }
    });
}


