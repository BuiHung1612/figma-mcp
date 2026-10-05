use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;

pub const HEALTH_TTL_MS: u64 = 120_000;
pub const SESSION_EXPIRE_MS: u64 = 1_800_000;
pub const MAX_QUEUE: usize = 50;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedOp {
    pub id: String,
    pub operation: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollResponse {
    pub requests: Vec<QueuedOp>,
    pub mode: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub ops: usize,
    #[serde(rename = "avgLatencyMs")]
    pub avg_latency_ms: u64,
}

pub struct PendingOp {
    pub sender: oneshot::Sender<Result<Value, String>>,
    pub start_ms: u64,
    /// The dispatched op, kept so it can be re-queued if the transport dies
    /// before the plugin ever acknowledged it.
    pub op: QueuedOp,
    /// Set once the plugin confirms it received the op over the WebSocket. An
    /// acknowledged op is already running in the plugin, so it must never be
    /// re-queued — that would run a write twice.
    pub acked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveSelection {
    pub count: usize,
    #[serde(rename = "pageName")]
    pub page_name: Option<String>,
    pub selection: Vec<Value>,
    #[serde(rename = "fullNode")]
    pub full_node: Option<Value>,
    #[serde(rename = "updatedAt")]
    pub updated_at: u64,
}

pub struct Session {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub bridge_calls: u64,
    pub bridge_time_ms: u64,
    pub read_cache: HashMap<String, Value>,
    pub cache_revision: u64,
    pub tool_lock: Arc<tokio::sync::Mutex<()>>,
    pub id: String,
    pub file_name: String,
    pub document_id: Option<String>,
    pub last_poll_at: u64,
    pub queue: Vec<QueuedOp>,
    pub pending: HashMap<String, PendingOp>,
    pub long_poll: Option<oneshot::Sender<PollResponse>>,
    pub poll_generation: u64,
    pub ws_tx: Option<tokio::sync::mpsc::UnboundedSender<axum::extract::ws::Message>>,
    pub stats: SessionStats,
    pub index: Option<crate::bridge::index::FigmaIndex>,
    pub active_selection: Option<ActiveSelection>,
    pub runtime_version: Option<String>,
    pub protocol_version: Option<u64>,
    pub operations: Option<Vec<String>>,
}

impl Session {
    pub fn new(id: String, file_name: Option<String>) -> Self {
        Self {
            cache_hits: 0,
            cache_misses: 0,
            bridge_calls: 0,
            bridge_time_ms: 0,
            read_cache: HashMap::new(),
            cache_revision: 0,
            tool_lock: Arc::new(tokio::sync::Mutex::new(())),
            id,
            file_name: file_name.unwrap_or_else(|| "unknown".to_string()),
            document_id: None,
            last_poll_at: 0,
            queue: Vec::new(),
            pending: HashMap::new(),
            long_poll: None,
            poll_generation: 0,
            ws_tx: None,
            stats: SessionStats::default(),
            index: None,
            active_selection: None,
            runtime_version: None,
            protocol_version: None,
            operations: None,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.ws_tx.is_some() || (self.last_poll_at > 0 && (now_ms() - self.last_poll_at) < HEALTH_TTL_MS)
    }

    pub fn invalidate_reads(&mut self) {
        self.read_cache.clear();
        self.cache_revision = self.cache_revision.wrapping_add(1);
    }

    pub fn invalidate_nodes(&mut self, ids: &[String]) {
        self.cache_revision = self.cache_revision.wrapping_add(1);
        let index = &self.index;
        self.read_cache.retain(|key, _| {
            let Some((operation, params)) = key.split_once(':') else { return false };
            if matches!(operation, "get_styles" | "get_variables" | "get_variable_tokens") { return true; }
            let Ok(params) = serde_json::from_str::<Value>(params) else { return false };
            let Some(root) = params.get("id").or_else(|| params.get("nodeId")).and_then(Value::as_str) else { return false };
            let Some(idx) = index else { return false };
            !ids.iter().any(|id| idx.related(root, id))
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    #[serde(rename = "documentId", default)]
    pub document_id: Option<String>,
    #[serde(rename = "fileName")]
    pub file_name: String,
    pub connected: bool,
    #[serde(rename = "lastPollAgoMs")]
    pub last_poll_ago_ms: Option<u64>,
    #[serde(rename = "queueLength")]
    pub queue_length: usize,
    pub ops: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn changes_invalidate_related_subtrees_but_keep_other_frames_and_tokens() {
        let mut session = Session::new("s".into(), None);
        session.index = Some(crate::bridge::index::FigmaIndex::from_raw("s","f",&json!([
            {"id":"a","type":"FRAME","children":[{"id":"t","type":"TEXT"}]},
            {"id":"b","type":"FRAME"}]),None,None,None,0));
        for id in ["a","b","t"] { session.read_cache.insert(format!("get_design:{}", json!({"id":id})),json!({})); }
        session.read_cache.insert("get_styles:{}".into(),json!({}));
        session.invalidate_nodes(&["t".into()]);
        assert_eq!(session.read_cache.len(),2);
        assert!(session.read_cache.contains_key(&format!("get_design:{}",json!({"id":"b"}))));
        assert!(session.read_cache.contains_key("get_styles:{}"));
        session.invalidate_nodes(&["unknown".into()]);
        assert_eq!(session.read_cache.len(),1);
    }
}
