use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallToolParams {
    pub name: String,
    pub arguments: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    pub content: Vec<ToolContent>,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            is_error: None,
            content: vec![ToolContent {
                content_type: "text".to_string(),
                text: Some(text.into()),
                data: None,
                mime_type: None,
            }],
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            is_error: Some(true),
            content: vec![ToolContent {
                content_type: "text".to_string(),
                text: Some(msg.into()),
                data: None,
                mime_type: None,
            }],
        }
    }

    pub fn image(base64_data: impl Into<String>, mime: impl Into<String>, meta: Option<String>) -> Self {
        let mut content = vec![ToolContent {
            content_type: "image".to_string(),
            text: None,
            data: Some(base64_data.into()),
            mime_type: Some(mime.into()),
        }];
        if let Some(m) = meta {
            content.push(ToolContent {
                content_type: "text".to_string(),
                text: Some(m),
                data: None,
                mime_type: None,
            });
        }
        Self {
            is_error: None,
            content,
        }
    }
}
