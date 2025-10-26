use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use crate::client::{jsonrpc_call, jsonrpc_ping};
use crate::component as c;
use crate::model::{ComponentBag, ComponentEntry, Edge, EntityId, StateMachineGraph, StateNode};
use crate::types::ServerEntity;

pub(crate) fn extract_components_map(v: Value) -> Result<serde_json::Map<String, Value>, String> {
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

pub(crate) fn extract_result_array(v: Value) -> Result<Vec<Value>, String> {
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

pub(crate) async fn list_state_machines(url: &str) -> Result<Vec<(u64, Option<String>)>, String> {
    jsonrpc_ping(url).await?;
    let list = jsonrpc_call(
        url,
        "world.query",
        Some(json!({
            "data": {},
            "filter": {"with": [c::STATE_MACHINE]},
            "strict": false
        })),
    ).await?;
    let mut machines = vec![];
    for row in extract_result_array(list)? {
        if let Some(id) = row.get("entity").and_then(|e| e.as_u64()) {
            let comps = jsonrpc_call(
                url,
                "world.get_components",
                Some(json!({
                    "entity": id,
                    "components": [c::NAME]
                })),
            ).await
            .ok()
            .and_then(|v| extract_components_map(v).ok());
            let name = comps
                .and_then(|m| m.get(c::NAME).cloned())
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            machines.push((id as u64, name));
        }
    }
    Ok(machines)
}

pub(crate) async fn get_components(url: &str, entity: u64, components: Option<&[&str]>) -> Result<HashMap<String, Value>, String> {
    let params = match components {
        Some(list) => json!({"entity": entity, "components": list}),
        None => json!({"entity": entity}),
    };
    let v = jsonrpc_call(url, "world.get_components", Some(params)).await?;
    let map = extract_components_map(v)?;
    Ok(map.into_iter().collect())
}

fn parse_entity_list(value: &Value) -> Vec<u64> {
    match value {
        Value::Array(arr) => arr.iter().filter_map(|v| parse_single_entity(v)).collect(),
        Value::Object(obj) => obj
            .get("entities")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| parse_single_entity(v)).collect())
            .unwrap_or_default(),
        _ => vec![],
    }
}

fn parse_single_entity(value: &Value) -> Option<u64> {
    // Direct number
    if let Some(id) = value.as_u64() { return Some(id); }
    // String encodings like "123" or "Entity(123)" or other text containing digits
    if let Some(s) = value.as_str() {
        // collect the first contiguous run of digits
        let mut digits = String::new();
        let mut in_run = false;
        for ch in s.chars() {
            if ch.is_ascii_digit() {
                digits.push(ch);
                in_run = true;
            } else if in_run {
                break;
            }
        }
        if !digits.is_empty() {
            if let Ok(n) = digits.parse::<u64>() { return Some(n); }
        }
    }
    match value {
        Value::Array(arr) => arr.get(0).and_then(|v| v.as_u64()).or_else(|| arr.get(0).and_then(|v| parse_single_entity(v))),
        Value::Object(obj) => {
            // Common shapes
            if let Some(id) = obj.get("entity").and_then(|v| v.as_u64()) { return Some(id); }
            if let Some(Value::Number(n)) = obj.get("0") { return n.as_u64(); }
            // Try any nested values
            for v in obj.values() {
                if let Some(id) = parse_single_entity(v) { return Some(id); }
            }
            None
        }
        _ => None,
    }
}

async fn get_name(url: &str, cache: &mut HashMap<u64, String>, entity: u64) -> Result<String, String> {
    if let Some(n) = cache.get(&entity) { return Ok(n.clone()); }
    let comps = get_components(url, entity, Some(&[c::NAME])).await?;
    let name = comps
        .get(c::NAME)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    cache.insert(entity, name.clone());
    Ok(name)
}

fn choose_edge_label(components: &HashMap<String, Value>) -> String {
    // 1) Prefer explicit Name text if present
    if let Some(name_val) = components.get(c::NAME) {
        let maybe_name = name_val.as_str().map(|s| s.trim().to_string()).or_else(|| {
            if let Value::Object(obj) = name_val {
                for v in obj.values() {
                    if let Some(s) = v.as_str() { return Some(s.trim().to_string()); }
                }
            }
            None
        });
        if let Some(name) = maybe_name { if !name.is_empty() { return name; } }
    }

    // 2) Otherwise, prefer EventEdge<T> → use inner T (simple name)
    let keys: HashSet<String> = components.keys().cloned().collect();
    let mut event_edge_types: Vec<&String> = keys.iter().filter(|s| s.contains(c::EVENT_EDGE_SUBSTR)) .collect();
    event_edge_types.sort();
    if let Some(ty) = event_edge_types.first() {
        let s = ty.as_str();
        if let (Some(start), Some(end)) = (s.find('<'), s.rfind('>')) {
            if end > start + 1 {
                let inner = &s[start + 1..end];
                if let Some(simple) = inner.rsplit("::").next() { return simple.to_string(); }
                return inner.to_string();
            }
        }
        if let Some(simple) = s.rsplit("::").next() { return simple.to_string(); }
        return (*ty).clone();
    }

    // 3) Else, if AlwaysEdge present, use "Always"
    if keys.contains(c::ALWAYS_EDGE) { return "Always".to_string(); }

    // Fallback
    "Edge".to_string()
}

pub(crate) async fn fetch_machine_graph_text(url: &str, machine: u64) -> Result<String, String> {
    let mut names: HashMap<u64, String> = HashMap::new();
    let mut states: Vec<u64> = Vec::new();
    let mut stack: Vec<u64> = Vec::new();
    let mut visited: HashSet<u64> = HashSet::new();

    let root_comps = get_components(url, machine, Some(&[c::STATE_CHILDREN, c::NAME, c::PARALLEL])).await?;
    if let Some(value) = root_comps.get(c::STATE_CHILDREN) {
        for child in parse_entity_list(value) { stack.push(child); }
    }
    if let Some(root_name) = root_comps.get(c::NAME).and_then(|v| v.as_str()) {
        names.insert(machine, root_name.to_string());
    }

    while let Some(entity) = stack.pop() {
        if !visited.insert(entity) { continue; }
        states.push(entity);
        let comps = get_components(url, entity, Some(&[c::STATE_CHILDREN, c::NAME])).await?;
        if let Some(n) = comps.get(c::NAME).and_then(|v| v.as_str()) {
            names.insert(entity, n.to_string());
        }
        if let Some(children) = comps.get(c::STATE_CHILDREN) {
            for child in parse_entity_list(children) { stack.push(child); }
        }
    }

    let mut edges_formatted: Vec<String> = Vec::new();
    for state in &states {
        let comps = get_components(url, *state, Some(&[c::TRANSITIONS])).await?;
        let Some(transitions_val) = comps.get(c::TRANSITIONS) else { continue; };
        let edge_entities = parse_entity_list(transitions_val);
        for edge in edge_entities {
            let all = get_components(
                url,
                edge,
                Some(&[
                    c::TARGET,
                    c::ALWAYS_EDGE,
                    c::AFTER,
                    c::NAME,
                ]),
            ).await?;
            let target_id = all
                .get(c::TARGET)
                .and_then(parse_single_entity)
                .unwrap_or(0);
            let source_name = get_name(url, &mut names, *state).await?;
            let mut target_name = if target_id != 0 { get_name(url, &mut names, target_id).await? } else { String::new() };
            if target_name.is_empty() {
                if let Some(Value::String(edge_name)) = all.get(c::NAME) {
                    if let Some(arrow) = edge_name.find("->") {
                        let rhs = edge_name[arrow+2..].trim();
                        let rhs = if let Some(paren) = rhs.find('(') { &rhs[..paren] } else { rhs };
                        let rhs = rhs.trim();
                        if !rhs.is_empty() { target_name = rhs.to_string(); }
                    }
                }
            }
            let label = choose_edge_label(&all);
            edges_formatted.push(format!("{} - {} -> {}", source_name, label, target_name));
        }
    }

    let mut out = String::new();
    let header = names.get(&machine).cloned().unwrap_or_default();
    out.push_str(&header);
    out.push('\n');
    for s in &states {
        let name = names.get(s).cloned().unwrap_or_default();
        out.push_str("   ");
        out.push_str(&name);
        out.push('\n');
    }
    out.push('\n');
    for line in edges_formatted {
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}


pub(crate) async fn fetch_machine_graph_model(url: &str, machine: u64) -> Result<StateMachineGraph, String> {
    // Build root node
    let root_comps = get_components(url, machine, Some(&[c::STATE_CHILDREN, c::NAME, c::PARALLEL, c::INITIAL_STATE])).await?;
    let root_id = EntityId::Server(ServerEntity(machine));
    let mut root_node = StateNode::new(root_id);
    // Fill components bag for root
    let mut root_bag = ComponentBag::default();
    if let Some(v) = root_comps.get(c::NAME).cloned() { root_bag.insert(ComponentEntry::new(c::NAME.to_string(), v)); }
    if let Some(v) = root_comps.get(c::STATE_CHILDREN).cloned() { root_bag.insert(ComponentEntry::new(c::STATE_CHILDREN.to_string(), v)); }
    if let Some(v) = root_comps.get(c::PARALLEL).cloned() { root_bag.insert(ComponentEntry::new(c::PARALLEL.to_string(), v)); }
    if let Some(v) = root_comps.get(c::INITIAL_STATE).cloned() { root_bag.insert(ComponentEntry::new(c::INITIAL_STATE.to_string(), v)); }
    root_node.components = root_bag;
    if let Some(n) = root_comps.get(c::NAME).and_then(|v| v.as_str()) { root_node.display_name = Some(n.to_string()); }

    let mut graph = StateMachineGraph::new(root_node);

    // Traverse children and build nodes
    let mut stack: Vec<(EntityId, u64)> = Vec::new();
    if let Some(value) = root_comps.get(c::STATE_CHILDREN) {
        for child in parse_entity_list(value) {
            stack.push((root_id, child));
        }
    }
    while let Some((parent_id, entity)) = stack.pop() {
        let id = EntityId::Server(ServerEntity(entity));
        if graph.nodes.contains_key(&id) { continue; }
        let comps = get_components(url, entity, Some(&[c::STATE_CHILDREN, c::NAME, c::PARALLEL, c::INITIAL_STATE])).await?;
        let mut node = StateNode::new(id);
        node.parent = Some(parent_id);
        // Components bag
        let mut bag = ComponentBag::default();
        if let Some(v) = comps.get(c::NAME).cloned() { bag.insert(ComponentEntry::new(c::NAME.to_string(), v)); }
        if let Some(v) = comps.get(c::STATE_CHILDREN).cloned() { bag.insert(ComponentEntry::new(c::STATE_CHILDREN.to_string(), v)); }
        if let Some(v) = comps.get(c::PARALLEL).cloned() { bag.insert(ComponentEntry::new(c::PARALLEL.to_string(), v)); }
        if let Some(v) = comps.get(c::INITIAL_STATE).cloned() { bag.insert(ComponentEntry::new(c::INITIAL_STATE.to_string(), v)); }
        node.components = bag;
        if let Some(n) = comps.get(c::NAME).and_then(|v| v.as_str()) { node.display_name = Some(n.to_string()); }
        // Children
        if let Some(children) = comps.get(c::STATE_CHILDREN) {
            let child_ids = parse_entity_list(children);
            for child in &child_ids { stack.push((id, *child)); }
            node.children = child_ids.into_iter().map(|e| EntityId::Server(ServerEntity(e))).collect();
        }
        // Insert and update parent's child list (ensure parent exists)
        graph.nodes.insert(id, node);
        if let Some(parent) = graph.nodes.get_mut(&parent_id) { if !parent.children.contains(&id) { parent.children.push(id); } }
    }

    // Build edges by scanning transitions of each node
    for node_id in graph.nodes.keys().cloned().collect::<Vec<_>>() {
        let entity = match node_id { EntityId::Server(ServerEntity(e)) => e, _ => continue };
        let comps = get_components(url, entity, Some(&[c::TRANSITIONS])).await?;
        let Some(transitions_val) = comps.get(c::TRANSITIONS) else { continue; };
        let edge_entities = parse_entity_list(transitions_val);
        for edge_e in edge_entities {
            let all = get_components(url, edge_e, Some(&[c::TARGET, c::ALWAYS_EDGE, c::AFTER, c::NAME])).await?;
            // Build edge bag
            let mut bag = ComponentBag::default();
            if let Some(v) = all.get(c::TARGET).cloned() { bag.insert(ComponentEntry::new(c::TARGET.to_string(), v)); }
            if let Some(v) = all.get(c::ALWAYS_EDGE).cloned() { bag.insert(ComponentEntry::new(c::ALWAYS_EDGE.to_string(), v)); }
            if let Some(v) = all.get(c::AFTER).cloned() { bag.insert(ComponentEntry::new(c::AFTER.to_string(), v)); }
            if let Some(v) = all.get(c::NAME).cloned() { bag.insert(ComponentEntry::new(c::NAME.to_string(), v)); }

            let target_id_u64 = all.get(c::TARGET).and_then(|v| parse_single_entity(v)).unwrap_or(0);
            let target_id = EntityId::Server(ServerEntity(target_id_u64));
            // Ensure target exists at least as a placeholder
            graph.nodes.entry(target_id).or_insert_with(|| StateNode::new(target_id));

            let e_id = EntityId::Server(ServerEntity(edge_e));
            let display_label = Some(choose_edge_label(&all));
            let edge = Edge { id: e_id, source: node_id, target: target_id, components: bag, display_label, dirty: Default::default(), server_version: None };
            graph.adjacency_out.entry(node_id).or_default().push(e_id);
            graph.adjacency_in.entry(target_id).or_default().push(e_id);
            graph.edges.insert(e_id, edge);
        }
    }

    Ok(graph)
}

// Transport helpers for server-exposed trackers

pub(crate) async fn fetch_active_states(url: &str, machine: u64) -> Result<(Vec<u64>, Vec<u64>), String> {
    let comps = get_components(url, machine, Some(&[c::ACTIVE_TRACKER])).await?;
    let Some(Value::Object(tracker)) = comps.get(c::ACTIVE_TRACKER) else { return Ok((Vec::new(), Vec::new())); };
    let active = tracker.get("active").map(parse_entity_list).unwrap_or_default();
    let leaves = tracker.get("leaves").map(parse_entity_list).unwrap_or_default();
    Ok((active, leaves))
}

#[derive(Debug, Clone, Default)]
pub struct TransitionFeedItem {
    pub seq: u64,
    pub machine: Option<u64>,
    pub source: Option<u64>,
    pub edge: Option<u64>,
    pub target: Option<u64>,
    pub kind: Option<String>,
}

pub(crate) async fn fetch_transition_feed(url: &str, machine: u64) -> Result<Option<TransitionFeedItem>, String> {
    let comps = get_components(url, machine, Some(&[c::TRANSITION_FEED])).await?;
    let Some(Value::Object(feed)) = comps.get(c::TRANSITION_FEED) else { return Ok(None); };
    let seq = feed.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
    let Some(Value::Object(last)) = feed.get("last") else { return Ok(None); };
    let machine_v = last.get("machine").and_then(parse_single_entity);
    let source_v = last.get("source").and_then(parse_single_entity);
    let edge_v = last.get("edge").and_then(parse_single_entity);
    let target_v = last.get("target").and_then(parse_single_entity);
    let kind_v = match last.get("kind") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Object(o)) => o.get("variant").and_then(|v| v.as_str()).map(|s| s.to_string()).or_else(|| o.get("name").and_then(|v| v.as_str()).map(|s| s.to_string())),
        _ => None,
    };
    Ok(Some(TransitionFeedItem { seq, machine: machine_v, source: source_v, edge: edge_v, target: target_v, kind: kind_v }))
}


// =========================
// File save helpers (client-side RPC calls)
// =========================
pub(crate) async fn save_graph(url: &str, entity: u64, asset_path_no_ext_or_full: &str) -> Result<(), String> {
    // Ensure .scn.ron extension on the logical asset path
    let path = if asset_path_no_ext_or_full.ends_with(".scn.ron") {
        asset_path_no_ext_or_full.to_string()
    } else {
        format!("{}.scn.ron", asset_path_no_ext_or_full)
    };
    let params = json!({"entity": entity, "path": path});
    let _ = jsonrpc_call(url, "editor.save_graph", Some(params)).await?;
    Ok(())
}

