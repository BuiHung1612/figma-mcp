//! HTTP/WebSocket handlers and the axum router for the bridge server.

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
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::oneshot;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use super::session::{
    PollResponse, QueuedOp, Session, SessionInfo, MAX_QUEUE,
};
use super::BridgeHandle;
use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use super::server::*;

// ── HTTP Handlers ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SessionQuery {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "fileName")]
    file_name: Option<String>,
    #[serde(rename = "documentId")]
    document_id: Option<String>,
    init: Option<bool>,
}

#[derive(Deserialize)]
struct ResponsePayload {
    id: String,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
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
        "server": "figma-rust-mcp",
        "version": env!("CARGO_PKG_VERSION"),
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

    let (immediate_resp, rx, poll_generation) = {
        let mut inner = state.inner.lock().await;
        let session = inner
            .sessions
            .entry(sid.clone())
            .or_insert_with(|| Session::new(sid.clone(), query.file_name.clone()));

        if let Some(document_id) = query.document_id { session.document_id = Some(document_id); }
        if let Some(fn_name) = query.file_name {
            session.file_name = fn_name;
        }
        session.poll_generation += 1;
        let generation = session.poll_generation;
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
                None, generation,
            )
        } else if is_init || is_first_poll {
            // Instant handshake on startup
            (
                Some(PollResponse {
                    requests: Vec::new(),
                    mode: "ready".to_string(),
                    session_id: sid.clone(),
                }),
                None, generation,
            )
        } else {
            let (tx, rx) = oneshot::channel();
            // Drop previous long poll if present
            session.long_poll = Some(tx);
            (None, Some(rx), generation)
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
                    if s.poll_generation == poll_generation {
                        s.long_poll = None;
                        s.last_poll_at = now_ms();
                    }
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
    if let Some(origin) = &payload.session_id {
        if inner.op_to_session.get(&payload.id) != Some(origin) { return Json(json!({"ok": false, "error": "Response belongs to a different tab"})); }
    }
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

    if matches!(payload.operation.as_str(), "task_start" | "task_end") {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "Use figma_task through MCP"})));
    }
    let (sid, lock, _) = match state.tool_target(sid.as_deref(), None, false).await {
        Ok(target) => target,
        Err(error) => return (StatusCode::BAD_REQUEST, Json(json!({"error": error}))),
    };
    let _guard = match tokio::time::timeout(Duration::from_secs(120), lock.lock_owned()).await {
        Ok(guard) => guard,
        Err(_) => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "Figma tab is busy"}))),
    };

    match state.send_operation(&payload.operation, payload.params, Some(&sid)).await {
        Ok(data) => (StatusCode::OK, Json(json!({ "success": true, "data": data }))),
        Err(e) => (StatusCode::OK, Json(json!({ "success": false, "error": e }))),
    }
}

async fn handle_health(State(state): State<BridgeState>) -> impl IntoResponse {
    let now = now_ms();
    let mut inner = state.inner.lock().await;

    inner.expire_sessions(now);

    let last_poll = inner.sessions.values().map(|s| s.last_poll_at).max().unwrap_or(0);
    let queue_len: usize = inner.sessions.values().map(|s| s.queue.len()).sum();
    let pending_cnt = inner.op_to_session.len();
    let connected = inner.sessions.values().any(|s| s.is_connected());
    let sessions_list: Vec<SessionInfo> = inner
        .sessions
        .values()
        .map(|s| SessionInfo {
            id: s.id.clone(),
                document_id: s.document_id.clone(),            file_name: s.file_name.clone(),
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

/// Resident memory of this process, or `None` where we have no reader (Windows).
pub(crate) fn get_process_memory_mb() -> Option<f64> {
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
                return Some((info.resident_size as f64) / (1024.0 * 1024.0));
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
                            return Some(kb / 1024.0);
                        }
                    }
                }
            }
        }
    }
    None
}

async fn handle_benchmark(State(state): State<BridgeState>) -> impl IntoResponse {
    Json(super::bench::run(&state).await)
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

    let socket_tx = tx.clone();

    // Register ws_tx in session and flush queued ops
    {
        let mut inner = state.inner.lock().await;
        let active_task_ids: Vec<_> = inner.tasks.values().filter(|task| task.session_id == sid).map(|task| task.task_id.clone()).collect();
        let session = inner
            .sessions
            .entry(sid.clone())
            .or_insert_with(|| Session::new(sid.clone(), query.file_name.clone()));
        if let Some(document_id) = query.document_id { session.document_id = Some(document_id); }
        if let Some(fn_name) = query.file_name {
            session.file_name = fn_name;
        }
        let tx_clone = tx.clone();
        session.invalidate_reads();
        if let Some(idx) = &mut session.index { idx.mark_dirty(); }
        session.ws_tx = Some(tx);
        session.operations = None;
        session.runtime_version = None;
        session.protocol_version = None;
        session.last_poll_at = now_ms();

        // Send initial handshake with server version
        let hello = json!({
            "type": "server-hello",
            "version": env!("CARGO_PKG_VERSION"),
            "name": "figma-rust-mcp",
            "dynamicRuntime": true,
            "runtimeHash": env!("FIGMA_RUNTIME_CODE_HASH"),
            "protocolVersion": 3,
            "activeTaskIds": active_task_ids,
            "connectedAt": now_ms()
        });
        let _ = tx_clone.send(Message::Text(hello.to_string()));

        for pending in session.pending.values() {
            if !pending.acked && !session.queue.iter().any(|op| op.id == pending.op.id) {
                session.queue.push(pending.op.clone());
            }
        }
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
    let owner_tx = socket_tx.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            {
                let inner = state_clone.inner.lock().await;
                if !inner.sessions.get(&sid_clone).and_then(|s| s.ws_tx.as_ref()).is_some_and(|tx| tx.same_channel(&owner_tx)) {
                    break;
                }
            }
            match msg {
                Message::Text(text) => {
                    if let Ok(mut val) = serde_json::from_str::<Value>(&text) {
                        if val["type"] == "runtime-capabilities" {
                            let mut inner = state_clone.inner.lock().await;
                            if let Some(s) = inner.sessions.get_mut(&sid_clone) {
                                s.document_id = val["documentId"].as_str().map(str::to_owned).or(s.document_id.clone());
                                s.runtime_version = val["runtimeVersion"].as_str().map(str::to_owned);
                                s.protocol_version = val["protocolVersion"].as_u64();
                                s.node_revision = 0;
                                s.pending_index = None;
                                s.resync_requested = false;
                                s.operations = val["operations"].as_array().map(|ops| ops.iter().filter_map(Value::as_str).map(str::to_owned).collect());
                                s.invalidate_reads();
                                if let Some(idx) = &mut s.index { idx.tokens_dirty = true; }
                            }
                            continue;
                        }
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

                        // "selection-change" = realtime selection event from Figma canvas
                        if val.get("type").and_then(|v| v.as_str()) == Some("selection-change") {
                            let count = val.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            let page_name = val.get("pageName").and_then(|v| v.as_str()).map(|s| s.to_string());
                            let sel_list = val.get("selection").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                            let full_node = val.get("fullNode").cloned();
                            let active_sel = crate::bridge::session::ActiveSelection {
                                count,
                                page_name,
                                selection: sel_list,
                                full_node,
                                updated_at: now_ms(),
                            };
                            state_clone.update_selection(&sid_clone, active_sel).await;
                            continue;
                        }

                        // "delta-diff" = micro delta updates for specific node properties
                        if val.get("type").and_then(|v| v.as_str()) == Some("delta-diff") {
                            if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
                                if let Some(diff) = val.get("diff") {
                                    state_clone.apply_delta(&sid_clone, val["pageId"].as_str(), id, diff).await;
                                }
                            }
                            continue;
                        }

                        // "node-diff" = incremental update of specific nodes
                        if val.get("type").and_then(|v| v.as_str()) == Some("node-diff") {
                            if let Some(nodes) = val.get("nodes").and_then(|v| v.as_array()) {
                                let deleted: Vec<String> = val["deletedIds"].as_array().map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
                                state_clone.update_changed_nodes(&sid_clone, val["pageId"].as_str(), nodes, &deleted).await;
                            }
                            continue;
                        }

                        // "document-change" = canvas modified in Figma, mark index dirty
                        if val.get("type").and_then(|v| v.as_str()) == Some("document-change") {
                            let ids: Vec<String> = val["changedNodeIds"].as_array().map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
                            if ids.is_empty() { state_clone.mark_index_dirty(&sid_clone).await; }
                            else { state_clone.invalidate_changed_nodes(&sid_clone, &ids).await; }
                            continue;
                        }

                        if matches!(val["type"].as_str(), Some("nodes-invalidated" | "node-patch" | "index-start" | "index-chunk" | "index-update" | "index-abort")) {
                            state_clone.receive_index_event(&sid_clone, &val).await;
                            continue;
                        }

                        if let Some(id) = val.get("id").and_then(|v| v.as_str()).map(str::to_owned) {
                            let success = val.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
                            let data = val.as_object_mut().and_then(|obj| obj.remove("data"));
                            let error = val.get("error").and_then(|v| v.as_str()).map(|s| s.to_string());

                            let mut inner = state_clone.inner.lock().await;
                            if inner.op_to_session.get(&id) != Some(&sid_clone) { continue; }
                            if let Some(s_id) = inner.op_to_session.remove(&id) {
                                if let Some(session) = inner.sessions.get_mut(&s_id) {
                                    if let Some(pending) = session.pending.remove(&id) {
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
                    if let Ok(mut val) = rmp_serde::from_slice::<Value>(&bin_bytes) {
                        if matches!(val["type"].as_str(), Some("nodes-invalidated" | "node-patch" | "index-start" | "index-chunk" | "index-update" | "index-abort")) {
                            state_clone.receive_index_event(&sid_clone, &val).await;
                            continue;
                        }
                        if val.get("type").and_then(|v| v.as_str()) == Some("selection-change") {
                            let count = val.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            let page_name = val.get("pageName").and_then(|v| v.as_str()).map(|s| s.to_string());
                            let sel_list = val.get("selection").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                            let full_node = val.get("fullNode").cloned();
                            let active_sel = crate::bridge::session::ActiveSelection {
                                count,
                                page_name,
                                selection: sel_list,
                                full_node,
                                updated_at: now_ms(),
                            };
                            state_clone.update_selection(&sid_clone, active_sel).await;
                        } else if val.get("type").and_then(|v| v.as_str()) == Some("delta-diff") {
                            if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
                                if let Some(diff) = val.get("diff") {
                                    state_clone.apply_delta(&sid_clone, val["pageId"].as_str(), id, diff).await;
                                }
                            }
                        } else if val.get("type").and_then(|v| v.as_str()) == Some("node-diff") {
                            if let Some(nodes) = val.get("nodes").and_then(|v| v.as_array()) {
                                let deleted: Vec<String> = val["deletedIds"].as_array().map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
                                state_clone.update_changed_nodes(&sid_clone, val["pageId"].as_str(), nodes, &deleted).await;
                            }
                        } else if val.get("type").and_then(|v| v.as_str()) == Some("index-chunk") {
                            if let Some(nodes) = val.get("nodes").and_then(|v| v.as_array()) {
                                let mut inner = state_clone.inner.lock().await;
                                if let Some(session) = inner.sessions.get_mut(&sid_clone) {
                                    session.invalidate_reads();
                                    if let Some(ref mut idx) = session.index {
                                        idx.merge_chunk(nodes);
                                    }
                                }
                            }
                        } else if let Some(id) = val.get("id").and_then(|v| v.as_str()).map(str::to_owned) {
                            let success = val.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
                            let data = val.as_object_mut().and_then(|obj| obj.remove("data"));
                            let error = val.get("error").and_then(|v| v.as_str()).map(|s| s.to_string());

                            let mut inner = state_clone.inner.lock().await;
                            if inner.op_to_session.get(&id) != Some(&sid_clone) { continue; }
                            if let Some(s_id) = inner.op_to_session.remove(&id) {
                                if let Some(session) = inner.sessions.get_mut(&s_id) {
                                    if let Some(pending) = session.pending.remove(&id) {
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
        if !s.ws_tx.as_ref().is_some_and(|tx| tx.same_channel(&socket_tx)) { return; }
        s.ws_tx = None;
        if s.long_poll.is_none() { s.last_poll_at = 0; }

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
                "[figma-rust-mcp] ↻ WebSocket closed with {} unacknowledged op(s) — re-queued",
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
            eprintln!("[figma-rust-mcp] 🤖 MCP Client disconnected (Session: {})", sid);
        });
    }
}

async fn handle_mcp_sse(
    State(state): State<BridgeState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let session_id = Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<JsonRpcResponse>();

    state.register_mcp_client(&session_id, tx).await;
    eprintln!("[figma-rust-mcp] 🤖 MCP Client connected via SSE (Session: {})", session_id);

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

async fn handle_asset_serve(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    let clean_path = path.trim_start_matches('/');
    if clean_path.contains("..") {
        return (StatusCode::BAD_REQUEST, "Invalid path").into_response();
    }
    let base_cache = std::env::temp_dir().join("figma-rust-mcp").join("assets");
    let asset_path = base_cache.join(clean_path);

    if let Ok(bytes) = tokio::fs::read(&asset_path).await {
        let mime = if clean_path.ends_with(".png") {
            "image/png"
        } else if clean_path.ends_with(".svg") {
            "image/svg+xml"
        } else if clean_path.ends_with(".jpg") || clean_path.ends_with(".jpeg") {
            "image/jpeg"
        } else {
            "application/octet-stream"
        };
        (
            StatusCode::OK,
            [("content-type", mime), ("cache-control", "public, max-age=3600")],
            bytes,
        ).into_response()
    } else {
        (StatusCode::NOT_FOUND, "Asset not found").into_response()
    }
}

pub const PLUGIN_RUNTIME_CODE_JS: &str = include_str!("../../plugin-runtime/code.js");
pub const PLUGIN_RUNTIME_UI_HTML: &str = include_str!("../../plugin-runtime/ui.html");

const RUNTIME_CODE_ETAG: &str = concat!("\"figma-code-", env!("CARGO_PKG_VERSION"), "-", env!("FIGMA_RUNTIME_CODE_HASH"), "\"");
const RUNTIME_UI_ETAG: &str = concat!("\"figma-ui-", env!("CARGO_PKG_VERSION"), "-", env!("FIGMA_RUNTIME_UI_HASH"), "\"");

#[cfg(test)]
fn runtime_code_hash() -> String { env!("FIGMA_RUNTIME_CODE_HASH").to_string() }

async fn handle_plugin_version() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            ("content-type", "application/json; charset=utf-8"),
            ("cache-control", "no-cache, no-store, must-revalidate"),
            ("access-control-allow-origin", "*"),
        ],
        Json(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "status": "ready",
            "name": "figma-rust-mcp",
            "dynamicRuntime": true,
            "runtimeHash": env!("FIGMA_RUNTIME_CODE_HASH"),
            "protocolVersion": 3
        })),
    )
}

async fn handle_plugin_code(headers: HeaderMap) -> impl IntoResponse {
    let runtime_etag = RUNTIME_CODE_ETAG.to_string();
    if let Some(if_none_match) = headers.get(axum::http::header::IF_NONE_MATCH) {
        if let Ok(val) = if_none_match.to_str() {
            if val == runtime_etag || val == "*" {
                return (
                    StatusCode::NOT_MODIFIED,
                    [
                        ("etag", runtime_etag),
                        ("cache-control", "public, max-age=0, must-revalidate".to_string()),
                        ("access-control-allow-origin", "*".to_string()),
                    ],
                    "",
                ).into_response();
            }
        }
    }

    (
        StatusCode::OK,
        [
            ("content-type", "application/javascript; charset=utf-8".to_string()),
            ("etag", runtime_etag),
            ("cache-control", "public, max-age=0, must-revalidate".to_string()),
            ("access-control-allow-origin", "*".to_string()),
        ],
        PLUGIN_RUNTIME_CODE_JS,
    ).into_response()
}

async fn handle_plugin_ui(headers: HeaderMap) -> impl IntoResponse {
    if let Some(if_none_match) = headers.get(axum::http::header::IF_NONE_MATCH) {
        if let Ok(val) = if_none_match.to_str() {
            if val == RUNTIME_UI_ETAG || val == "*" {
                return (
                    StatusCode::NOT_MODIFIED,
                    [
                        ("etag", RUNTIME_UI_ETAG),
                        ("cache-control", "public, max-age=0, must-revalidate"),
                        ("access-control-allow-origin", "*"),
                    ],
                    "",
                ).into_response();
            }
        }
    }

    (
        StatusCode::OK,
        [
            ("content-type", "text/html; charset=utf-8"),
            ("etag", RUNTIME_UI_ETAG),
            ("cache-control", "public, max-age=0, must-revalidate"),
            ("access-control-allow-origin", "*"),
        ],
        PLUGIN_RUNTIME_UI_HTML,
    ).into_response()
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
        .route("/benchmark", post(handle_benchmark))
        .route("/clear", get(handle_clear).post(handle_clear))
        .route("/sse", get(handle_mcp_sse))
        .route("/message", post(handle_mcp_message))
        .route("/messages", post(handle_mcp_message))
        .route("/mcp", post(handle_mcp_direct))
        .route("/plugin/version", get(handle_plugin_version))
        .route("/plugin/code.js", get(handle_plugin_code))
        .route("/plugin/ui.html", get(handle_plugin_ui))
        .route("/assets/*path", get(handle_asset_serve))
        // Large design trees arrive through /response; axum's 2MB default would 413 them.
        .layer(axum::extract::DefaultBodyLimit::max(64 << 20))
        .layer(cors)
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::BridgeState;
    use serde_json::json;

    #[tokio::test]
    async fn rejects_mcp_tool_names_before_plugin_dispatch() {
        let state = BridgeState::new(0);
        let err = state
            .send_operation("figma_prepare_design", json!({}), None)
            .await
            .expect_err("MCP tool names must never be queued as plugin operations");

        assert!(err.contains("call it through MCP tools/call"));
    }

    #[test]
    fn runtime_hash_is_stable_and_nonempty() {
        let first = super::runtime_code_hash();
        assert_eq!(first, super::runtime_code_hash());
        assert_eq!(first.len(), 16);
    }

    #[tokio::test]
    async fn resolves_connected_session_by_file_name() {
        let state = BridgeState::new(0);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut session = super::Session::new("session-a".to_string(), Some("Checkout".to_string()));
        session.ws_tx = Some(tx);
        state.inner.lock().await.sessions.insert(session.id.clone(), session);

        assert_eq!(state.resolved_session_id(Some("checkout")).await, "session-a");
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[tokio::test]
    async fn served_assets_and_mcp_handshake_match_binary_version() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let router = create_router(BridgeState::new(port));
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        let client = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{port}");
        let version = env!("CARGO_PKG_VERSION");
        let root: Value = client.get(format!("{base}/")).send().await.unwrap().json().await.unwrap();
        let plugin: Value = client.get(format!("{base}/plugin/version")).send().await.unwrap().json().await.unwrap();
        assert_eq!(root["version"], version);
        assert_eq!(plugin["version"], version);
        assert_eq!(plugin["protocolVersion"], 3);
        let ui = client.get(format!("{base}/plugin/ui.html")).send().await.unwrap();
        assert_eq!(ui.headers()["etag"], RUNTIME_UI_ETAG);
        assert!(ui.text().await.unwrap().contains(&format!("id=\"runtime-version\">v{version}</span>")));
        let code = client.get(format!("{base}/plugin/code.js")).send().await.unwrap();
        assert_eq!(code.headers()["etag"], RUNTIME_CODE_ETAG);
        assert!(code.text().await.unwrap().contains(&format!("runtimeVersion: \"{version}\"")));
        let rpc: Value = client.post(format!("{base}/mcp")).json(&json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize", "params":{}
        })).send().await.unwrap().json().await.unwrap();
        assert_eq!(rpc["result"]["serverInfo"]["version"], version);
        server.abort();
    }

    #[tokio::test]
    async fn unsupported_operation_is_rejected_without_queueing() {
        let state = BridgeState::new(0);
        {
            let mut inner = state.inner.lock().await;
            let mut session = Session::new("test".into(), None);
            session.last_poll_at = now_ms();
            session.operations = Some(vec!["status".into()]);
            session.runtime_version = Some("old-runtime".into());
            session.protocol_version = Some(2);
            inner.sessions.insert("test".into(), session);
        }
        let error = state.send_operation("not_an_operation", json!({}), Some("test")).await.unwrap_err();
        assert!(error.contains("old-runtime"));
        assert!(error.contains("No request dispatched"));
        let inner = state.inner.lock().await;
        assert!(inner.sessions["test"].queue.is_empty());
        assert!(inner.op_to_session.is_empty());
    }

    #[tokio::test]
    async fn legacy_export_node_is_normalized_before_dispatch() {
        let state = BridgeState::new(0);
        {
            let mut inner = state.inner.lock().await;
            let mut session = Session::new("test".into(), None);
            session.last_poll_at = now_ms();
            inner.sessions.insert("test".into(), session);
        }
        let cloned = state.clone();
        let task = tokio::spawn(async move { cloned.send_operation("export_node", json!({"format":"SVG", "id":"1:2"}), Some("test")).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !state.inner.lock().await.sessions["test"].queue.is_empty() { break; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        {
            let mut inner = state.inner.lock().await;
            let session = inner.sessions.get_mut("test").unwrap();
            assert_eq!(session.queue[0].operation, "export_svg");
            let id = session.queue[0].id.clone();
            session.pending.remove(&id).unwrap().sender.send(Ok(json!({"svg":"<svg/>"}))).unwrap();
        }
        assert_eq!(task.await.unwrap().unwrap()["svg"], "<svg/>");
        assert!(state.send_operation("export_node", json!({"format":"PDF"}), Some("test")).await.unwrap_err().contains("supports PNG"));
    }

    #[test]
    fn bundled_ui_and_runtime_match_binary_version() {
        let version = env!("CARGO_PKG_VERSION");
        assert!(PLUGIN_RUNTIME_CODE_JS.contains(&format!("runtimeVersion: \"{version}\"")));
        assert!(PLUGIN_RUNTIME_UI_HTML.contains(&format!("id=\"runtime-version\">v{version}</span>")));
        assert!(PLUGIN_RUNTIME_UI_HTML.contains("updateRuntimeVersion(serverVer)"));
    }
}
