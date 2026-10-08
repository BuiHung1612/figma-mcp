use crate::bridge::BridgeHandle;
use crate::docs::get_docs;
use crate::executor::execute_code;
use super::protocol::{CallToolParams, JsonRpcRequest, JsonRpcResponse, ToolResult};
use super::{project_tools, read_tools};
use super::tools::get_tools;
use base64::prelude::*;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub(super) async fn save_export_to_disk(
    output_path: &str,
    data: &Value,
    default_ext: &str,
) -> Result<Value, String> {
    let path = std::path::Path::new(output_path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create directory '{}': {}", parent.display(), e))?;
        }
    }

    // Check if SVG string
    if let Some(svg_str) = data.get("svg").and_then(|v| v.as_str()) {
        tokio::fs::write(path, svg_str.as_bytes())
            .await
            .map_err(|e| format!("Failed to write SVG to '{}': {}", output_path, e))?;

        let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let width = data.get("width").cloned().unwrap_or(json!(null));
        let height = data.get("height").cloned().unwrap_or(json!(null));
        let node_id = data.get("nodeId").cloned().unwrap_or(json!(null));

        return Ok(json!({
            "success": true,
            "savedTo": abs_path.to_string_lossy(),
            "relativePath": output_path,
            "format": "svg",
            "width": width,
            "height": height,
            "sizeBytes": svg_str.len(),
            "nodeId": node_id
        }));
    }

    // Check if base64 (from export_image: data["base64"] or screenshot: data["dataUrl"])
    let (b64_str, fmt) = if let Some(b64) = data.get("base64").and_then(|v| v.as_str()) {
        let fmt = data.get("format").and_then(|v| v.as_str()).unwrap_or(default_ext);
        (b64, fmt)
    } else if let Some(data_url) = data.get("dataUrl").and_then(|v| v.as_str()) {
        let b64 = if let Some(idx) = data_url.find(',') {
            &data_url[idx + 1..]
        } else {
            data_url
        };
        (b64, "png")
    } else {
        return Err("No exportable image or SVG data found in response".to_string());
    };

    let bytes = BASE64_STANDARD
        .decode(b64_str.trim())
        .map_err(|e| format!("Base64 decode failed: {}", e))?;

    tokio::fs::write(path, &bytes)
        .await
        .map_err(|e| format!("Failed to write image to '{}': {}", output_path, e))?;

    let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let width = data.get("width").cloned().unwrap_or(json!(null));
    let height = data.get("height").cloned().unwrap_or(json!(null));
    let node_id = data.get("nodeId").cloned().unwrap_or(json!(null));
    let node_name = data.get("nodeName").cloned().unwrap_or(json!(null));

    Ok(json!({
        "success": true,
        "savedTo": abs_path.to_string_lossy(),
        "relativePath": output_path,
        "format": fmt,
        "width": width,
        "height": height,
        "sizeBytes": bytes.len(),
        "nodeId": node_id,
        "nodeName": node_name
    }))
}

pub(super) fn is_supported_read_operation(operation: &str) -> bool {
    matches!(
        operation,
        "get_selection"
            | "read_nodes"
            | "get_design"
            | "get_page_nodes"
            | "screenshot"
            | "export_svg"
            | "get_styles"
            | "get_local_components"
            | "get_viewport"
            | "get_variables"
            | "get_variable_tokens"
            | "get_tokens"
            | "get_node_detail"
            | "get_css"
            | "get_design_context"
            | "get_component_map"
            | "get_unmapped_components"
            | "export_node"
            | "exportNode"
            | "export_image"
            | "export_assets"
            | "search_nodes"
            | "scan_design"
    )
}

pub async fn handle_jsonrpc_request(
    bridge: BridgeHandle,
    req: JsonRpcRequest,
) -> Option<JsonRpcResponse> {
    match req.method.as_str() {
        "initialize" => Some(JsonRpcResponse::success(
            req.id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "figma-rust-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )),
        "notifications/initialized" => {
            // Client notification, no response required
            None
        }
        "tools/list" => Some(JsonRpcResponse::success(
            req.id,
            json!({
                "tools": get_tools()
            }),
        )),
        "tools/call" => {
            let tool_name = req
                .params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let start = std::time::Instant::now();
            let tool_res = handle_tool_call(bridge.clone(), req.params).await;
            let elapsed = start.elapsed();

            eprintln!(
                "[figma-rust-mcp] ⚡ Tool '{}' executed in {:.1}ms",
                tool_name,
                elapsed.as_secs_f64() * 1000.0
            );

            Some(JsonRpcResponse::success(
                req.id,
                serde_json::to_value(tool_res).unwrap_or(json!({})),
            ))
        }
        "ping" => Some(JsonRpcResponse::success(req.id, json!({}))),
        _ => Some(JsonRpcResponse::error(
            req.id,
            -32601,
            format!("Method '{}' not found", req.method),
        )),
    }
}

pub async fn run_mcp_server(bridge: BridgeHandle) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = JsonRpcResponse::error(None, -32700, format!("Parse error: {}", e));
                let resp_str = serde_json::to_string(&err_resp)? + "\n";
                stdout.write_all(resp_str.as_bytes()).await?;
                stdout.flush().await?;
                continue;
            }
        };

        if let Some(resp) = handle_jsonrpc_request(bridge.clone(), req).await {
            let resp_str = serde_json::to_string(&resp)? + "\n";
            stdout.write_all(resp_str.as_bytes()).await?;
            stdout.flush().await?;
        }
    }

    Ok(())
}

async fn handle_tool_call(bridge: BridgeHandle, params: Option<Value>) -> ToolResult {
    let name = params.as_ref().and_then(|p| p["name"].as_str()).unwrap_or("invalid").to_string();
    let start = std::time::Instant::now();
    let result = handle_tool_call_inner(bridge.clone(), params).await;
    if let BridgeHandle::Direct(state) = bridge {
        let mut inner = state.inner.lock().await;
        if name.starts_with("figma_") && (inner.tool_timings.len() < 64 || inner.tool_timings.contains_key(&name)) {
            let timing = inner.tool_timings.entry(name).or_default();
            timing.0 += 1;
            timing.1 += start.elapsed().as_millis() as u64;
        }
    }
    result
}

async fn handle_tool_call_inner(bridge: BridgeHandle, params: Option<Value>) -> ToolResult {
    if let BridgeHandle::Proxy(proxy) = &bridge {
        return match proxy.call_tool(params.unwrap_or(json!({}))).await {
            Ok(result) => result,
            Err(error) => ToolResult::error(error),
        };
    }
    let call_params: CallToolParams = match params.and_then(|p| serde_json::from_value(p).ok()) {
        Some(p) => p,
        None => return ToolResult::error("Invalid tool call parameters"),
    };

    let mut args = call_params.arguments.unwrap_or(json!({}));
    if !args.is_object() { return ToolResult::error("Tool arguments must be an object"); }
    for key in ["sessionId", "taskId"] {
        if let Some(value) = args.get(key) {
            if !value.as_str().is_some_and(|s| !s.trim().is_empty()) { return ToolResult::error(format!("{key} must be a non-empty string")); }
        }
    }
    if call_params.name == "figma_task" && args["action"] == "start" && args.get("taskId").is_some() {
        return ToolResult::error("taskId is generated by figma_task start; omit it when starting");
    }
    let mut bridge = bridge;
    let mut tool_guard = None;
    if !matches!(call_params.name.as_str(), "figma_status" | "figma_docs")
        && !(call_params.name == "figma_match_components" && args.get("nodeId").is_none() && args.get("sessionId").is_none() && args.get("taskId").is_none()) {
        if let BridgeHandle::Direct(state) = &bridge {
            let (sid, lock, task) = match state.tool_target(args["sessionId"].as_str(), args["taskId"].as_str(), call_params.name == "figma_task" && args["action"] == "end").await {
                Ok(target) => target,
                Err(error) => return ToolResult::error(error),
            };
            tool_guard = match tokio::time::timeout(std::time::Duration::from_secs(120), lock.lock_owned()).await {
                Ok(guard) => Some(guard),
                Err(_) => return ToolResult::error("Figma tab is busy; retry this task later"),
            };
            if let Some(task) = &task {
                if !state.inner.lock().await.tasks.contains_key(&task.task_id) { return ToolResult::error("Task was released while waiting"); }
                if args.get("nodeId").is_none() && args.get("nodeName").is_none() { args["nodeId"] = json!(task.frame_id); }
            }
            args["sessionId"] = json!(sid);
            if call_params.name != "figma_task" {
                let mut scoped_state = state.clone();
                scoped_state.task_scope = task;
                bridge = BridgeHandle::Direct(scoped_state);
            }
        }
    }
    // Hold the tab lock for the entire tool, including multi-op figma_write.
    let _tool_guard = tool_guard;

    match call_params.name.as_str() {
        "figma_status" => {
            let connected = bridge.is_plugin_connected(None).await;
            let mut plugin_info = None;

            if connected && bridge.get_sessions().await.iter().filter(|s| s.connected).count() == 1 {
                if let Ok(info) = bridge.send_operation("status", json!({}), None).await {
                    plugin_info = Some(info);
                }
            }

            let port = bridge.get_port().await;
            let queue_len = bridge.get_queue_length().await;
            let last_poll = bridge.get_last_poll_at().await;
            let stats = bridge.get_stats().await;
            let sessions = bridge.get_sessions().await;

            let hint = if connected {
                "CONNECTED. BEFORE drawing anything: call figma_docs to load mandatory design rules (token system, component-first, icon sizing, layer order). Skipping figma_docs causes incorrect, hardcoded, low-quality UI."
            } else {
                "Plugin not connected. In Figma Desktop: Plugins → Development → Figma Rust MCP Bridge → Run"
            };

            let out = json!({
                "bridgePort": port,
                "pluginConnected": connected,
                "pluginInfo": plugin_info,
                "queueLength": queue_len,
                "lastPollAgoMs": if last_poll > 0 { Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64 - last_poll) } else { None },
                "stats": stats,
                "sessions": sessions,
                "hint": hint,
            });

            ToolResult::text(serde_json::to_string(&out).unwrap_or_default())
        }

        "figma_task" => {
            let state = match &bridge { BridgeHandle::Direct(state) => state, _ => unreachable!() };
            let sid = args["sessionId"].as_str().unwrap();
            match args["action"].as_str().unwrap_or("list") {
                "start" => {
                    let frame_id = match args["frameId"].as_str().filter(|id| !id.is_empty()) {
                        Some(id) => id, None => return ToolResult::error("frameId is required"),
                    };
                    let task = crate::bridge::server::TaskBinding {
                        task_id: uuid::Uuid::new_v4().to_string(), session_id: sid.into(), frame_id: frame_id.into(),
                        document_id: state.inner.lock().await.sessions[sid].document_id.clone().unwrap_or_else(|| sid.into()),
                    };
                    {
                        let mut inner = state.inner.lock().await;
                        if inner.tasks.values().any(|owned| owned.document_id == task.document_id && owned.frame_id == task.frame_id) {
                            return ToolResult::error("Frame is already reserved by another task, possibly in another tab of the same document");
                        }
                        // Reserve before awaiting the plugin: another tab can start concurrently.
                        inner.tasks.insert(task.task_id.clone(), task.clone());
                    }
                    let data = match state.send_operation("task_start", json!({"taskId": task.task_id, "frameId": frame_id}), Some(sid)).await {
                        Ok(data) => data,
                        Err(error) => {
                            state.inner.lock().await.tasks.remove(&task.task_id);
                            return ToolResult::error(error);
                        }
                    };
                    ToolResult::text(json!({"taskId": task.task_id, "sessionId": sid, "frameId": frame_id, "frame": data,
                        "instructions": "Pass taskId on every subsequent tool call. Writes are confined to this frame; release with figma_task action=end."}).to_string())
                }
                "end" => {
                    let task_id = match args["taskId"].as_str() { Some(id) => id, None => return ToolResult::error("taskId is required") };
                    {
                        let mut inner = state.inner.lock().await;
                        let session = &inner.sessions[sid];
                        if !session.is_connected() {
                            if !session.pending.is_empty() { return ToolResult::error("Disconnected task still has pending operations; wait for them to settle before releasing"); }
                            inner.tasks.remove(task_id);
                            return ToolResult::text(json!({"taskId": task_id, "released": true, "pluginDisconnected": true}).to_string());
                        }
                    }
                    match state.send_operation("task_end", json!({"taskId": task_id}), Some(sid)).await {
                        Ok(data) => { state.inner.lock().await.tasks.remove(task_id); ToolResult::text(data.to_string()) },
                        Err(error) => ToolResult::error(error),
                    }
                }
                "list" => {
                    let inner = state.inner.lock().await;
                    let tasks: Vec<_> = inner.tasks.values().filter(|task| task.session_id == sid).collect();
                    ToolResult::text(json!({"sessionId": sid, "tasks": tasks}).to_string())
                }
                _ => ToolResult::error("figma_task action must be start, end or list"),
            }
        }

        "figma_docs" => {
            let section = args.get("section").and_then(|v| v.as_str());
            ToolResult::text(get_docs(section))
        }

        "figma_get_selection" => {
            if args["taskId"].is_string() {
                let mut params = args.clone();
                params["id"] = args["nodeId"].clone();
                params["detail"] = json!(args["detail"].as_str().unwrap_or("compact"));
                return match bridge.send_operation("get_design", params, args["sessionId"].as_str()).await {
                    Ok(data) => ToolResult::text(crate::mcp::semantic_optimizer::compress_tree(&data, args["detail"].as_str() != Some("full")).to_string()),
                    Err(error) => ToolResult::error(error),
                };
            }
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
            }

            let detail = args.get("detail").and_then(|v| v.as_str()).unwrap_or("compact");

            // Cache snapshots do not carry the requested detail/visibility contract.
            // Only use them for the default compact selection request.
            if let BridgeHandle::Direct(ref state) = bridge {
                if let Some(active_sel) = state.get_active_selection(session_id).await {
                    if active_sel.count > 0 && detail == "compact" && args.get("depth").is_none() && args.get("maxNodes").is_none() && args.get("absolute").is_none() && args.get("includeHidden").is_none() {
                        let out = json!({
                            "count": active_sel.count,
                            "pageName": active_sel.page_name,
                            "selection": active_sel.selection,
                            "fullNode": active_sel.full_node,
                            "cached": true,
                            "source": "realtime_selection_stream"
                        });
                        let out = crate::mcp::semantic_optimizer::compress_tree(&out, true);
                        return ToolResult::text(serde_json::to_string(&out).unwrap_or_default());
                    }
                }
            }

            let mut op_params = json!({});
            if let Some(depth) = args.get("depth") { op_params["depth"] = depth.clone(); }
            if let Some(mn) = args.get("maxNodes") { op_params["maxNodes"] = mn.clone(); }
            if let Some(abs) = args.get("absolute") { op_params["absolute"] = abs.clone(); }
            if let Some(ih) = args.get("includeHidden") { op_params["includeHidden"] = ih.clone(); }
            op_params["detail"] = json!(detail);

            match bridge.send_operation("get_selection", op_params, session_id).await {
                Ok(data) => {
                    let optimized = crate::mcp::semantic_optimizer::compress_tree(&data, detail != "full");
                    ToolResult::text(serde_json::to_string(&optimized).unwrap_or_default())
                }
                Err(e) => ToolResult::error(e),
            }
        }

        "figma_inspect_node" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
            }

            let mut node_id = args.get("nodeId")
                .or_else(|| args.get("id"))
                .or_else(|| args.get("node_id"))
                .or_else(|| args.get("targetId"))
                .or_else(|| args.get("target_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let node_name = args.get("nodeName")
                .or_else(|| args.get("name"))
                .or_else(|| args.get("node_name"))
                .and_then(|v| v.as_str());

            // If neither nodeId nor nodeName is passed, fallback to currently active selection ID
            if node_id.is_none() && node_name.is_none() {
                if let BridgeHandle::Direct(ref state) = bridge {
                    if let Some(active_sel) = state.get_active_selection(session_id).await {
                        if let Some(first_sel) = active_sel.selection.first() {
                            if let Some(id) = first_sel.get("id").and_then(|v| v.as_str()) {
                                node_id = Some(id.to_string());
                            }
                        }
                    }
                }
            }

            let mut op_params = json!({"depth":0,"fields":["geometry","content","text","style","layout","tokens","component"]});
            if let Some(ref nid) = node_id { op_params["id"] = json!(nid); }
            if let Some(nname) = node_name { op_params["name"] = json!(nname); }
            for key in ["fields","depth","limit","includeHidden","expandInstances"] {
                if let Some(value) = args.get(key) { op_params[key] = value.clone(); }
            }

            match bridge.send_operation("read_nodes", op_params, session_id).await {
                Ok(data) => ToolResult::text(serde_json::to_string(&data).unwrap_or_default()),
                Err(e) => ToolResult::error(e),
            }
        }

        "figma_export_asset" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
            }
            let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("png").to_lowercase();
            let mut op_params = json!({});
            let node_id = args.get("nodeId")
                .or_else(|| args.get("id"))
                .or_else(|| args.get("node_id"))
                .or_else(|| args.get("targetId"))
                .or_else(|| args.get("target_id"));
            let node_name = args.get("nodeName")
                .or_else(|| args.get("name"))
                .or_else(|| args.get("node_name"));

            if let Some(nid) = node_id { op_params["id"] = nid.clone(); }
            if let Some(nname) = node_name { op_params["name"] = nname.clone(); }
            if let Some(scale) = args.get("scale") { op_params["scale"] = scale.clone(); }
            op_params["format"] = json!(format);

            let operation = if format == "svg" { "export_svg" } else { "export_image" };
            match bridge.send_operation(operation, op_params, session_id).await {
                Ok(data) => {
                    if let Some(output_path) = args.get("outputPath").and_then(|v| v.as_str()) {
                        match save_export_to_disk(output_path, &data, &format).await {
                            Ok(disk_res) => ToolResult::text(serde_json::to_string(&disk_res).unwrap_or_default()),
                            Err(e) => ToolResult::error(e),
                        }
                    } else {
                        ToolResult::text(serde_json::to_string(&data).unwrap_or_default())
                    }
                }
                Err(e) => ToolResult::error(e),
            }
        }

        "figma_read" => read_tools::figma_read(bridge, args).await,

        "figma_write" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
            }

            let code = match args.get("code")
                .or_else(|| args.get("script"))
                .or_else(|| args.get("js"))
                .and_then(|v| v.as_str()) {
                Some(c) => c,
                None => return ToolResult::error("'code' is required"),
            };

            let exec_res = execute_code(code, bridge, session_id.map(|s| s.to_string())).await;
            let mut parts = Vec::new();
            if !exec_res.logs.is_empty() {
                parts.push(format!("Logs:\n{}", exec_res.logs.join("\n")));
            }

            if exec_res.success {
                let res_str = match exec_res.result {
                    Some(v) => serde_json::to_string(&v).unwrap_or_default(),
                    None => "null".to_string(),
                };
                parts.push(format!("Result: {}", res_str));
                ToolResult::text(parts.join("\n\n"))
            } else {
                parts.push(format!("Error: {}", exec_res.error.unwrap_or_else(|| "Unknown error".to_string())));
                ToolResult::error(parts.join("\n\n"))
            }
        }

        "figma_rules" => read_tools::figma_rules(bridge, args).await,

        "figma_index" => read_tools::figma_index(bridge, args).await,

        "figma_get_tokens" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
            }

            let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("css");
            let collection = args.get("collection").and_then(|v| v.as_str());
            let mode = args.get("mode").and_then(|v| v.as_str());
            let prefix = args.get("prefix").and_then(|v| v.as_str());
            let output_path = args.get("outputPath").and_then(|v| v.as_str());

            let cached = if let BridgeHandle::Direct(ref state) = bridge {
                let inner = state.inner.lock().await;
                let sid = crate::bridge::server::BridgeState::resolve_session_id(&inner, session_id);
                inner.sessions.get(&sid).and_then(|s| s.index.as_ref()).and_then(|idx| idx.token_snapshot())
            } else { None };
            let (styles_val, vars_val) = if let Some(snapshot) = cached { snapshot } else {
                let (styles, vars) = tokio::join!(
                    bridge.send_operation("get_styles", json!({}), session_id),
                    bridge.send_operation("get_variables", json!({}), session_id));
                match (styles, vars) {
                    (Ok(s), Ok(v)) => (s, v),
                    (Err(e), _) | (_, Err(e)) => return ToolResult::error(format!("Token extraction failed: {e}")),
                }
            };

            match crate::mcp::tokens::generate_tokens(&styles_val, &vars_val, format, collection, mode, prefix) {
                Ok(content) => {
                    if let Some(out_path) = output_path {
                        let path = std::path::Path::new(out_path);
                        if let Some(parent) = path.parent() {
                            if !parent.as_os_str().is_empty() {
                                let _ = tokio::fs::create_dir_all(parent).await;
                            }
                        }
                        match tokio::fs::write(path, content.as_bytes()).await {
                            Ok(_) => {
                                let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                                let out = json!({
                                    "success": true,
                                    "savedTo": abs_path.to_string_lossy(),
                                    "relativePath": out_path,
                                    "format": format,
                                    "sizeBytes": content.len(),
                                    "preview": content.lines().take(30).collect::<Vec<_>>().join("\n"),
                                });
                                ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                            }
                            Err(e) => ToolResult::error(format!("Failed to write tokens to '{}': {}", out_path, e)),
                        }
                    } else {
                        ToolResult::text(content)
                    }
                }
                Err(err) => ToolResult::error(err),
            }
        }

        "figma_to_code" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
            }

            let framework = args.get("framework").and_then(|v| v.as_str()).unwrap_or("react-tailwind");
            let component_name = args.get("componentName").and_then(|v| v.as_str());
            let output_path = args.get("outputPath").and_then(|v| v.as_str());

            let mut op_params = json!({});
            let node_id = args.get("nodeId")
                .or_else(|| args.get("id"))
                .or_else(|| args.get("node_id"))
                .or_else(|| args.get("targetId"))
                .or_else(|| args.get("target_id"));
            let node_name = args.get("nodeName")
                .or_else(|| args.get("name"))
                .or_else(|| args.get("node_name"));
            if let Some(nid) = node_id { op_params["id"] = nid.clone(); }
            if let Some(nname) = node_name { op_params["name"] = nname.clone(); }

            op_params["expandInstances"] = json!(true);
            match bridge.send_operation("get_design_context", op_params, session_id).await {
                Ok(context) => {
                    match crate::mcp::codegen::generate_code_from_context(&context, framework, component_name) {
                        Ok(code) => {
                            if let Some(out_path) = output_path {
                                let path = std::path::Path::new(out_path);
                                if let Some(parent) = path.parent() {
                                    if !parent.as_os_str().is_empty() {
                                        let _ = tokio::fs::create_dir_all(parent).await;
                                    }
                                }
                                match tokio::fs::write(path, code.as_bytes()).await {
                                    Ok(_) => {
                                        let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                                        let out = json!({
                                            "success": true,
                                            "savedTo": abs_path.to_string_lossy(),
                                            "relativePath": out_path,
                                            "framework": framework,
                                            "sizeBytes": code.len(),
                                            "preview": code.lines().take(40).collect::<Vec<_>>().join("\n"),
                                        });
                                        ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                                    }
                                    Err(e) => ToolResult::error(format!("Failed to write component to '{}': {}", out_path, e)),
                                }
                            } else {
                                ToolResult::text(code)
                            }
                        }
                        Err(err) => ToolResult::error(err),
                    }
                }
                Err(e) => ToolResult::error(e),
            }
        }

        "figma_export_assets" => project_tools::figma_export_assets(bridge, args).await,

        "figma_verify_ui" => project_tools::figma_verify_ui(bridge, args).await,

        "figma_match_components" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            let base_dir = args.get("projectDir").and_then(|v| v.as_str()).unwrap_or(".");
            let node_id = args.get("nodeId").and_then(|v| v.as_str());

            // 1. Scan codebase for UI components
            let scan_result = crate::mcp::component_matcher::scan_project_components(base_dir).await;

            // 2. If a specific nodeId is provided, check match for it
            let specific_match = if let Some(nid) = node_id {
                let mut matched_comp = None;
                if let BridgeHandle::Direct(ref state) = bridge {
                    let inner = state.inner.lock().await;
                    let sid = crate::bridge::server::BridgeState::resolve_session_id(&inner, session_id);
                    if let Some(session) = inner.sessions.get(&sid) {
                        if let Some(ref idx) = session.index {
                            if let Some(node) = idx.get_node(nid) {
                                matched_comp = crate::mcp::component_matcher::match_figma_to_codebase_component(&node.name, &scan_result).cloned();
                            }
                        }
                    }
                }
                matched_comp
            } else {
                None
            };

            let out = json!({
                "success": true,
                "projectDirectory": base_dir,
                "scannedDirectories": scan_result.scanned_directories,
                "totalComponentsDiscovered": scan_result.total_components,
                "components": scan_result.components,
                "targetedMatch": specific_match,
            });

            ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
        }

        "figma_prepare_design" => project_tools::figma_prepare_design(bridge, args).await,

        _ => ToolResult::error(format!("Unknown tool: {}", call_params.name)),
    }
}

#[cfg(test)]
mod tests {
    use super::is_supported_read_operation;

    #[tokio::test]
    async fn typography_tool_uses_cached_canonical_read_and_reports_timing() {
        let state = crate::bridge::server::BridgeState::new(0);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut session = crate::bridge::session::Session::new("s".into(),None);
        session.ws_tx = Some(tx);
        let tree = serde_json::json!({"id":"a","type":"FRAME","children":[{"id":"t","type":"TEXT","content":"Label","fontSize":15.5,"fontWeight":"Semi Bold","fontFamily":"Inter"}]});
        session.index = Some(crate::bridge::index::FigmaIndex::from_raw("s","f",&serde_json::json!([tree]),None,None,None,0));
        let params = serde_json::json!({"id":"a","fields":["geometry","text"],"depth":"full"});
        session.cache_read(format!("read_nodes:{params}"),serde_json::json!({"schemaVersion":4,"nodes":[{"id":"t","fontSize":15.5,"fontWeight":"Semi Bold"}],"complete":true,"nextCursor":null}));
        state.inner.lock().await.sessions.insert("s".into(),session);
        let result = super::handle_tool_call(crate::bridge::BridgeHandle::Direct(state.clone()),Some(serde_json::json!({"name":"figma_index","arguments":{"operation":"typography","nodeId":"a"}}))).await;
        let serialized = serde_json::to_value(result).unwrap();
        let payload: serde_json::Value = serde_json::from_str(serialized["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(payload["nodes"][0]["fontSize"],15.5);
        assert_eq!(payload["nodes"][0]["fontWeight"],"Semi Bold");
        assert!(rx.try_recv().is_err());
        let inner = state.inner.lock().await;
        assert_eq!(inner.sessions["s"].cache_hits,1);
        assert_eq!(inner.tool_timings["figma_index"].0,1);
    }

    #[test]
    fn read_operation_contract_rejects_unknown_plugin_operations() {
        assert!(is_supported_read_operation("get_design_context"));
        assert!(is_supported_read_operation("export_assets"));
        assert!(is_supported_read_operation("export_node"));
        assert!(!is_supported_read_operation("figma_prepare_design"));
        assert!(!is_supported_read_operation("does_not_exist"));
    }
}
