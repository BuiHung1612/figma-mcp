use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
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

pub struct Session {
    pub id: String,
    pub file_name: String,
    pub last_poll_at: u64,
    pub queue: Vec<QueuedOp>,
    pub pending: HashMap<String, PendingOp>,
    pub long_poll: Option<oneshot::Sender<PollResponse>>,
    pub ws_tx: Option<tokio::sync::mpsc::UnboundedSender<axum::extract::ws::Message>>,
    pub stats: SessionStats,
}

impl Session {
    pub fn new(id: String, file_name: Option<String>) -> Self {
        Self {
            id,
            file_name: file_name.unwrap_or_else(|| "unknown".to_string()),
            last_poll_at: 0,
            queue: Vec::new(),
            pending: HashMap::new(),
            long_poll: None,
            ws_tx: None,
            stats: SessionStats::default(),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.ws_tx.is_some() || (self.last_poll_at > 0 && (now_ms() - self.last_poll_at) < HEALTH_TTL_MS)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    #[serde(rename = "fileName")]
    pub file_name: String,
    pub connected: bool,
    #[serde(rename = "lastPollAgoMs")]
    pub last_poll_ago_ms: Option<u64>,
    #[serde(rename = "queueLength")]
    pub queue_length: usize,
    pub ops: usize,
}
