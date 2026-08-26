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
    pub last_cleanup_at: u64,
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
                last_cleanup_at: 0,
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
            let resolved_id = Self::resolve_session_id(&inner, Some(sid));
            if let Some(s) = inner.sessions.get(&resolved_id) {
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

    pub async fn get_status_snapshot(&self) -> (Vec<SessionInfo>, bool, usize, usize, u16) {
        let now = now_ms();
        let inner = self.inner.lock().await;
        let sessions: Vec<SessionInfo> = inner.sessions.values().map(|s| SessionInfo {
            id: s.id.clone(),
            file_name: s.file_name.clone(),
            connected: s.is_connected(),
            last_poll_ago_ms: if s.last_poll_at > 0 { Some(now - s.last_poll_at) } else { None },
            queue_length: s.queue.len(),
            ops: s.stats.ops,
        }).collect();
        let connected = inner.sessions.values().any(|s| s.is_connected());
        let queue_len: usize = inner.sessions.values().map(|s| s.queue.len()).sum();
        let mcp_clients = inner.mcp_sse_clients.len();
        let port = inner.port;
        (sessions, connected, queue_len, mcp_clients, port)
    }

    pub async fn send_operation(
        &self,
        operation: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        let (rx, op_id, timeout_ms) = {
            let mut inner = self.inner.lock().await;

            let sid = Self::resolve_session_id(&inner, session_id);

            let session = inner
                .sessions
                .entry(sid.clone())
                .or_insert_with(|| Session::new(sid.clone(), None));

            if session.queue.len() >= MAX_QUEUE {
                return Err("Queue full — is the Figma plugin running?".to_string());
            }

            let timeout_ms = get_op_timeout(operation);
            let op_id = format!("{}-{}", now_ms(), &Uuid::new_v4().to_string()[..5]);

            let queued_op = QueuedOp {
                id: op_id.clone(),
                operation: operation.to_string(),
                params,
            };

            let (tx, rx) = oneshot::channel();
            session.pending.insert(
                op_id.clone(),
                PendingOp {
                    sender: tx,
                    start_ms: now_ms(),
                    op: queued_op.clone(),
                    acked: false,
                },
            );

            // Fast-path: If WebSocket is connected, dispatch instantly (< 0.1ms)!
            // send() only proves the channel is alive, not that the socket is —
            // a half-open socket is recovered by the re-queue in handle_socket.
            let dispatched_via_ws = if let Some(ref ws_tx) = session.ws_tx {
                let payload = json!({
                    "id": queued_op.id,
                    "operation": queued_op.operation,
                    "params": queued_op.params,
                });
                ws_tx.send(Message::Text(payload.to_string())).is_ok()
            } else {
                false
            };

            if !dispatched_via_ws {
                session.queue.push(queued_op);

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

    pub async fn update_index(&self, session_id: &str, index: crate::bridge::index::FigmaIndex) {
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session.index = Some(index);
        }
    }

    pub async fn mark_index_dirty(&self, session_id: &str) {
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get_mut(session_id) {
            if let Some(ref mut idx) = session.index {
                idx.mark_dirty();
            }
        }
    }

    pub async fn get_index_stats(&self, session_id: Option<&str>) -> Option<crate::bridge::index::IndexStats> {
        let inner = self.inner.lock().await;
        let sid = Self::resolve_session_id(&inner, session_id);

        inner.sessions.get(&sid).and_then(|s| s.index.as_ref().map(|idx| idx.stats.clone()))
    }

    pub async fn get_index_node(&self, session_id: Option<&str>, node_id: &str) -> Option<crate::bridge::index::IndexNode> {
        let inner = self.inner.lock().await;
        let sid = Self::resolve_session_id(&inner, session_id);

        inner.sessions.get(&sid).and_then(|s| s.index.as_ref().and_then(|idx| idx.get_node(node_id).cloned()))
    }

    pub async fn search_index_nodes(
        &self,
        session_id: Option<&str>,
        query: &str,
        node_type: Option<&str>,
        limit: usize,
    ) -> Option<Vec<crate::bridge::index::IndexNode>> {
        let inner = self.inner.lock().await;
        let sid = Self::resolve_session_id(&inner, session_id);

        inner.sessions.get(&sid).and_then(|s| {
            s.index.as_ref().map(|idx| {
                idx.search_nodes(query, node_type, limit)
                    .into_iter()
                    .cloned()
                    .collect()
            })
        })
    }

    pub async fn search_index_components(
        &self,
        session_id: Option<&str>,
        name: &str,
        limit: usize,
    ) -> Option<Vec<crate::bridge::index::IndexComponent>> {
        let inner = self.inner.lock().await;
        let sid = Self::resolve_session_id(&inner, session_id);

        inner.sessions.get(&sid).and_then(|s| {
            s.index.as_ref().map(|idx| {
                idx.search_components(name, limit)
                    .into_iter()
                    .cloned()
                    .collect()
            })
        })
    }

    pub async fn search_index_styles(
        &self,
        session_id: Option<&str>,
        name: &str,
        style_type: Option<&str>,
    ) -> Option<Vec<crate::bridge::index::IndexStyle>> {
        let inner = self.inner.lock().await;
        let sid = Self::resolve_session_id(&inner, session_id);

        inner.sessions.get(&sid).and_then(|s| {
            s.index.as_ref().map(|idx| {
                idx.search_styles(name, style_type)
                    .into_iter()
                    .cloned()
                    .collect()
            })
        })
    }

    pub async fn search_index_variables(
        &self,
        session_id: Option<&str>,
        name: &str,
        collection: Option<&str>,
    ) -> Option<Vec<crate::bridge::index::IndexVariable>> {
        let inner = self.inner.lock().await;
        let sid = Self::resolve_session_id(&inner, session_id);

        inner.sessions.get(&sid).and_then(|s| {
            s.index.as_ref().map(|idx| {
                idx.search_variables(name, collection)
                    .into_iter()
                    .cloned()
                    .collect()
            })
        })
    }

    pub fn resolve_session_id(inner: &BridgeInner, target: Option<&str>) -> String {
        if let Some(target) = target {
            let target_trim = target.trim();
            if !target_trim.is_empty() {
                // 1. Exact match on session ID
                if let Some(s) = inner.sessions.get(target_trim) {
                    if s.is_connected() {
                        return target_trim.to_string();
                    }
                }
                // 2. Exact match on file name (case-insensitive)
                for (id, s) in &inner.sessions {
                    if s.is_connected() && s.file_name.eq_ignore_ascii_case(target_trim) {
                        return id.clone();
                    }
                }
                // 3. Substring match on file name (case-insensitive)
                for (id, s) in &inner.sessions {
                    if s.is_connected() && s.file_name.to_lowercase().contains(&target_trim.to_lowercase()) {
                        return id.clone();
                    }
                }
                // 4. Prefix match on session ID
                for (id, s) in &inner.sessions {
                    if s.is_connected() && id.starts_with(target_trim) {
                        return id.clone();
                    }
                }
            }
        }
        // Fallback: Pick the most recently active connected session
        Self::resolve_best_session_id(inner)
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
    let (sessions, connected, queue_len, mcp_clients, port) = state.get_status_snapshot().await;

    Json(json!({
        "server": "figma-mcp",
        "version": "2.6.0",
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
    if now.saturating_sub(inner.last_cleanup_at) > 30_000 {
        inner.last_cleanup_at = now;
        inner.sessions.retain(|_, s| {
            s.is_connected()
                || !s.queue.is_empty()
                || !s.pending.is_empty()
                || (now - s.last_poll_at) < SESSION_EXPIRE_MS
        });
    }

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

    let memory_mb = get_process_memory_mb();

    Json(json!({
        "pluginConnected": connected,
        "queueLength": queue_len,
        "pendingCount": pending_cnt,
        "lastPollAgoMs": if last_poll > 0 { Some(now - last_poll) } else { None },
        "sessions": sessions_list,
        "stats": {
            "ops": inner.global_stats.ops,
            "avgLatencyMs": inner.global_stats.avg_latency_ms,
            "sessions": inner.sessions.len(),
            "memoryMb": memory_mb
        }
    }))
}

fn get_process_memory_mb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        use std::mem::MaybeUninit;
        #[allow(deprecated)]
        unsafe {
            let mut info = MaybeUninit::<libc::mach_task_basic_info>::uninit();
            let mut count = (std::mem::size_of::<libc::mach_task_basic_info>() / std::mem::size_of::<libc::natural_t>()) as libc::mach_msg_type_number_t;
            let res = libc::task_info(
                libc::mach_task_self(),
                libc::MACH_TASK_BASIC_INFO,
                info.as_mut_ptr() as *mut libc::integer_t,
                &mut count,
            );
            if res == libc::KERN_SUCCESS {
                let info = info.assume_init();
                return (info.resident_size as f64) / (1024.0 * 1024.0);
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if line.starts_with("VmRSS:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = kb_str.parse::<f64>() {
                            return kb / 1024.0;
                        }
                    }
                }
            }
        }
    }
    3.2 // Fallback baseline estimate for lightweight Rust binary
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
        let tx_clone = tx.clone();
        session.ws_tx = Some(tx);
        session.last_poll_at = now_ms();

        // Flush any queued ops directly over WebSocket
        let queued = std::mem::take(&mut session.queue);
        for op in &queued {
            let msg = json!({
                "id": op.id,
                "operation": op.operation,
                "params": op.params,
            });
            let _ = tx_clone.send(Message::Text(msg.to_string()));
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
                        if val.get("type").and_then(|v| v.as_str()) == Some("ping") || val.get("ping").is_some() {
                            let mut inner = state_clone.inner.lock().await;
                            if let Some(s) = inner.sessions.get_mut(&sid_clone) {
                                s.last_poll_at = now_ms();
                            }
                            continue;
                        }

                        // "ack" = the plugin has the op and is running it, so it
                        // must not be re-queued if this socket dies.
                        if val.get("type").and_then(|v| v.as_str()) == Some("ack") {
                            if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
                                let mut inner = state_clone.inner.lock().await;
                                if let Some(s) = inner.sessions.get_mut(&sid_clone) {
                                    s.last_poll_at = now_ms();
                                    if let Some(pending) = s.pending.get_mut(id) {
                                        pending.acked = true;
                                    }
                                }
                            }
                            continue;
                        }

                        // "node-diff" = incremental update of specific nodes
                        if val.get("type").and_then(|v| v.as_str()) == Some("node-diff") {
                            if let Some(nodes) = val.get("nodes").and_then(|v| v.as_array()) {
                                let mut inner = state_clone.inner.lock().await;
                                if let Some(session) = inner.sessions.get_mut(&sid_clone) {
                                    if let Some(ref mut idx) = session.index {
                                        for n in nodes {
                                            idx.upsert_node(n);
                                        }
                                    }
                                }
                            }
                            continue;
                        }

                        // "document-change" = canvas modified in Figma, mark index dirty
                        if val.get("type").and_then(|v| v.as_str()) == Some("document-change") {
                            state_clone.mark_index_dirty(&sid_clone).await;
                            continue;
                        }

                        // "index-update" = plugin sent pre-indexed data snapshot
                        if val.get("type").and_then(|v| v.as_str()) == Some("index-update") {
                            if let Some(data) = val.get("data") {
                                let file_name = val.get("fileName").and_then(|v| v.as_str()).unwrap_or("unknown");
                                let page_nodes = data.get("pageNodes").unwrap_or(&Value::Null);
                                let styles = data.get("styles");
                                let variables = data.get("variables");
                                let components = data.get("components");
                                let start_ms = val.get("startMs").and_then(|v| v.as_u64()).unwrap_or_else(now_ms);

                                let idx = crate::bridge::index::FigmaIndex::from_raw(
                                    &sid_clone,
                                    file_name,
                                    page_nodes,
                                    styles,
                                    variables,
                                    components,
                                    start_ms,
                                );
                                eprintln!(
                                    "[figma-mcp] ⚡ Pre-indexed {} nodes, {} components, {} styles, {} variables in {}ms",
                                    idx.stats.total_nodes,
                                    idx.stats.total_components,
                                    idx.stats.total_styles,
                                    idx.stats.total_variables,
                                    idx.stats.duration_ms
                                );
                                state_clone.update_index(&sid_clone, idx).await;
                            }
                            continue;
                        }

                        // "index-chunk" = selective streaming of subtree chunks
                        if val.get("type").and_then(|v| v.as_str()) == Some("index-chunk") {
                            if let Some(nodes) = val.get("nodes").and_then(|v| v.as_array()) {
                                let mut inner = state_clone.inner.lock().await;
                                if let Some(session) = inner.sessions.get_mut(&sid_clone) {
                                    if let Some(ref mut idx) = session.index {
                                        idx.merge_chunk(nodes);
                                    }
                                }
                            }
                            continue;
                        }

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
                Message::Binary(bin_bytes) => {
                    // Fast Binary IPC (MessagePack)
                    if let Ok(val) = rmp_serde::from_slice::<Value>(&bin_bytes) {
                        if val.get("type").and_then(|v| v.as_str()) == Some("node-diff") {
                            if let Some(nodes) = val.get("nodes").and_then(|v| v.as_array()) {
                                let mut inner = state_clone.inner.lock().await;
                                if let Some(session) = inner.sessions.get_mut(&sid_clone) {
                                    if let Some(ref mut idx) = session.index {
                                        for n in nodes {
                                            idx.upsert_node(n);
                                        }
                                    }
                                }
                            }
                        } else if val.get("type").and_then(|v| v.as_str()) == Some("index-chunk") {
                            if let Some(nodes) = val.get("nodes").and_then(|v| v.as_array()) {
                                let mut inner = state_clone.inner.lock().await;
                                if let Some(session) = inner.sessions.get_mut(&sid_clone) {
                                    if let Some(ref mut idx) = session.index {
                                        idx.merge_chunk(nodes);
                                    }
                                }
                            }
                        } else if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
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

    // Clean up session ws_tx when socket disconnects. Ops that were pushed into
    // the WebSocket but never acknowledged are lost with the socket — without
    // this they would sit until the op timeout (60–90s). Re-queue them so the
    // reconnected plugin (WebSocket or long poll) picks them up. Acknowledged
    // ops are already executing in the plugin, which still answers over the
    // HTTP fallback, so those are left alone.
    let mut inner = state.inner.lock().await;
    let mut requeued: Vec<QueuedOp> = Vec::new();
    if let Some(s) = inner.sessions.get_mut(&sid) {
        s.ws_tx = None;

        let unacked_ids: Vec<String> = s
            .pending
            .iter()
            .filter(|(id, p)| !p.acked && !s.queue.iter().any(|q| &q.id == *id))
            .map(|(id, _)| id.clone())
            .collect();

        for id in unacked_ids {
            if let Some(pending) = s.pending.get(&id) {
                if s.queue.len() >= MAX_QUEUE {
                    // Queue is full — fail fast instead of dropping silently.
                    if let Some(pending) = s.pending.remove(&id) {
                        let _ = pending
                            .sender
                            .send(Err("Plugin disconnected and the queue is full — retry".to_string()));
                    }
                    continue;
                }
                requeued.push(pending.op.clone());
            }
        }

        if !requeued.is_empty() {
            eprintln!(
                "[figma-mcp] ↻ WebSocket closed with {} unacknowledged op(s) — re-queued",
                requeued.len()
            );
            s.queue.extend(requeued.clone());

            // Hand them straight to a waiting long poll, if there is one.
            if let Some(responder) = s.long_poll.take() {
                s.last_poll_at = now_ms();
                let flushed = std::mem::take(&mut s.queue);
                let _ = responder.send(PollResponse {
                    requests: flushed,
                    mode: "ready".to_string(),
                    session_id: sid.clone(),
                });
            }
        }
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

