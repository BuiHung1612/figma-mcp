//! In-process benchmark behind `POST /benchmark`, used by the plugin "Bench" tab and `npm run bench`.

use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Instant;

use super::{BridgeHandle, BridgeState};
use crate::mcp::protocol::JsonRpcRequest;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();
static READY_MS: OnceLock<f64> = OnceLock::new();

/// Call first thing in `main` so startup is measured from process entry.
pub fn mark_process_start() { PROCESS_START.get_or_init(Instant::now); }

/// Call once the HTTP listener is bound.
pub fn mark_ready() {
    let start = *PROCESS_START.get_or_init(Instant::now);
    READY_MS.get_or_init(|| start.elapsed().as_secs_f64() * 1000.0);
}

/// p50/p95/max of samples in milliseconds.
fn summarize(mut ms: Vec<f64>) -> Value {
    if ms.is_empty() { return Value::Null; }
    ms.sort_by(|a, b| a.total_cmp(b));
    let pick = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
    json!({ "n": ms.len(), "p50Ms": pick(0.5), "p95Ms": pick(0.95), "maxMs": ms[ms.len() - 1] })
}

async fn sample<F, Fut>(n: usize, mut f: F) -> Value
where F: FnMut() -> Fut, Fut: std::future::Future<Output = bool> {
    let mut ms = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        if !f().await { break; }
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    summarize(ms)
}

pub async fn run(state: &BridgeState) -> Value {
    let handle = BridgeHandle::Direct(state.clone());

    // MCP dispatch cost without network: what every agent call pays inside the server.
    let mcp = sample(200, || {
        let handle = handle.clone();
        async move {
            let req = JsonRpcRequest { jsonrpc: "2.0".into(), id: Some(json!(1)), method: "tools/list".into(), params: None };
            crate::mcp::server::handle_jsonrpc_request(handle, req).await.is_some()
        }
    }).await;

    let stats = state.get_index_stats(None).await;
    let index_nodes = stats.as_ref().map(|s| s.total_nodes);
    let search = if index_nodes.is_some() {
        sample(200, || async { state.search_index_nodes(None, "a", None, 20).await.is_some() }).await
    } else { Value::Null };

    // Real round trip through the plugin; skipped when Figma is not connected.
    let plugin = if state.is_plugin_connected(None).await {
        sample(10, || async { state.send_operation("get_viewport", json!({}), None).await.is_ok() }).await
    } else { Value::Null };

    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "startupMs": READY_MS.get(),
        "uptimeS": PROCESS_START.get().map(|s| s.elapsed().as_secs()),
        "memoryMb": super::http::get_process_memory_mb(),
        "mcpToolsList": mcp,
        "indexNodes": index_nodes,
        "indexSearch": search,
        "pluginRoundTrip": plugin,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn percentiles_use_sorted_samples() {
        let s = super::summarize(vec![5.0, 1.0, 3.0, 2.0, 4.0]);
        assert_eq!(s["p50Ms"], 3.0);
        assert_eq!(s["p95Ms"], 5.0);
        assert_eq!(s["n"], 5);
    }

    #[tokio::test]
    async fn run_reports_mcp_latency_without_plugin() {
        let r = super::run(&super::BridgeState::new(0)).await;
        assert_eq!(r["mcpToolsList"]["n"], 200);
        assert!(r["pluginRoundTrip"].is_null());
    }
}
