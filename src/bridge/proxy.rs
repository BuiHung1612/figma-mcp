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

fn resolve_tool_paths(mut params: Value, cwd: &std::path::Path) -> Value {
    let name = params["name"].as_str().unwrap_or("").to_string();
    if params.get("arguments").is_none() { params["arguments"] = json!({}); }
    if let Some(args) = params["arguments"].as_object_mut() {
        if matches!(name.as_str(), "figma_prepare_design" | "figma_export_assets") {
            args.entry("iconDir").or_insert(json!("src/assets/icons"));
        }
        if name == "figma_export_assets" { args.entry("imageDir").or_insert(json!("public/images")); }
        if matches!(name.as_str(), "figma_prepare_design" | "figma_match_components") {
            args.entry("projectDir").or_insert(json!("."));
        }
        if let Some(icon_dir) = args.get("iconDir").cloned() { args.insert("_importIconDir".into(), icon_dir); }
        for key in ["outputPath", "projectDir", "iconDir", "imageDir"] {
            if let Some(path) = args.get(key).and_then(Value::as_str) {
                if std::path::Path::new(path).is_relative() { args.insert(key.into(), json!(cwd.join(path).to_string_lossy())); }
            }
        }
    }
    params
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

    pub async fn call_tool(&self, params: Value) -> Result<crate::mcp::protocol::ToolResult, String> {
        let cwd = std::env::current_dir().map_err(|e| format!("Cannot resolve client project directory: {e}"))?;
        let params = resolve_tool_paths(params, &cwd);
        let response = self.client.post(format!("http://127.0.0.1:{}/mcp", self.port))
            .timeout(Duration::from_secs(300))
            .json(&json!({"jsonrpc": "2.0", "id": uuid::Uuid::new_v4().to_string(), "method": "tools/call", "params": params}))
            .send().await.map_err(|e| format!("MCP daemon connection failed: {e}"))?
            .error_for_status().map_err(|e| format!("MCP daemon rejected request: {e}"))?
            .json::<crate::mcp::protocol::JsonRpcResponse>().await.map_err(|e| format!("Invalid MCP response: {e}"))?;
        if let Some(error) = response.error { return Err(error.message); }
        serde_json::from_value(response.result.ok_or("Missing MCP tool result")?)
            .map_err(|e| format!("Invalid MCP tool result: {e}"))
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


#[cfg(test)]
mod tests {
    #[test]
    fn forwarded_tools_keep_client_filesystem_paths_and_relative_imports() {
        let request = super::resolve_tool_paths(serde_json::json!({"name": "figma_prepare_design", "arguments": {"nodeId": "1:2"}}), std::path::Path::new("/client/project"));
        assert_eq!(request["arguments"]["iconDir"], std::path::Path::new("/client/project").join("src/assets/icons").to_string_lossy().as_ref());
        assert_eq!(request["arguments"]["_importIconDir"], "src/assets/icons");
        assert_eq!(request["arguments"]["projectDir"], std::path::Path::new("/client/project").join(".").to_string_lossy().as_ref());
    }
}
