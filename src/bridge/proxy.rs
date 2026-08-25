use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    #[serde(rename = "pluginConnected", default)]
    pub plugin_connected: bool,
    #[serde(rename = "queueLength", default)]
    pub queue_length: usize,
    #[serde(rename = "pendingCount", default)]
    pub pending_count: usize,
    #[serde(rename = "lastPollAgoMs")]
    pub last_poll_ago_ms: Option<u64>,
    pub stats: Option<Value>,
    pub sessions: Option<Vec<Value>>,
}

#[derive(Clone)]
pub struct HttpProxy {
    pub port: u16,
    pub client: Client,
}

impl HttpProxy {
    pub fn new(port: u16) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .unwrap_or_default();
        Self { port, client }
    }

    pub async fn is_running(&self) -> bool {
        let url = format!("http://127.0.0.1:{}/health", self.port);
        self.client
            .get(&url)
            .timeout(Duration::from_millis(500))
            .send()
            .await
            .is_ok()
    }

    pub async fn check_health(&self) -> HealthResponse {
        let url = format!("http://127.0.0.1:{}/health", self.port);
        match self.client.get(&url).timeout(Duration::from_millis(2000)).send().await {
            Ok(res) => res.json::<HealthResponse>().await.unwrap_or(HealthResponse {
                plugin_connected: false,
                queue_length: 0,
                pending_count: 0,
                last_poll_ago_ms: None,
                stats: None,
                sessions: None,
            }),
            Err(_) => HealthResponse {
                plugin_connected: false,
                queue_length: 0,
                pending_count: 0,
                last_poll_ago_ms: None,
                stats: None,
                sessions: None,
            },
        }
    }

    pub async fn send_operation(&self, operation: &str, params: Value, session_id: Option<&str>) -> Result<Value, String> {
        let mut url = format!("http://127.0.0.1:{}/exec", self.port);
        if let Some(sid) = session_id {
            url.push_str(&format!("?sessionId={}", sid));
        }

        let payload = json!({
            "operation": operation,
            "params": params,
        });

        let mut req = self.client.post(&url).json(&payload);
        if let Some(sid) = session_id {
            req = req.header("X-Session-Id", sid);
        }

        let res = req.send().await.map_err(|e| format!("Bridge connection failed: {}", e))?;
        let status = res.status();
        let body: Value = res.json().await.map_err(|e| format!("Invalid bridge response: {}", e))?;

        if !status.is_success() {
            let err_msg = body.get("error").and_then(|v| v.as_str()).unwrap_or("Bridge error");
            return Err(err_msg.to_string());
        }

        if body.get("success").and_then(|v| v.as_bool()) == Some(true) {
            Ok(body.get("data").cloned().unwrap_or(Value::Null))
        } else {
            let err_msg = body.get("error").and_then(|v| v.as_str()).unwrap_or("Bridge error");
            Err(err_msg.to_string())
        }
    }
}
