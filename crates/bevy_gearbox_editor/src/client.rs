use serde_json::{json, Value};

pub fn jsonrpc_call(url: &str, method: &str, params: Option<Value>) -> Result<Value, String> {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let mut resp = ureq::post(url)
        .header("content-type", "application/json")
        .send_json(&req)
        .map_err(|e| format!("HTTP: {e}"))?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("Read: {e}"))?;
    let value: Value = serde_json::from_str(&body).map_err(|e| format!("Parse: {e}"))?;
    if let Some(err) = value.get("error") {
        // Basic JSON-RPC error propagation
        let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
        let msg = err.get("message").and_then(|v| v.as_str()).unwrap_or("error");
        return Err(format!("JSON-RPC {code}: {msg}"));
    }
    if let Some(v) = value.get("result").cloned() { Ok(v) } else { Ok(value) }
}

pub fn jsonrpc_ping(url: &str) -> Result<(), String> {
    let _ = jsonrpc_call(url, "rpc.discover", None)?;
    Ok(())
}

pub fn jsonrpc_select(url: &str, entity: Option<u32>) -> Result<(), String> {
    let params = match entity { Some(e) => json!({"entity": e}), None => json!({"entity": null}) };
    let _ = jsonrpc_call(url, "editor.select", Some(params))?;
    Ok(())
}

pub fn jsonrpc_save_machine(url: &str, id: u32) -> Result<(), String> {
    let params = json!({"entity": id});
    let _ = jsonrpc_call(url, "editor.save_machine", Some(params))?;
    Ok(())
}


