use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use crate::client::jsonrpc_call;

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

pub(crate) fn get_components(url: &str, entity: u64, components: Option<&[&str]>) -> Result<HashMap<String, Value>, String> {
    let params = match components {
        Some(list) => json!({"entity": entity, "components": list}),
        None => json!({"entity": entity}),
    };
    let v = jsonrpc_call(url, "world.get_components", Some(params))?;
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

fn get_name(url: &str, cache: &mut HashMap<u64, String>, entity: u64) -> Result<String, String> {
    if let Some(n) = cache.get(&entity) { return Ok(n.clone()); }
    let comps = get_components(url, entity, Some(&["bevy_ecs::name::Name"]))?;
    let name = comps
        .get("bevy_ecs::name::Name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    cache.insert(entity, name.clone());
    Ok(name)
}

fn choose_edge_label(components: &HashMap<String, Value>) -> String {
    let keys: HashSet<String> = components.keys().cloned().collect();
    if keys.contains("bevy_gearbox_core::transitions::AlwaysEdge") { return "Always".to_string(); }
    if keys.contains("bevy_gearbox_core::transitions::After") { return "After".to_string(); }

    // Prefer generic EventEdge types
    let mut event_edge_types: Vec<&String> = keys.iter().filter(|s| s.contains("EventEdge")) .collect();
    event_edge_types.sort();
    if let Some(ty) = event_edge_types.first() {
        let s = ty.as_str();
        if let (Some(start), Some(end)) = (s.find('<'), s.rfind('>')) {
            if end > start + 1 {
                let inner = &s[start + 1..end];
                if let Some(simple) = inner.rsplit("::").next() {
                    return simple.to_string();
                }
                return inner.to_string();
            }
        }
        if let Some(simple) = s.rsplit("::").next() { return simple.to_string(); }
        return (*ty).clone();
    }

    // Fallback: try to extract a label from the edge's Name like "... (OnComplete)"
    if let Some(name_val) = components.get("bevy_ecs::name::Name") {
        let maybe_name = name_val.as_str().map(|s| s.to_string()).or_else(|| {
            if let Value::Object(obj) = name_val {
                for v in obj.values() {
                    if let Some(s) = v.as_str() { return Some(s.to_string()); }
                }
            }
            None
        });
        if let Some(name) = maybe_name {
            if let (Some(l), Some(r)) = (name.rfind('('), name.rfind(')')) {
                if r > l + 1 {
                    return name[l+1..r].to_string();
                }
            }
        }
    }

    "Edge".to_string()
}

pub fn fetch_machine_graph_text(url: &str, machine: u64) -> Result<String, String> {
    let mut names: HashMap<u64, String> = HashMap::new();
    let mut states: Vec<u64> = Vec::new();
    let mut stack: Vec<u64> = Vec::new();
    let mut visited: HashSet<u64> = HashSet::new();

    let root_comps = get_components(url, machine, Some(&["bevy_gearbox_core::StateChildren", "bevy_ecs::name::Name"]))?;
    if let Some(value) = root_comps.get("bevy_gearbox_core::StateChildren") {
        for child in parse_entity_list(value) { stack.push(child); }
    }
    if let Some(root_name) = root_comps.get("bevy_ecs::name::Name").and_then(|v| v.as_str()) {
        names.insert(machine, root_name.to_string());
    }

    while let Some(entity) = stack.pop() {
        if !visited.insert(entity) { continue; }
        states.push(entity);
        let comps = get_components(url, entity, Some(&["bevy_gearbox_core::StateChildren", "bevy_ecs::name::Name"]))?;
        if let Some(n) = comps.get("bevy_ecs::name::Name").and_then(|v| v.as_str()) {
            names.insert(entity, n.to_string());
        }
        if let Some(children) = comps.get("bevy_gearbox_core::StateChildren") {
            for child in parse_entity_list(children) { stack.push(child); }
        }
    }

    let mut edges_formatted: Vec<String> = Vec::new();
    for state in &states {
        let comps = get_components(url, *state, Some(&["bevy_gearbox_core::transitions::Transitions"]))?;
        let Some(transitions_val) = comps.get("bevy_gearbox_core::transitions::Transitions") else { continue; };
        let edge_entities = parse_entity_list(transitions_val);
        for edge in edge_entities {
            let all = get_components(
                url,
                edge,
                Some(&[
                    "bevy_gearbox_core::transitions::Target",
                    "bevy_gearbox_core::transitions::AlwaysEdge",
                    "bevy_gearbox_core::transitions::After",
                    "bevy_ecs::name::Name",
                ]),
            )?;
            let target_id = all
                .get("bevy_gearbox_core::transitions::Target")
                .and_then(parse_single_entity)
                .unwrap_or(0);
            let source_name = get_name(url, &mut names, *state)?;
            let mut target_name = if target_id != 0 { get_name(url, &mut names, target_id)? } else { String::new() };
            if target_name.is_empty() {
                if let Some(Value::String(edge_name)) = all.get("bevy_ecs::name::Name") {
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


