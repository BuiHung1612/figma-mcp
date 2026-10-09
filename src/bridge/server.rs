use axum::extract::ws::Message;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

use super::session::{
    PendingOp, PollResponse, QueuedOp, Session, SessionInfo, SessionStats, MAX_QUEUE,
};
use crate::mcp::protocol::JsonRpcResponse;

pub const DEFAULT_PORT: u16 = 41730;
pub const PORT_RANGE: u16 = 10;
pub const LONG_POLL_MS: u64 = 8_000;
pub const DEFAULT_OP_TIMEOUT_MS: u64 = 60_000;

pub(super) fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn get_op_timeout(op: &str) -> u64 {
    match op {
        "screenshot" | "scan_design" | "export_image" | "batch" => 90_000,
        "export_svg" | "get_design" => 60_000,
        _ => DEFAULT_OP_TIMEOUT_MS,
    }
}

fn is_read_operation(operation: &str) -> bool {
    let key: String = operation.chars().filter(char::is_ascii_alphanumeric).flat_map(char::to_lowercase).collect();
    matches!(key.as_str(), "status" | "query" | "listpages" | "listcomponents" | "getselection" | "getdesign" |
        "getpagenodes" | "getnodedetail" | "getdesigncontext" | "getcss" | "getcomponentmap" | "getunmappedcomponents" |
        "getstyles" | "getvariables" | "getvariabletokens" | "gettokens" | "getlocalcomponents" | "getviewport" |
        "screenshot" | "exportsvg" | "exportimage" | "exportassets" | "scandesign" | "searchnodes" | "indexscan" |
        "getcomponentproperties" | "getreactions" | "readnodes")
}

/// Viewport/selection changes touch no document nodes, so they keep the index ready.
fn is_mutating_operation(operation: &str) -> bool {
    let key: String = operation.chars().filter(char::is_ascii_alphanumeric).flat_map(char::to_lowercase).collect();
    !is_read_operation(operation) && !matches!(key.as_str(), "setviewport" | "setselection")
}

fn read_cache_key(operation: &str, params: &Value) -> Option<String> {
    // Cache exact live responses, never infer a detail contract from compact nodes.
    // ponytail: 64 entries / 32MB per tab, sized by serializing under the bridge lock; size outside the lock if inserts show up in profiles.
    if matches!(operation, "get_styles" | "get_variables" | "get_variable_tokens")
        || (matches!(operation, "get_design" | "get_node_detail" | "get_design_context" | "read_nodes")
        && params.get("cursor").is_none()
        && params.get("id").or_else(|| params.get("nodeId")).and_then(Value::as_str).is_some()) {
        Some(format!("{operation}:{}", params))
    } else { None }
}

#[cfg(test)]
mod read_cache_tests {
    use super::*;

    #[tokio::test]
    async fn stale_index_completion_is_rejected_and_requests_resync() {
        let state = BridgeState::new(0);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut session = Session::new("s".into(), None);
        session.ws_tx = Some(tx);
        state.inner.lock().await.sessions.insert("s".into(), session);
        state.receive_index_event("s", &json!({"type":"index-start","scanId":"1","pageId":"page",
            "revision":0,"scope":{"id":"page"}})).await;
        state.inner.lock().await.sessions.get_mut("s").unwrap().node_revision = 1;
        state.receive_index_event("s", &json!({"type":"index-update","scanId":"1","pageId":"page",
            "revision":0,"data":{"complete":true}})).await;
        let Message::Text(ack) = rx.try_recv().unwrap() else { panic!("expected acknowledgement") };
        let ack: Value = serde_json::from_str(&ack).unwrap();
        assert_eq!(ack["success"], false);
        assert_eq!(ack["error"], "Document changed during sync");
        assert!(state.get_index_stats(Some("s")).await.is_none());
        let Message::Text(request) = rx.try_recv().unwrap() else { panic!("expected resync") };
        assert_eq!(serde_json::from_str::<Value>(&request).unwrap()["type"], "index-resync");
    }

    #[tokio::test]
    async fn scoped_snapshots_commit_atomically_and_patch_gaps_request_resync() {
        let state = BridgeState::new(0);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut session = Session::new("s".into(), None);
        session.ws_tx = Some(tx);
        let mut idx = crate::bridge::index::FigmaIndex::from_raw("s", "f",
            &json!([{"id":"a","type":"FRAME"},{"id":"b","type":"FRAME"}]), None, None, None, 0);
        idx.page_id = Some("page".into());
        session.index = Some(idx);
        state.inner.lock().await.sessions.insert("s".into(), session);
        state.receive_index_event("s", &json!({"type":"index-start","scanId":"1","pageId":"page","revision":0,
            "scope":{"id":"a","depth":256,"expandInstances":false}})).await;
        state.receive_index_event("s", &json!({"type":"index-chunk","scanId":"old","pageId":"page","revision":0,
            "nodes":[{"id":"bad","type":"TEXT"}]})).await;
        state.receive_index_event("s", &json!({"type":"index-chunk","scanId":"1","pageId":"page","revision":0,
            "nodes":[{"id":"a","type":"FRAME","parentId":null,"childIds":["t"]},
                {"id":"t","type":"TEXT","parentId":"a","content":"new"}]})).await;
        assert_eq!(state.get_index_stats(Some("s")).await.unwrap().total_nodes, 2);
        state.receive_index_event("s", &json!({"type":"index-update","scanId":"1","pageId":"page","revision":0,
            "data":{"complete":true}})).await;
        let Message::Text(ack) = rx.try_recv().unwrap() else { panic!("expected index acknowledgement") };
        let ack: Value = serde_json::from_str(&ack).unwrap();
        assert_eq!(ack["type"], "index-ack");
        assert_eq!(ack["scanId"], "1");
        assert_eq!(ack["success"], true);
        let inner = state.inner.lock().await;
        let idx = inner.sessions["s"].index.as_ref().unwrap();
        assert_eq!(idx.nodes.len(), 3);
        assert!(idx.nodes.contains_key("b"));
        assert!(!idx.nodes.contains_key("bad"));
        drop(inner);
        state.receive_index_event("s", &json!({"type":"node-patch","pageId":"page","baseRevision":0,"revision":1,
            "patches":[{"kind":"update","id":"t","values":{"content":"patched","indexDetail":"minimal"}}]})).await;
        assert_eq!(state.inner.lock().await.sessions["s"].index.as_ref().unwrap().nodes["t"].characters.as_deref(), Some("patched"));
        state.receive_index_event("s", &json!({"type":"node-patch","pageId":"page","baseRevision":2,"revision":3,"patches":[]})).await;
        let Message::Text(request) = rx.try_recv().unwrap() else { panic!("expected resync") };
        assert_eq!(serde_json::from_str::<Value>(&request).unwrap()["type"], "index-resync");
        assert!(state.inner.lock().await.sessions["s"].index.as_ref().unwrap().dirty);
    }

    #[tokio::test]
    async fn writes_request_resync_and_index_becomes_ready_again() {
        let state = BridgeState::new(0);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut session = Session::new("s".into(), None);
        session.ws_tx = Some(tx);
        let mut idx = crate::bridge::index::FigmaIndex::from_raw("s", "f", &json!([{"id":"a","type":"FRAME"}]), None, None, None, 0);
        idx.page_id = Some("page".into());
        session.index = Some(idx);
        state.inner.lock().await.sessions.insert("s".into(), session);
        let reply = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<Message>, state: BridgeState| {
            let Message::Text(request) = rx.try_recv().unwrap() else { panic!("expected request") };
            let id = serde_json::from_str::<Value>(&request).unwrap()["id"].as_str().unwrap().to_string();
            tokio::spawn(async move {
                let mut inner = state.inner.lock().await;
                inner.sessions.get_mut("s").unwrap().pending.remove(&id).unwrap().sender.send(Ok(json!({}))).unwrap();
            })
        };
        // Viewport changes keep the index ready and request nothing.
        let view = tokio::spawn({ let s = state.clone(); async move { s.send_operation("set_viewport", json!({}), Some("s")).await } });
        while rx.is_empty() { tokio::task::yield_now().await; }
        reply(&mut rx, state.clone()).await.unwrap();
        view.await.unwrap().unwrap();
        assert!(rx.try_recv().is_err());
        assert!(state.inner.lock().await.sessions["s"].index.as_ref().unwrap().is_ready());
        // A write marks the index dirty, then asks for a resync once it settles.
        let write = tokio::spawn({ let s = state.clone(); async move { s.send_operation("create", json!({}), Some("s")).await } });
        while rx.is_empty() { tokio::task::yield_now().await; }
        reply(&mut rx, state.clone()).await.unwrap();
        write.await.unwrap().unwrap();
        let Message::Text(request) = rx.try_recv().unwrap() else { panic!("expected resync") };
        assert_eq!(serde_json::from_str::<Value>(&request).unwrap()["type"], "index-resync");
        assert!(state.inner.lock().await.op_to_session.is_empty());
        for event in [json!({"type":"index-start","scanId":"2","pageId":"page","revision":0,"scope":{"id":"page"}}),
            json!({"type":"index-chunk","scanId":"2","pageId":"page","revision":0,"nodes":[{"id":"a","type":"FRAME","parentId":null}]}),
            json!({"type":"index-update","scanId":"2","pageId":"page","revision":0,"data":{"complete":true}})] {
            state.receive_index_event("s", &event).await;
        }
        let inner = state.inner.lock().await;
        assert!(inner.sessions["s"].index.as_ref().unwrap().is_ready());
        assert!(!inner.sessions["s"].resync_requested);
    }

    #[test]
    fn cache_keys_preserve_detail_contract_and_exclude_selection_and_exports() {
        assert_ne!(read_cache_key("get_design", &json!({"id":"1:1", "detail":"full"})),
            read_cache_key("get_design", &json!({"id":"1:1", "detail":"compact"})));
        for op in ["get_selection", "export_image", "query", "get_design"] {
            assert!(read_cache_key(op, &json!({})).is_none());
        }
    }

    #[tokio::test]
    async fn token_reads_reuse_index_and_live_responses_without_dispatch() {
        let state = BridgeState::new(0);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut session = Session::new("s".into(), None);
        session.ws_tx = Some(tx);
        let styles = json!({"schemaVersion": 2, "paintStyles": []});
        session.index = Some(crate::bridge::index::FigmaIndex::from_raw(
            "s", "f", &json!([]), Some(&styles), None, None, 0));
        state.inner.lock().await.sessions.insert("s".into(), session);
        assert_eq!(state.send_operation("get_styles", json!({}), Some("s")).await.unwrap(), styles);
        assert!(rx.try_recv().is_err());

        state.mark_index_dirty("s").await;
        let dispatch = state.clone();
        let read = tokio::spawn(async move { dispatch.send_operation("get_styles", json!({}), Some("s")).await });
        let Message::Text(request) = rx.recv().await.unwrap() else { panic!("expected request") };
        let request: Value = serde_json::from_str(&request).unwrap();
        let mut inner = state.inner.lock().await;
        let session = inner.sessions.get_mut("s").unwrap();
        session.pending.remove(request["id"].as_str().unwrap()).unwrap().sender.send(Ok(styles.clone())).unwrap();
        drop(inner);
        assert_eq!(read.await.unwrap().unwrap(), styles);
        assert_eq!(state.send_operation("get_styles", json!({}), Some("s")).await.unwrap(), styles);
        assert!(rx.try_recv().is_err());
        assert_eq!(state.inner.lock().await.sessions["s"].read_cache.len(), 1);
        state.mark_index_dirty("s").await;
        assert!(state.inner.lock().await.sessions["s"].read_cache.is_empty());
    }

    #[tokio::test]
    async fn node_cache_is_session_local_and_writes_invalidate_before_dispatch() {
        let state = BridgeState::new(0);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut session = Session::new("s".into(), None);
        session.ws_tx = Some(tx);
        session.index = Some(crate::bridge::index::FigmaIndex::from_raw(
            "s", "f", &json!([{"id":"1:1", "name":"Card", "type":"FRAME", "indexDetail":"minimal"}]), None, None, None, 0));
        let params = json!({"id":"1:1", "detail":"full"});
        let data = json!({"id":"1:1", "resolvedPaints": []});
        session.cache_read(read_cache_key("get_design_context", &params).unwrap(), data.clone());
        state.inner.lock().await.sessions.insert("s".into(), session);
        assert!(state.get_index_node(Some("s"), "1:1").await.is_none());
        assert_eq!(state.search_index_nodes(Some("s"), "Card", None, 10).await.unwrap().len(), 1);
        assert_eq!(state.send_operation("get_design_context", params, Some("s")).await.unwrap(), data);
        assert!(rx.try_recv().is_err());
        let dispatch = state.clone();
        let write = tokio::spawn(async move { dispatch.send_operation("modify", json!({"id":"1:1", "name":"New"}), Some("s")).await });
        let Message::Text(request) = rx.recv().await.unwrap() else { panic!("expected request") };
        let request: Value = serde_json::from_str(&request).unwrap();
        let mut inner = state.inner.lock().await;
        let session = inner.sessions.get_mut("s").unwrap();
        assert!(session.read_cache.is_empty());
        assert!(session.index.as_ref().unwrap().pending_nodes.contains("1:1"));
        session.pending.remove(request["id"].as_str().unwrap()).unwrap().sender.send(Ok(json!({}))).unwrap();
        drop(inner);
        write.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn invalidation_during_read_does_not_cache_stale_response() {
        let state = BridgeState::new(0);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut session = Session::new("s".into(), None);
        session.ws_tx = Some(tx);
        state.inner.lock().await.sessions.insert("s".into(), session);
        let dispatch = state.clone();
        let read = tokio::spawn(async move { dispatch.send_operation("get_variables", json!({}), Some("s")).await });
        let Message::Text(request) = rx.recv().await.unwrap() else { panic!("expected request") };
        let request: Value = serde_json::from_str(&request).unwrap();
        state.mark_index_dirty("s").await;
        state.inner.lock().await.sessions.get_mut("s").unwrap().pending
            .remove(request["id"].as_str().unwrap()).unwrap().sender.send(Ok(json!({}))).unwrap();
        read.await.unwrap().unwrap();
        assert!(state.inner.lock().await.sessions["s"].read_cache.is_empty());
    }
}

pub struct BridgeInner {
    pub tool_timings: HashMap<String, (u64, u64)>,
    pub port: u16,
    pub sessions: HashMap<String, Session>,
    pub op_to_session: HashMap<String, String>,
    pub global_stats: SessionStats,
    pub mcp_sse_clients: HashMap<String, tokio::sync::mpsc::UnboundedSender<JsonRpcResponse>>,
    pub tasks: HashMap<String, TaskBinding>,
    pub last_cleanup_at: u64,
}

impl BridgeInner {
    /// Drop idle disconnected sessions (and their tasks) at most every 30s.
    pub fn expire_sessions(&mut self, now: u64) {
        if now.saturating_sub(self.last_cleanup_at) <= 30_000 { return; }
        self.last_cleanup_at = now;
        self.sessions.retain(|_, s| {
            s.is_connected() || !s.queue.is_empty() || !s.pending.is_empty()
                || now.saturating_sub(s.last_poll_at) < super::session::SESSION_EXPIRE_MS
        });
        let sessions: std::collections::HashSet<_> = self.sessions.keys().cloned().collect();
        self.tasks.retain(|_, task| sessions.contains(&task.session_id));
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskBinding {
    pub task_id: String,
    pub session_id: String,
    pub frame_id: String,
    pub document_id: String,
}

#[derive(Clone)]
pub struct BridgeState {
    pub inner: Arc<Mutex<BridgeInner>>,
    pub task_scope: Option<TaskBinding>,
}

impl BridgeState {
    pub fn new(port: u16) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BridgeInner {
                tool_timings: HashMap::new(),
                port,
                sessions: HashMap::new(),
                op_to_session: HashMap::new(),
                global_stats: SessionStats::default(),
                mcp_sse_clients: HashMap::new(),
                tasks: HashMap::new(),
                last_cleanup_at: 0,
            })),
            task_scope: None,
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
                document_id: s.document_id.clone(),                file_name: s.file_name.clone(),
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
                document_id: s.document_id.clone(),            file_name: s.file_name.clone(),
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
        mut params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        if operation.starts_with("figma_") {
            return Err(format!(
                "Invalid plugin operation '{}'. This is an MCP tool name; call it through MCP tools/call instead of figma_write or the Figma bridge.",
                operation
            ));
        }

        // Normalize before queueing so even an older plugin never receives the
        // unsupported export_node spelling. No unsupported format is discarded.
        let operation = if operation == "export_node" || operation == "exportNode" {
            match params["format"].as_str().unwrap_or("PNG").to_uppercase().as_str() {
                "SVG" => "export_svg",
                "PNG" => { params["format"] = json!("PNG"); "export_image" }
                "JPG" | "JPEG" => { params["format"] = json!("JPG"); "export_image" }
                _ => return Err("export_node supports PNG, JPG/JPEG or SVG".into()),
            }
        } else { operation };
        let (rx, op_id, timeout_ms, target_sid, cache_key, cache_revision) = {
            let mut inner = self.inner.lock().await;

            let sid = Self::checked_session_id(&inner, session_id)?;

            if self.task_scope.is_none() && !is_read_operation(operation)
                && operation != "task_start" && operation != "task_end"
                && inner.tasks.values().any(|task| task.document_id == inner.sessions.get(&sid).and_then(|s| s.document_id.as_deref()).unwrap_or(&sid)) {
                return Err("This tab has active frame tasks. Pass taskId through MCP for writes.".into());
            }

            let session = inner
                .sessions
                .entry(sid.clone())
                .or_insert_with(|| Session::new(sid.clone(), None));

            if let Some(task) = &self.task_scope {
                if task.session_id != sid { return Err("Task cannot switch tabs".into()); }
                if !params.is_object() && !params.is_array() { return Err("Scoped operations require object/array params".into()); }
                if params.is_array() { params = json!({"operations": params}); }
                params["_taskId"] = json!(task.task_id);
            } else {
                if let Some(obj) = params.as_object_mut() { obj.remove("_taskId"); }
            }

            if matches!(operation, "read_nodes" | "index_scan") && session.protocol_version.is_some_and(|version| version < 3) {
                return Err("Node API v4 requires plugin protocol 3. Update and restart both the server and the Figma plugin.".to_string());
            }
            if let Some(operations) = &session.operations {
                let key = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
                let canonical = match key(operation).as_str() {
                    "getnode" | "getnodeinfo" | "nodeinfo" | "nodedetail" => "get_node_detail",
                    "inspect" | "inspectnode" | "designcontext" => "get_design_context",
                    "selection" => "get_selection", "pagenodes" => "get_page_nodes",
                    "styles" => "get_styles", "variables" | "tokens" | "gettokens" => "get_variables",
                    "components" => "get_local_components",
                    _ => operation,
                };
                if !operations.iter().any(|op| key(op) == key(canonical)) {
                    return Err(format!("Unsupported operation '{operation}' for session {sid}; runtime {}, protocol {}. No request dispatched. Update/restart the Figma plugin or use an advertised operation.",
                        session.runtime_version.as_deref().unwrap_or("unknown"), session.protocol_version.unwrap_or(0)));
                }
            }
            // Only cache node reads for indexed active-page nodes: other pages
            // are not covered by the plugin's nodechange subscription.
            let node_read = matches!(operation, "get_design" | "get_node_detail" | "get_design_context" | "read_nodes");
            let active_node = params.get("id").or_else(|| params.get("nodeId")).and_then(Value::as_str)
                .is_some_and(|id| session.index.as_ref().is_some_and(|idx| idx.is_ready() && idx.nodes.contains_key(id)));
            let cache_key = if self.task_scope.is_none() && (!node_read || active_node) {
                read_cache_key(operation, &params)
            } else { None };
            let writes_pending = session.pending.values().any(|p| is_mutating_operation(&p.op.operation));
            if !writes_pending {
                if let Some(value) = cache_key.as_ref().and_then(|key| session.cached_read(key)) {
                    session.cache_hits += 1;
                    tracing::debug!(operation, session_id = %sid, "Rust read cache hit");
                    drop(inner);
                    return Ok(Value::clone(&value));
                }
                if let Some(idx) = session.index.as_ref().filter(|idx| !idx.tokens_dirty && !idx.dirty && idx.stats.indexed_at_ms > 0) {
                    let cached = match operation {
                        "get_styles" => idx.raw_styles.as_ref(),
                        "get_variables" | "get_variable_tokens" => idx.raw_variables.as_ref(),
                        _ => None,
                    };
                    if cache_key.is_some() {
                        if let Some(value) = cached.filter(|v| v["schemaVersion"] == 2) {
                            let value = value.clone();
                            session.cache_hits += 1;
                            return Ok(value);
                        }
                    }
                }
            }
            if is_mutating_operation(operation) {
                if operation == "modify" {
                    if let Some(id) = params["id"].as_str() {
                        session.invalidate_nodes(&[id.to_string()]);
                        if let Some(idx) = &mut session.index { idx.pending_nodes.insert(id.to_string()); }
                    } else { session.invalidate_reads(); if let Some(idx) = &mut session.index { idx.mark_dirty(); } }
                } else {
                    session.invalidate_reads();
                    if let Some(idx) = &mut session.index { idx.mark_dirty(); }
                }
            }
            let cache_revision = session.cache_revision;
            if cache_key.is_some() { session.cache_misses += 1; }
            session.bridge_calls += 1;
            if session.queue.len() >= MAX_QUEUE {
                return Err("Queue full — is the Figma plugin running?".to_string());
            }

            let timeout_ms = get_op_timeout(operation);
            let op_id = format!("{}-{}", now_ms(), Uuid::new_v4());

            tracing::debug!(request_id = %op_id, operation, session_id = %sid,
                runtime_version = ?session.runtime_version, protocol_version = ?session.protocol_version,
                "Dispatching plugin operation");
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
            (rx, op_id, timeout_ms, sid, cache_key, cache_revision)
        };

        // Await with timeout
        let started = std::time::Instant::now();
        let response = tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await;
        {
            let mut inner = self.inner.lock().await;
            // Every exit path (reply, timeout, dropped sender) releases the op mapping.
            inner.op_to_session.remove(&op_id);
            if let Some(session) = inner.sessions.get_mut(&target_sid) {
                session.bridge_time_ms += started.elapsed().as_millis() as u64;
                if response.is_err() {
                    session.pending.remove(&op_id);
                    session.queue.retain(|q| q.id != op_id);
                }
                // A write left the index dirty; once writes settle, ask the plugin
                // for a fresh page snapshot so the index becomes ready again.
                if is_mutating_operation(operation) && !session.resync_requested
                    && session.index.as_ref().is_some_and(|idx| idx.dirty)
                    && !session.pending.values().any(|p| is_mutating_operation(&p.op.operation)) {
                    if let Some(tx) = &session.ws_tx {
                        session.resync_requested = tx.send(Message::Text(json!({"type":"index-resync"}).to_string())).is_ok();
                    }
                }
            }
        }
        match response {
            Ok(Ok(val)) => {
                if operation == "get_design" && cache_key.is_some() {
                    if let Ok(data) = &val {
                        let mut inner = self.inner.lock().await;
                        if let Some(session) = inner.sessions.get_mut(&target_sid) {
                            if session.cache_revision == cache_revision {
                                if let Some(idx) = &mut session.index {
                                    if let Some(tree) = data.get("tree").filter(|v| v.is_object()) { idx.cache_subtree(tree); }
                                }
                            }
                        }
                    }
                }
                if let (Some(key), Ok(data)) = (cache_key, &val) {
                    let mut inner = self.inner.lock().await;
                    if let Some(session) = inner.sessions.get_mut(&target_sid) {
                        if session.cache_revision == cache_revision
                            && !session.pending.values().any(|p| is_mutating_operation(&p.op.operation))
                            && (operation != "read_nodes" || data["nextCursor"].is_null())
                        {
                            session.cache_read(key, data.clone());
                        }
                    }
                }
                if operation == "get_local_components" {
                    if let Ok(data) = &val {
                        let mut inner = self.inner.lock().await;
                        if let Some(idx) = inner.sessions.get_mut(&target_sid).and_then(|s| s.index.as_mut()) {
                            if idx.is_ready() { idx.cache_components(data); }
                        }
                    }
                }
                if operation == "read_nodes" {
                    if let Ok(data) = &val {
                        let mut inner = self.inner.lock().await;
                        if let Some(session) = inner.sessions.get_mut(&target_sid) {
                            if session.cache_revision == cache_revision {
                                if let Some(idx) = &mut session.index {
                                    if idx.page_id.as_deref() == data["pageId"].as_str()
                                        && data["revision"].as_u64().is_some_and(|rev| rev >= session.node_revision) {
                                        if let Some(nodes) = data["nodes"].as_array() { idx.merge_projected_nodes(nodes); }
                                        idx.record_scope(&data["scope"], data["complete"] == true);
                                    }
                                }
                            }
                        }
                    }
                }
                val
            },
            Ok(Err(_)) => Err("Operation cancelled or bridge closed".to_string()),
            Err(_) => Err(format!("Operation \"{}\" timed out after {}ms", operation, timeout_ms)),
        }
    }

    pub async fn update_index(&self, session_id: &str, index: crate::bridge::index::FigmaIndex) {
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session.invalidate_reads();
            session.index = Some(index);
        }
    }

    pub async fn mark_index_dirty(&self, session_id: &str) {
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session.invalidate_reads();
            if let Some(ref mut idx) = session.index {
                idx.mark_dirty();
            }
        }
    }

    pub(super) async fn invalidate_changed_nodes(&self, sid: &str, ids: &[String]) {
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get_mut(sid) {
            session.invalidate_nodes(ids);
            if let Some(idx) = &mut session.index {
                idx.pending_nodes.extend(ids.iter().cloned());
                idx.stats.components_indexed = false;
                idx.raw_components = None;
            }
        }
    }

    pub(super) async fn update_changed_nodes(&self, sid: &str, page: Option<&str>, nodes: &[Value], deleted: &[String]) {
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get_mut(sid) {
            if session.index.as_ref().is_some_and(|idx| idx.page_id.as_deref() != page) { return; }
            let ids: Vec<_> = nodes.iter().flat_map(|n| [n["id"].as_str(),n["parentId"].as_str()])
                .flatten().filter(|id| session.index.as_ref().is_some_and(|idx| idx.nodes.contains_key(*id)))
                .map(str::to_owned).chain(deleted.iter().cloned()).collect();
            session.invalidate_nodes(&ids);
            if let Some(idx) = &mut session.index {
                for id in deleted { idx.remove_node(id); }
                for node in nodes { idx.upsert_node(node); }
            }
        }
    }

    pub(super) async fn receive_index_event(&self, sid: &str, event: &Value) {
        let mut inner = self.inner.lock().await;
        let Some(session) = inner.sessions.get_mut(sid) else { return };
        let page = event["pageId"].as_str();
        let revision = event["revision"].as_u64();
        let mut resync = false;
        match event["type"].as_str() {
            Some("nodes-invalidated") => {
                if session.index.as_ref().is_some_and(|idx| idx.page_id.as_deref() != page) {
                    return;
                }
                if let Some(rev) = revision { session.node_revision = session.node_revision.max(rev); }
                let ids: Vec<String> = event["ids"].as_array().map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
                if event["reset"] == true {
                    session.invalidate_reads();
                    if let Some(idx) = &mut session.index { idx.mark_dirty(); }
                    resync = true;
                } else {
                    let affected: Vec<String> = ids.iter().map(|id| {
                        if session.index.as_ref().is_some_and(|idx| idx.nodes.contains_key(id)) { return id.clone(); }
                        event["paths"][id].as_array().and_then(|path| path.iter().filter_map(Value::as_str)
                            .find(|ancestor| session.index.as_ref().is_some_and(|idx| idx.nodes.contains_key(*ancestor))))
                            .unwrap_or(id).to_string()
                    }).collect();
                    session.invalidate_nodes(&affected);
                    if let Some(idx) = &mut session.index {
                        let known: Vec<_> = ids.into_iter().filter(|id| idx.nodes.contains_key(id)).collect();
                        idx.pending_nodes.extend(known);
                    } else { resync = true; }
                }
            }
            Some("node-patch") => {
                if session.index.as_ref().is_some_and(|idx| idx.page_id.as_deref() != page) { return; }
                if let (Some(base), Some(rev), Some(patches)) = (event["baseRevision"].as_u64(), revision, event["patches"].as_array()) {
                    if let Some(idx) = &mut session.index {
                        resync = !idx.apply_patch_batch(base, rev, patches);
                        session.node_revision = session.node_revision.max(rev);
                    } else { resync = true; }
                } else { resync = true; }
            }
            Some("index-start") => {
                if let (Some(scan), Some(rev), Some(page)) = (event["scanId"].as_str(), revision, page) {
                    if rev < session.node_revision { resync = true; }
                    else {
                        let mut idx = crate::bridge::index::FigmaIndex::from_raw(sid,
                            event["fileName"].as_str().unwrap_or("unknown"), &json!([]), None, None, None,
                            event["startMs"].as_u64().unwrap_or_else(now_ms));
                        idx.page_id = Some(page.to_string());
                        idx.revision = rev;
                        session.pending_index = Some((scan.to_string(), idx, event["scope"].clone()));
                    }
                } else { resync = true; }
            }
            Some("index-chunk") => {
                if let Some((scan, idx, _)) = &mut session.pending_index {
                    if Some(scan.as_str()) == event["scanId"].as_str() && idx.page_id.as_deref() == page && Some(idx.revision) == revision {
                        if let Some(nodes) = event["nodes"].as_array() { idx.merge_chunk(nodes); }
                    }
                } else { resync = true; }
            }
            Some("index-update") => {
                let matches = session.pending_index.as_ref().is_some_and(|(scan, idx, _)|
                    Some(scan.as_str()) == event["scanId"].as_str() && idx.page_id.as_deref() == page && Some(idx.revision) == revision);
                if !matches {
                    if let Some(tx) = &session.ws_tx {
                        let _ = tx.send(Message::Text(json!({"type":"index-ack", "scanId":event["scanId"],
                            "pageId":event["pageId"], "success":false, "error":"Index stream no longer active"}).to_string()));
                    }
                    return;
                }
                let (_, mut nodes, scope) = session.pending_index.take().unwrap();
                session.resync_requested = false;
                if nodes.revision < session.node_revision { resync = true; }
                else {
                    let data = &event["data"];
                    let mut metadata = crate::bridge::index::FigmaIndex::from_raw(sid,
                        event["fileName"].as_str().unwrap_or("unknown"), &json!([]), data.get("styles"),
                        data.get("variables"), data.get("components"), event["startMs"].as_u64().unwrap_or_else(now_ms));
                    let page_snapshot = scope["id"].as_str() == page;
                    session.invalidate_nodes(&[scope["id"].as_str().unwrap_or("").to_string()]);
                    if page_snapshot || session.index.as_ref().is_none_or(|idx| idx.page_id.as_deref() != page) {
                        metadata.nodes = std::mem::take(&mut nodes.nodes);
                        metadata.top_level_frames = std::mem::take(&mut nodes.top_level_frames);
                        metadata.page_id = nodes.page_id;
                        metadata.revision = nodes.revision;
                        metadata.stats.nodes_truncated = data["nodesTruncated"] == true;
                        metadata.stats.total_nodes = metadata.nodes.len();
                        metadata.record_scope(&scope, data["complete"] == true);
                        session.index = Some(metadata);
                        session.invalidate_reads();
                    } else if let Some(idx) = &mut session.index {
                        idx.drop_missing_children(&nodes.nodes);
                        for node in nodes.nodes.into_values() {
                            if let Some(data) = node.full_data { idx.merge_projected_nodes(&[data]); }
                        }
                        if data["components"].is_object() { idx.cache_components(&data["components"]); }
                        idx.record_scope(&scope, data["complete"] == true);
                        idx.stats.duration_ms = metadata.stats.duration_ms;
                    }
                    session.node_revision = session.node_revision.max(revision.unwrap_or(0));
                }
                if let Some(tx) = &session.ws_tx {
                    let _ = tx.send(Message::Text(json!({"type":"index-ack", "scanId":event["scanId"],
                        "pageId":event["pageId"], "success":!resync,
                        "error":if resync { Some("Document changed during sync") } else { None }}).to_string()));
                }
            }
            Some("index-abort") => {
                if session.pending_index.as_ref().is_some_and(|(scan, _, _)| Some(scan.as_str()) == event["scanId"].as_str()) {
                    session.pending_index = None;
                }
                session.resync_requested = false;
                resync = event["retry"] == true;
            }
            _ => return,
        }
        if resync && !session.resync_requested {
            session.invalidate_reads();
            if let Some(idx) = &mut session.index { idx.mark_dirty(); }
            if let Some(tx) = &session.ws_tx {
                let _ = tx.send(Message::Text(json!({"type":"index-resync"}).to_string()));
                session.resync_requested = true;
            }
        }
    }

    pub async fn get_index_stats(&self, session_id: Option<&str>) -> Option<crate::bridge::index::IndexStats> {
        let inner = self.inner.lock().await;
        let sid = Self::resolve_session_id(&inner, session_id);

        inner.sessions.get(&sid).and_then(|s| s.index.as_ref().map(|idx| {
            let mut stats = idx.stats.clone();
            stats.complete &= idx.pending_nodes.is_empty() && !idx.dirty;
            stats
        }))
    }

    pub async fn get_index_node(&self, session_id: Option<&str>, node_id: &str) -> Option<crate::bridge::index::IndexNode> {
        let inner = self.inner.lock().await;
        let sid = Self::resolve_session_id(&inner, session_id);

        inner.sessions.get(&sid).and_then(|s| s.index.as_ref().filter(|idx| idx.is_ready()).and_then(|idx| idx.get_node(node_id).filter(|node| node.has_details()).cloned()))
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
            s.index.as_ref().filter(|idx| idx.is_ready()).map(|idx| {
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
            s.index.as_ref().filter(|idx| idx.is_ready() && idx.stats.components_indexed).map(|idx| {
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

    pub async fn update_selection(&self, session_id: &str, sel: crate::bridge::session::ActiveSelection) {
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session.active_selection = Some(sel);
        }
    }

    pub async fn get_active_selection(&self, session_id: Option<&str>) -> Option<crate::bridge::session::ActiveSelection> {
        let inner = self.inner.lock().await;
        let sid = Self::resolve_session_id(&inner, session_id);
        inner.sessions.get(&sid).and_then(|s| s.active_selection.clone())
    }

    pub async fn apply_delta(&self, session_id: &str, page: Option<&str>, node_id: &str, delta: &Value) {
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get_mut(session_id) {
            if session.index.as_ref().is_some_and(|idx| idx.page_id.as_deref() != page) { return; }
            session.invalidate_nodes(&[node_id.to_string()]);
            if let Some(ref mut idx) = session.index {
                idx.apply_delta(node_id, delta);
            }
        }
    }

    pub fn checked_session_id(inner: &BridgeInner, target: Option<&str>) -> Result<String, String> {
        let connected: Vec<_> = inner.sessions.values().filter(|s| s.is_connected()).collect();
        let target = target.map(str::trim).filter(|t| !t.is_empty());
        let matches: Vec<_> = if let Some(target) = target {
            if let Some(session) = inner.sessions.get(target) {
                return if session.is_connected() { Ok(target.into()) }
                    else { Err(format!("Figma session '{target}' is disconnected. Call figma_status.")) };
            }
            let exact: Vec<_> = connected.iter().copied().filter(|s| s.file_name.eq_ignore_ascii_case(target)).collect();
            if !exact.is_empty() { exact } else {
                connected.iter().copied().filter(|s| s.file_name.to_lowercase().contains(&target.to_lowercase()) || s.id.starts_with(target)).collect()
            }
        } else { connected };
        match matches.as_slice() {
            [session] => Ok(session.id.clone()),
            [] => Err("No matching connected Figma tab. Call figma_status and pass an exact sessionId.".into()),
            _ => Err("Ambiguous Figma tab. Call figma_status and pass an exact sessionId; automatic tab switching is disabled.".into()),
        }
    }

    // Cache-only callers get a sentinel on failure, never a different tab.
    pub fn resolve_session_id(inner: &BridgeInner, target: Option<&str>) -> String {
        Self::checked_session_id(inner, target).unwrap_or_default()
    }

    pub async fn resolved_session_id(&self, target: Option<&str>) -> String {
        Self::resolve_session_id(&*self.inner.lock().await, target)
    }

    pub async fn tool_target(&self, target: Option<&str>, task_id: Option<&str>, allow_disconnected: bool)
        -> Result<(String, Arc<Mutex<()>>, Option<TaskBinding>), String> {
        let inner = self.inner.lock().await;
        let task = match task_id {
            Some(id) => Some(inner.tasks.get(id).cloned().ok_or_else(|| format!("Unknown taskId '{id}'. Start a task with figma_task."))?),
            None => None,
        };
        let sid = match (allow_disconnected, &task) {
            (true, Some(task)) => task.session_id.clone(),
            _ => Self::checked_session_id(&inner, task.as_ref().map(|t| t.session_id.as_str()).or(target))?,
        };
        if task.is_some() && target.is_some() && target != Some(sid.as_str()) && Self::checked_session_id(&inner, target)? != sid {
            return Err("taskId is bound to a different Figma tab".into());
        }
        Ok((sid.clone(), inner.sessions.get(&sid).ok_or("Task session expired")?.tool_lock.clone(), task))
    }

}
