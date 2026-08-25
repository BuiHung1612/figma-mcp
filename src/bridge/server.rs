use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json,
    },
    routing::{get, post},
    Router,
};
use futures_util::{stream::Stream, SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{oneshot, Mutex};
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use super::session::{
    PendingOp, PollResponse, QueuedOp, Session, SessionInfo, SessionStats, MAX_QUEUE,
    SESSION_EXPIRE_MS,
};
use super::BridgeHandle;
use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};

pub const DEFAULT_PORT: u16 = 38451;
pub const PORT_RANGE: u16 = 10;
pub const LONG_POLL_MS: u64 = 8_000;
pub const DEFAULT_OP_TIMEOUT_MS: u64 = 60_000;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn get_op_timeout(op: &str) -> u64 {
    match op {
        "screenshot" | "scan_design" | "export_image" | "batch" => 90_000,
        "export_svg" | "get_design" => 60_000,
        _ => DEFAULT_OP_TIMEOUT_MS,
    }
}

pub struct BridgeInner {
    pub port: u16,
    pub sessions: HashMap<String, Session>,
    pub op_to_session: HashMap<String, String>,
    pub global_stats: SessionStats,
    pub mcp_sse_clients: HashMap<String, tokio::sync::mpsc::UnboundedSender<JsonRpcResponse>>,
}

#[derive(Clone)]
pub struct BridgeState {
    pub inner: Arc<Mutex<BridgeInner>>,
}

impl BridgeState {
    pub fn new(port: u16) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BridgeInner {
                port,
                sessions: HashMap::new(),
                op_to_session: HashMap::new(),
                global_stats: SessionStats::default(),
                mcp_sse_clients: HashMap::new(),
            })),
        }
    }

    pub async fn register_mcp_client(
        &self,
        session_id: &str,
        tx: tokio::sync::mpsc::UnboundedSender<JsonRpcResponse>,
    ) {
        let mut inner = self.inner.lock().await;
        inner.mcp_sse_clients.insert(session_id.to_string(), tx);
    }

    pub async fn remove_mcp_client(&self, session_id: &str) {
        let mut inner = self.inner.lock().await;
        inner.mcp_sse_clients.remove(session_id);
    }

    pub async fn send_mcp_response(&self, session_id: &str, resp: JsonRpcResponse) -> bool {
        let inner = self.inner.lock().await;
        if let Some(tx) = inner.mcp_sse_clients.get(session_id) {
            tx.send(resp).is_ok()
        } else {
            false
        }
    }

    pub async fn get_mcp_client_count(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.mcp_sse_clients.len()
    }

    pub async fn is_plugin_connected(&self, session_id: Option<&str>) -> bool {
        let inner = self.inner.lock().await;
        if let Some(sid) = session_id {
            if let Some(s) = inner.sessions.get(sid) {
                return s.is_connected();
            }
            return false;
        }
        inner.sessions.values().any(|s| s.is_connected())
    }

    pub async fn get_sessions(&self) -> Vec<SessionInfo> {
        let inner = self.inner.lock().await;
        let now = now_ms();
        inner
            .sessions
            .values()
            .map(|s| SessionInfo {
                id: s.id.clone(),
                file_name: s.file_name.clone(),
                connected: s.is_connected(),
                last_poll_ago_ms: if s.last_poll_at > 0 { Some(now - s.last_poll_at) } else { None },
                queue_length: s.queue.len(),
                ops: s.stats.ops,
            })
            .collect()
    }

    pub async fn get_queue_length(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.sessions.values().map(|s| s.queue.len()).sum()
    }

    pub async fn get_pending_count(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.op_to_session.len()
    }

    pub async fn get_last_poll_at(&self) -> u64 {
        let inner = self.inner.lock().await;
        inner.sessions.values().map(|s| s.last_poll_at).max().unwrap_or(0)
    }

    pub async fn send_operation(
        &self,
        operation: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        let (rx, op_id, timeout_ms) = {
            let mut inner = self.inner.lock().await;

            let sid = if let Some(id) = session_id {
                if inner.sessions.get(id).is_some_and(|s| s.is_connected()) {
                    id.to_string()
                } else {
                    Self::resolve_best_session_id(&inner)
                }
            } else {
                Self::resolve_best_session_id(&inner)
            };

            let session = inner
                .sessions
                .entry(sid.clone())
                .or_insert_with(|| Session::new(sid.clone(), None));

            if session.queue.len() >= MAX_QUEUE {
                return Err("Queue full — is the Figma plugin running?".to_string());
            }

            let timeout_ms = get_op_timeout(operation);
            let op_id = format!("{}-{}", now_ms(), &Uuid::new_v4().to_string()[..5]);

            let (tx, rx) = oneshot::channel();
            session.pending.insert(
                op_id.clone(),
                PendingOp {
                    sender: tx,
                    start_ms: now_ms(),
                },
            );

            // Fast-path: If WebSocket is connected, dispatch instantly (< 0.1ms)!
            let dispatched_via_ws = if let Some(ref ws_tx) = session.ws_tx {
                let payload = json!({
                    "id": op_id,
                    "operation": operation,
                    "params": params,
                });
                ws_tx.send(Message::Text(payload.to_string())).is_ok()
            } else {
                false
            };

            if !dispatched_via_ws {
                session.queue.push(QueuedOp {
                    id: op_id.clone(),
                    operation: operation.to_string(),
                    params,
                });

                let responder_opt = session.long_poll.take();
                let mut flushed_ops = Vec::new();
                if responder_opt.is_some() {
                    session.last_poll_at = now_ms();
                    flushed_ops = std::mem::take(&mut session.queue);
                }

                if let Some(responder) = responder_opt {
                    let _ = responder.send(PollResponse {
                        requests: flushed_ops,
                        mode: "ready".to_string(),
                        session_id: sid.clone(),
                    });
                }
            }

            inner.op_to_session.insert(op_id.clone(), sid.clone());
            (rx, op_id, timeout_ms)
        };

        // Await with timeout
        match tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(val)) => val,
            Ok(Err(_)) => Err("Operation cancelled or bridge closed".to_string()),
            Err(_) => {
                // Timeout clean up
                let mut inner = self.inner.lock().await;
                if let Some(sid) = inner.op_to_session.remove(&op_id) {
                    if let Some(s) = inner.sessions.get_mut(&sid) {
                        s.pending.remove(&op_id);
                        s.queue.retain(|q| q.id != op_id);
                    }
                }
                Err(format!("Operation \"{}\" timed out after {}ms", operation, timeout_ms))
            }
        }
    }

    fn resolve_best_session_id(inner: &BridgeInner) -> String {
        let mut best_lp: Option<(&Session, u64)> = None;
        let mut best_conn: Option<(&Session, u64)> = None;

        for s in inner.sessions.values() {
            if s.is_connected() {
                if s.long_poll.is_some() && best_lp.is_none_or(|(_, t)| s.last_poll_at > t) {
                    best_lp = Some((s, s.last_poll_at));
                }
                if best_conn.is_none_or(|(_, t)| s.last_poll_at > t) {
                    best_conn = Some((s, s.last_poll_at));
                }
            }
        }

        if let Some((s, _)) = best_lp {
            return s.id.clone();
        }
        if let Some((s, _)) = best_conn {
            return s.id.clone();
        }
        "_default".to_string()
    }
}

// ── HTTP Handlers ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SessionQuery {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "fileName")]
    file_name: Option<String>,
    init: Option<bool>,
}

#[derive(Deserialize)]
struct ResponsePayload {
    id: String,
    #[serde(default)]
    success: bool,
    data: Option<Value>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ExecPayload {
    operation: String,
    #[serde(default)]
    params: Value,
}

async fn handle_root(State(state): State<BridgeState>) -> impl IntoResponse {
    let port = state.inner.lock().await.port;
    let sessions = state.get_sessions().await;
    let connected = state.is_plugin_connected(None).await;
    let queue_len = state.get_queue_length().await;
    let mcp_clients = state.get_mcp_client_count().await;

    Json(json!({
        "server": "figma-mcp",
        "version": "2.5.26",
        "port": port,
        "pluginConnected": connected,
        "mcpClientsConnected": mcp_clients,
        "sessions": sessions,
        "queueLength": queue_len,
        "endpoints": [
            "/health",
            "/poll",
            "/response",
            "/exec",
            "/clear",
            "/sessions",
            "/sse",
            "/message",
            "/mcp"
        ]
    }))
}

async fn handle_sessions(State(state): State<BridgeState>) -> impl IntoResponse {
    let sessions = state.get_sessions().await;
    Json(json!({ "sessions": sessions }))
}

async fn handle_poll(
    State(state): State<BridgeState>,
    Query(query): Query<SessionQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let sid = query.session_id.or_else(|| {
        headers.get("x-session-id").and_then(|h| h.to_str().ok()).map(|s| s.to_string())
    }).unwrap_or_else(|| "_default".to_string());

    let is_init = query.init.unwrap_or(false);

    let (immediate_resp, rx) = {
        let mut inner = state.inner.lock().await;
        let session = inner
            .sessions
            .entry(sid.clone())
            .or_insert_with(|| Session::new(sid.clone(), query.file_name.clone()));

        if let Some(fn_name) = query.file_name {
            session.file_name = fn_name;
        }
        let is_first_poll = session.last_poll_at == 0;
        session.last_poll_at = now_ms();

        // Check if there are queued items that have active pending callers
        let alive_ops: Vec<QueuedOp> = session
            .queue
            .drain(..)
            .filter(|q| session.pending.contains_key(&q.id))
            .collect();

        if !alive_ops.is_empty() {
            (
                Some(PollResponse {
                    requests: alive_ops,
                    mode: "ready".to_string(),
                    session_id: sid.clone(),
                }),
                None,
            )
        } else if is_init || is_first_poll {
            // Instant handshake on startup
            (
                Some(PollResponse {
                    requests: Vec::new(),
                    mode: "ready".to_string(),
                    session_id: sid.clone(),
                }),
                None,
            )
        } else {
            let (tx, rx) = oneshot::channel();
            // Drop previous long poll if present
            session.long_poll = Some(tx);
            (None, Some(rx))
        }
    };

    if let Some(resp) = immediate_resp {
        return Json(resp);
    }

    if let Some(rx) = rx {
        match tokio::time::timeout(Duration::from_millis(LONG_POLL_MS), rx).await {
            Ok(Ok(resp)) => Json(resp),
            _ => {
                // Poll timeout, return empty requests
                let mut inner = state.inner.lock().await;
                if let Some(s) = inner.sessions.get_mut(&sid) {
                    s.long_poll = None;
                    s.last_poll_at = now_ms();
                }
                Json(PollResponse {
                    requests: Vec::new(),
                    mode: "ready".to_string(),
                    session_id: sid,
                })
            }
        }
    } else {
        Json(PollResponse {
            requests: Vec::new(),
            mode: "ready".to_string(),
            session_id: sid,
        })
    }
}

async fn handle_response(
    State(state): State<BridgeState>,
    Json(payload): Json<ResponsePayload>,
) -> impl IntoResponse {
    let mut inner = state.inner.lock().await;
    if let Some(sid) = inner.op_to_session.remove(&payload.id) {
        if let Some(session) = inner.sessions.get_mut(&sid) {
            if let Some(pending) = session.pending.remove(&payload.id) {
                let latency = now_ms() - pending.start_ms;
                session.stats.ops += 1;
                session.stats.avg_latency_ms = (session.stats.avg_latency_ms * 9 + latency) / 10;
                inner.global_stats.ops += 1;
                inner.global_stats.avg_latency_ms = (inner.global_stats.avg_latency_ms * 9 + latency) / 10;

                let res = if payload.success {
                    Ok(payload.data.unwrap_or(Value::Null))
                } else {
                    Err(payload.error.unwrap_or_else(|| "Plugin error".to_string()))
                };
                let _ = pending.sender.send(res);
            }
        }
    }
    Json(json!({ "ok": true }))
}

async fn handle_exec(
    State(state): State<BridgeState>,
    Query(query): Query<SessionQuery>,
    headers: HeaderMap,
    Json(payload): Json<ExecPayload>,
) -> impl IntoResponse {
    let sid = query.session_id.or_else(|| {
        headers.get("x-session-id").and_then(|h| h.to_str().ok()).map(|s| s.to_string())
    });

    if !state.is_plugin_connected(sid.as_deref()).await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Plugin not connected" })),
        );
    }

    match state.send_operation(&payload.operation, payload.params, sid.as_deref()).await {
        Ok(data) => (StatusCode::OK, Json(json!({ "success": true, "data": data }))),
        Err(e) => (StatusCode::OK, Json(json!({ "success": false, "error": e }))),
    }
}

async fn handle_health(State(state): State<BridgeState>) -> impl IntoResponse {
    let now = now_ms();
    let mut inner = state.inner.lock().await;

    // Cleanup expired sessions
    inner.sessions.retain(|_, s| {
        s.is_connected()
            || !s.queue.is_empty()
            || !s.pending.is_empty()
            || (now - s.last_poll_at) < SESSION_EXPIRE_MS
    });

    let last_poll = inner.sessions.values().map(|s| s.last_poll_at).max().unwrap_or(0);
    let queue_len: usize = inner.sessions.values().map(|s| s.queue.len()).sum();
    let pending_cnt = inner.op_to_session.len();
    let connected = inner.sessions.values().any(|s| s.is_connected());
    let sessions_list: Vec<SessionInfo> = inner
        .sessions
        .values()
        .map(|s| SessionInfo {
            id: s.id.clone(),
            file_name: s.file_name.clone(),
            connected: s.is_connected(),
            last_poll_ago_ms: if s.last_poll_at > 0 { Some(now - s.last_poll_at) } else { None },
            queue_length: s.queue.len(),
            ops: s.stats.ops,
        })
        .collect();

    Json(json!({
        "pluginConnected": connected,
        "queueLength": queue_len,
        "pendingCount": pending_cnt,
        "lastPollAgoMs": if last_poll > 0 { Some(now - last_poll) } else { None },
        "sessions": sessions_list,
        "stats": {
            "ops": inner.global_stats.ops,
            "avgLatencyMs": inner.global_stats.avg_latency_ms,
            "sessions": inner.sessions.len()
        }
    }))
}

async fn handle_clear(
    State(state): State<BridgeState>,
    Query(query): Query<SessionQuery>,
) -> impl IntoResponse {
    let mut inner = state.inner.lock().await;
    let mut cleared = 0;

    let target_sids: Vec<String> = if let Some(sid) = query.session_id {
        vec![sid]
    } else {
        inner.sessions.keys().cloned().collect()
    };

    let mut removed_ids = Vec::new();
    for sid in target_sids {
        if let Some(s) = inner.sessions.get_mut(&sid) {
            cleared += s.queue.len() + s.pending.len();
            for (id, p) in s.pending.drain() {
                let _ = p.sender.send(Err("Queue cleared manually".to_string()));
                removed_ids.push(id);
            }
            s.queue.clear();
        }
    }
    for id in removed_ids {
        inner.op_to_session.remove(&id);
    }

    Json(json!({
        "cleared": cleared,
        "queueLength": 0,
        "pendingCount": 0
    }))
}

async fn handle_ws(
    ws: WebSocketUpgrade,
    Query(query): Query<SessionQuery>,
    State(state): State<BridgeState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, query, state))
}

async fn handle_socket(
    socket: WebSocket,
    query: SessionQuery,
    state: BridgeState,
) {
    let sid = query.session_id.unwrap_or_else(|| "_default".to_string());
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    // Register ws_tx in session and flush queued ops
    {
        let mut inner = state.inner.lock().await;
        let session = inner
            .sessions
            .entry(sid.clone())
            .or_insert_with(|| Session::new(sid.clone(), query.file_name.clone()));
        if let Some(fn_name) = query.file_name {
            session.file_name = fn_name;
        }
        session.ws_tx = Some(tx);
        session.last_poll_at = now_ms();

        // Flush any queued ops directly over WebSocket
        let queued = std::mem::take(&mut session.queue);
        for op in queued {
            let msg = json!({
                "id": op.id,
                "operation": op.operation,
                "params": op.params,
            });
            let _ = sender.send(Message::Text(msg.to_string())).await;
        }
    }

    // Task to forward outgoing messages from tx -> WebSocket
    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Task to receive incoming responses from WebSocket
    let state_clone = state.clone();
    let sid_clone = sid.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&text) {
                        if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
                            let success = val.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
                            let data = val.get("data").cloned();
                            let error = val.get("error").and_then(|v| v.as_str()).map(|s| s.to_string());

                            let mut inner = state_clone.inner.lock().await;
                            if let Some(s_id) = inner.op_to_session.remove(id) {
                                if let Some(session) = inner.sessions.get_mut(&s_id) {
                                    if let Some(pending) = session.pending.remove(id) {
                                        let latency = now_ms() - pending.start_ms;
                                        session.stats.ops += 1;
                                        session.stats.avg_latency_ms = (session.stats.avg_latency_ms * 9 + latency) / 10;
                                        inner.global_stats.ops += 1;

                                        if success {
                                            let _ = pending.sender.send(Ok(data.unwrap_or(Value::Null)));
                                        } else {
                                            let _ = pending.sender.send(Err(error.unwrap_or_else(|| "Unknown error".to_string())));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Message::Ping(_) => {
                    let mut inner = state_clone.inner.lock().await;
                    if let Some(s) = inner.sessions.get_mut(&sid_clone) {
                        s.last_poll_at = now_ms();
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    };

    // Clean up session ws_tx when socket disconnects
    let mut inner = state.inner.lock().await;
    if let Some(s) = inner.sessions.get_mut(&sid) {
        s.ws_tx = None;
    }
}

struct McpSseStream {
    session_id: String,
    state: BridgeState,
    initial_sent: bool,
    rx: tokio::sync::mpsc::UnboundedReceiver<JsonRpcResponse>,
}

impl Stream for McpSseStream {
    type Item = Result<Event, std::convert::Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if !self.initial_sent {
            self.initial_sent = true;
            let endpoint_url = format!("/message?sessionId={}", self.session_id);
            let event = Event::default().event("endpoint").data(endpoint_url);
            return Poll::Ready(Some(Ok(event)));
        }

        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(resp)) => {
                let data_str = serde_json::to_string(&resp).unwrap_or_default();
                let event = Event::default().event("message").data(data_str);
                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for McpSseStream {
    fn drop(&mut self) {
        let state = self.state.clone();
        let sid = self.session_id.clone();
        tokio::spawn(async move {
            state.remove_mcp_client(&sid).await;
            eprintln!("[figma-mcp] 🤖 MCP Client disconnected (Session: {})", sid);
        });
    }
}

async fn handle_mcp_sse(
    State(state): State<BridgeState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let session_id = Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<JsonRpcResponse>();

    state.register_mcp_client(&session_id, tx).await;
    eprintln!("[figma-mcp] 🤖 MCP Client connected via SSE (Session: {})", session_id);

    let stream = McpSseStream {
        session_id,
        state,
        initial_sent: false,
        rx,
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}

#[derive(Deserialize)]
struct McpMessageQuery {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

async fn handle_mcp_message(
    State(state): State<BridgeState>,
    Query(query): Query<McpMessageQuery>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let bridge = BridgeHandle::Direct(state.clone());
    let resp = crate::mcp::server::handle_jsonrpc_request(bridge, req).await;

    if let Some(resp) = resp {
        if let Some(ref sid) = query.session_id {
            if state.send_mcp_response(sid, resp.clone()).await {
                return (StatusCode::ACCEPTED, Json(json!({ "ok": true }))).into_response();
            }
        }
        (StatusCode::OK, Json(serde_json::to_value(resp).unwrap_or(json!({})))).into_response()
    } else {
        (StatusCode::ACCEPTED, Json(json!({ "ok": true }))).into_response()
    }
}

async fn handle_mcp_direct(
    State(state): State<BridgeState>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let bridge = BridgeHandle::Direct(state.clone());
    let resp = crate::mcp::server::handle_jsonrpc_request(bridge, req).await;

    if let Some(resp) = resp {
        (StatusCode::OK, Json(serde_json::to_value(resp).unwrap_or(json!({})))).into_response()
    } else {
        (StatusCode::ACCEPTED, Json(json!({ "ok": true }))).into_response()
    }
}

pub fn create_router(state: BridgeState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/", get(handle_root))
        .route("/sessions", get(handle_sessions))
        .route("/poll", get(handle_poll))
        .route("/response", post(handle_response))
        .route("/exec", post(handle_exec))
        .route("/ws", get(handle_ws))
        .route("/health", get(handle_health))
        .route("/clear", get(handle_clear).post(handle_clear))
        .route("/sse", get(handle_mcp_sse))
        .route("/message", post(handle_mcp_message))
        .route("/messages", post(handle_mcp_message))
        .route("/mcp", post(handle_mcp_direct))
        .layer(cors)
        .with_state(state)
}

pub async fn kill_stale_bridges(port: u16) {
    #[cfg(unix)]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("lsof").arg("-ti").arg(format!("tcp:{}", port)).output() {
            if let Ok(pids_str) = String::from_utf8(output.stdout) {
                let current_pid = std::process::id();
                let pids: Vec<&str> = pids_str
                    .split_whitespace()
                    .filter(|p| p.parse::<u32>().is_ok_and(|pid| pid != current_pid))
                    .collect();

                if !pids.is_empty() {
                    for pid in &pids {
                        let _ = Command::new("kill").arg("-9").arg(pid).output();
                    }
                    eprintln!(
                        "[figma-mcp] Killed {} zombie(s) on port {}: {}",
                        pids.len(),
                        port,
                        pids.join(",")
                    );
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
            }
        }
    }
}
