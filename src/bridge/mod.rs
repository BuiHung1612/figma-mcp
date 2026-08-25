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
}

pub async fn start_bridge_server(port: u16) -> Result<(BridgeState, u16), String> {
    for attempt in 0..PORT_RANGE {
        let current_port = port + attempt;
        let addr_v4 = SocketAddr::from(([0, 0, 0, 0], current_port));
        let addr_v6 = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], current_port));

        match TcpListener::bind(addr_v4).await {
            Ok(listener_v4) => {
                let state = BridgeState::new(current_port);
                let app = server::create_router(state.clone());

                let app_v4 = app.clone();
                tokio::spawn(async move {
                    if let Err(e) = axum::serve(listener_v4, app_v4).await {
                        eprintln!("[figma-mcp bridge] Server error: {}", e);
                    }
                });

                // Also bind IPv6 (localhost / ::1) so Figma desktop on macOS connects seamlessly
                if let Ok(listener_v6) = TcpListener::bind(addr_v6).await {
                    let app_v6 = app.clone();
                    tokio::spawn(async move {
                        let _ = axum::serve(listener_v6, app_v6).await;
                    });
                }

                return Ok((state, current_port));
            }
            Err(_) => {
                eprintln!(
                    "[figma-mcp] Port {} in use — trying {}...",
                    current_port,
                    current_port + 1
                );
            }
        }
    }

    Err(format!("All ports {}-{} in use", port, port + PORT_RANGE - 1))
}
