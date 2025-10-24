use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use reqwest::Client;

static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn jsonrpc_call(url: &str, method: &str, params: Option<Value>) -> Result<Value, String> {
    let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let client = Client::new();
    let resp = client
        .post(url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("HTTP: {e}"))?;
    let value: Value = resp.json().await.map_err(|e| format!("Parse: {e}"))?;
    if let Some(err) = value.get("error") {
        let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
        let msg = err.get("message").and_then(|v| v.as_str()).unwrap_or("error");
        return Err(format!("JSON-RPC {code}: {msg}"));
    }
    if let Some(v) = value.get("result").cloned() { Ok(v) } else { Ok(value) }
}

pub(crate) async fn jsonrpc_ping(url: &str) -> Result<(), String> {
    let _ = jsonrpc_call(url, "rpc.discover", None).await?;
    Ok(())
}

pub(crate) async fn jsonrpc_select(url: &str, entity: Option<u32>) -> Result<(), String> {
    let params = match entity { Some(e) => json!({"entity": e}), None => json!({"entity": null}) };
    let _ = jsonrpc_call(url, "editor.select", Some(params)).await?;
    Ok(())
}

pub(crate) async fn jsonrpc_save_machine(url: &str, id: u32) -> Result<(), String> {
    let params = json!({"entity": id});
    let _ = jsonrpc_call(url, "editor.save_machine", Some(params)).await?;
    Ok(())
}


