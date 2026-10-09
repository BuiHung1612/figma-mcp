pub mod bench;
pub mod http;
pub mod index;
pub mod proxy;
pub mod server;
pub mod session;

use serde_json::Value;
use std::net::SocketAddr;
use tokio::net::TcpListener;

pub use proxy::HttpProxy;
pub use server::{BridgeState, DEFAULT_PORT, PORT_RANGE};
pub use session::SessionInfo;

#[derive(Clone)]
pub enum BridgeHandle {
    Direct(BridgeState),
    Proxy(HttpProxy),
}

impl BridgeHandle {
    pub async fn is_plugin_connected(&self, session_id: Option<&str>) -> bool {
        match self {
            BridgeHandle::Direct(b) => b.is_plugin_connected(session_id).await,
            BridgeHandle::Proxy(p) => p.check_health().await.plugin_connected,
        }
    }

    pub async fn send_operation(&self, operation: &str, params: Value, session_id: Option<&str>) -> Result<Value, String> {
        match self {
            BridgeHandle::Direct(b) => b.send_operation(operation, params, session_id).await,
            BridgeHandle::Proxy(p) => p.send_operation(operation, params, session_id).await,
        }
    }

    pub async fn get_port(&self) -> u16 {
        match self {
            BridgeHandle::Direct(b) => b.inner.lock().await.port,
            BridgeHandle::Proxy(p) => p.port,
        }
    }

    pub async fn get_sessions(&self) -> Vec<SessionInfo> {
        match self {
            BridgeHandle::Direct(b) => b.get_sessions().await,
            BridgeHandle::Proxy(p) => {
                let health = p.check_health().await;
                if let Some(sessions_val) = health.sessions {
                    serde_json::from_value(Value::Array(sessions_val)).unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
        }
    }

    pub async fn get_queue_length(&self) -> usize {
        match self {
            BridgeHandle::Direct(b) => b.get_queue_length().await,
            BridgeHandle::Proxy(p) => p.check_health().await.queue_length,
        }
    }

    pub async fn get_last_poll_at(&self) -> u64 {
        match self {
            BridgeHandle::Direct(b) => b.get_last_poll_at().await,
            BridgeHandle::Proxy(_) => 0,
        }
    }

    pub async fn get_stats(&self) -> Option<Value> {
        match self {
            BridgeHandle::Direct(b) => {
                let inner = b.inner.lock().await;
                Some(serde_json::json!({
                    "ops": inner.global_stats.ops,
                    "avgLatencyMs": inner.global_stats.avg_latency_ms,
                    "sessions": inner.sessions.len()
                }))
            }
            BridgeHandle::Proxy(p) => p.check_health().await.stats,
        }
    }

    pub async fn get_index_stats(&self, session_id: Option<&str>) -> Option<crate::bridge::index::IndexStats> {
        match self {
            BridgeHandle::Direct(b) => b.get_index_stats(session_id).await,
            BridgeHandle::Proxy(_) => None,
        }
    }

    pub async fn get_index_node(&self, session_id: Option<&str>, node_id: &str) -> Option<crate::bridge::index::IndexNode> {
        match self {
            BridgeHandle::Direct(b) => b.get_index_node(session_id, node_id).await,
            BridgeHandle::Proxy(_) => None,
        }
    }

    pub async fn search_index_nodes(
        &self,
        session_id: Option<&str>,
        query: &str,
        node_type: Option<&str>,
        limit: usize,
    ) -> Option<Vec<crate::bridge::index::IndexNode>> {
        match self {
            BridgeHandle::Direct(b) => b.search_index_nodes(session_id, query, node_type, limit).await,
            BridgeHandle::Proxy(_) => None,
        }
    }

    pub async fn search_index_components(
        &self,
        session_id: Option<&str>,
        name: &str,
        limit: usize,
    ) -> Option<Vec<crate::bridge::index::IndexComponent>> {
        match self {
            BridgeHandle::Direct(b) => b.search_index_components(session_id, name, limit).await,
            BridgeHandle::Proxy(_) => None,
        }
    }

    pub async fn search_index_styles(
        &self,
        session_id: Option<&str>,
        name: &str,
        style_type: Option<&str>,
    ) -> Option<Vec<crate::bridge::index::IndexStyle>> {
        match self {
            BridgeHandle::Direct(b) => b.search_index_styles(session_id, name, style_type).await,
            BridgeHandle::Proxy(_) => None,
        }
    }

    pub async fn search_index_variables(
        &self,
        session_id: Option<&str>,
        name: &str,
        collection: Option<&str>,
    ) -> Option<Vec<crate::bridge::index::IndexVariable>> {
        match self {
            BridgeHandle::Direct(b) => b.search_index_variables(session_id, name, collection).await,
            BridgeHandle::Proxy(_) => None,
        }
    }
}

/// Who answers on a port in the bridge range.
enum PortProbe {
    Free,
    /// A figma-rust-mcp with this exact version: reuse it instead of starting a second bridge.
    Same,
    /// Anything else (another app, or an older/newer figma-rust-mcp): skip to the next port.
    Taken(String),
}

async fn probe_port(client: &reqwest::Client, port: u16) -> PortProbe {
    let url = format!("http://127.0.0.1:{port}/plugin/version");
    match client.get(&url).timeout(std::time::Duration::from_millis(500)).send().await {
        Err(e) if e.is_connect() => PortProbe::Free,
        // Windows retries SYN on a refused localhost port and only fails after ~2s, so a
        // free port times out here. Let the bind decide: it fails if the port is really held.
        Err(e) if e.is_timeout() => PortProbe::Free,
        Err(_) => PortProbe::Taken("a process that does not answer HTTP".into()),
        Ok(res) => match res.json::<Value>().await {
            Ok(v) if v["name"] == "figma-rust-mcp" && v["version"] == env!("CARGO_PKG_VERSION") => PortProbe::Same,
            Ok(v) if v["name"] == "figma-rust-mcp" => PortProbe::Taken(format!("figma-rust-mcp v{}", v["version"].as_str().unwrap_or("?"))),
            _ => PortProbe::Taken("another app or a pre-4.0 figma-rust-mcp".into()),
        },
    }
}

pub enum BridgeStart {
    /// A same-version server already runs on this port.
    Attached(u16),
    Started(BridgeState, u16),
}

/// Scan `base..base+PORT_RANGE`: reuse a same-version server, skip ports held by
/// anything else, and start on the first free one. The plugin scans the same range.
pub async fn connect_or_start(base: u16) -> Result<BridgeStart, String> {
    let client = reqwest::Client::new();
    let last = base.saturating_add(PORT_RANGE - 1);
    for port in base..=last {
        match probe_port(&client, port).await {
            PortProbe::Same => return Ok(BridgeStart::Attached(port)),
            PortProbe::Taken(who) => eprintln!("[figma-rust-mcp] Port {port} is held by {who}; trying the next port"),
            PortProbe::Free => match bind_bridge_server(port).await {
                Ok(state) => return Ok(BridgeStart::Started(state, port)),
                Err(e) => eprintln!("[figma-rust-mcp] Port {port}: {e}; trying the next port"),
            },
        }
    }
    Err(format!("No usable port in {base}-{last}"))
}

async fn bind_bridge_server(port: u16) -> Result<BridgeState, String> {
    let addr_v4 = SocketAddr::from(([0, 0, 0, 0], port));
    let addr_v6 = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port));
    let listener_v4 = TcpListener::bind(addr_v4).await.map_err(|e| e.to_string())?;
    // `localhost` may resolve to ::1 first; if another process holds it there, the
    // plugin would reach that process instead, so treat the port as taken.
    let listener_v6 = match TcpListener::bind(addr_v6).await {
        Ok(l) => Some(l),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => return Err(format!("[::1]:{port} is in use")),
        Err(_) => None, // IPv6 unavailable on this machine
    };
    bench::mark_ready();
    let state = BridgeState::new(port);
    let app = http::create_router(state.clone());

    let app_v4 = app.clone();
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener_v4, app_v4).await {
            eprintln!("[figma-rust-mcp bridge] Server error: {}", e);
        }
    });

    if let Some(listener_v6) = listener_v6 {
        tokio::spawn(async move {
            let _ = axum::serve(listener_v6, app).await;
        });
    }
    Ok(state)
}

#[cfg(test)]
mod port_tests {
    use super::*;

    #[tokio::test]
    async fn skips_a_different_version_server_and_starts_on_next_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = listener.local_addr().unwrap().port();
        let old = axum::Router::new().route("/plugin/version", axum::routing::get(|| async {
            axum::Json(serde_json::json!({"name": "figma-rust-mcp", "version": "0.0.1"}))
        }));
        tokio::spawn(async move { axum::serve(listener, old).await.unwrap() });

        match connect_or_start(base).await.unwrap() {
            BridgeStart::Started(_, port) => assert!(port > base),
            BridgeStart::Attached(port) => panic!("attached to old server on {port}"),
        }
    }
}
