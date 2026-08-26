use crate::bridge::BridgeHandle;
use crate::docs::get_docs;
use crate::executor::execute_code;
use super::protocol::{CallToolParams, JsonRpcRequest, JsonRpcResponse, ToolResult};
use super::tools::get_tools;
use base64::prelude::*;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn save_export_to_disk(
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
                    "name": "figma-mcp",
                    "version": "2.6.0"
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
                "[figma-mcp] ⚡ Tool '{}' executed in {:.1}ms",
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
    let call_params: CallToolParams = match params.and_then(|p| serde_json::from_value(p).ok()) {
        Some(p) => p,
        None => return ToolResult::error("Invalid tool call parameters"),
    };

    let args = call_params.arguments.unwrap_or(json!({}));

    match call_params.name.as_str() {
        "figma_status" => {
            let connected = bridge.is_plugin_connected(None).await;
            let mut plugin_info = None;

            if connected {
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
                "Plugin not connected. In Figma Desktop: Plugins → Development → Figma MCP Bridge → Run"
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

        "figma_docs" => {
            let section = args.get("section").and_then(|v| v.as_str());
            ToolResult::text(get_docs(section))
        }

        "figma_get_selection" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma MCP Bridge' plugin in Figma Desktop first.");
            }

            // Fast-path: Check if realtime active_selection cache is available
            if let BridgeHandle::Direct(ref state) = bridge {
                if let Some(active_sel) = state.get_active_selection(session_id).await {
                    if active_sel.count > 0 && args.get("depth").is_none() && args.get("maxNodes").is_none() {
                        let out = json!({
                            "count": active_sel.count,
                            "pageName": active_sel.page_name,
                            "selection": active_sel.selection,
                            "fullNode": active_sel.full_node,
                            "cached": true,
                            "source": "realtime_selection_stream"
                        });
                        return ToolResult::text(serde_json::to_string(&out).unwrap_or_default());
                    }
                }
            }

            let mut op_params = json!({});
            if let Some(depth) = args.get("depth") { op_params["depth"] = depth.clone(); }
            if let Some(mn) = args.get("maxNodes") { op_params["maxNodes"] = mn.clone(); }
            if let Some(abs) = args.get("absolute") { op_params["absolute"] = abs.clone(); }
            if let Some(ih) = args.get("includeHidden") { op_params["includeHidden"] = ih.clone(); }
            let detail = args.get("detail").and_then(|v| v.as_str()).unwrap_or("compact");
            op_params["detail"] = json!(detail);

            match bridge.send_operation("get_selection", op_params, session_id).await {
                Ok(data) => ToolResult::text(serde_json::to_string(&data).unwrap_or_default()),
                Err(e) => ToolResult::error(e),
            }
        }

        "figma_inspect_node" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma MCP Bridge' plugin in Figma Desktop first.");
            }

            let mut node_id = args.get("nodeId").and_then(|v| v.as_str()).map(|s| s.to_string());
            let node_name = args.get("nodeName").and_then(|v| v.as_str());

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

            // Fast-path: Check if node detail is available directly in Rust In-Memory Index
            if let BridgeHandle::Direct(ref state) = bridge {
                let sid = session_id.unwrap_or("_default");
                let inner = state.inner.lock().await;
                if let Some(session) = inner.sessions.get(sid) {
                    if let Some(ref idx) = session.index {
                        if idx.is_ready() {
                            let matched_node = if let Some(ref id) = node_id {
                                idx.get_node(id)
                            } else if let Some(name) = node_name {
                                idx.get_node_by_name(name)
                            } else {
                                None
                            };

                            if let Some(n) = matched_node {
                                let mut context = n.to_css_spec();
                                if let Some(obj) = context.as_object_mut() {
                                    obj.insert("cached".to_string(), json!(true));
                                    obj.insert("source".to_string(), json!("rust_memory_index"));
                                }
                                return ToolResult::text(serde_json::to_string(&context).unwrap_or_default());
                            }
                        }
                    }
                }
            }

            let mut op_params = json!({});
            if let Some(nid) = args.get("nodeId") { op_params["id"] = nid.clone(); }
            if let Some(nname) = args.get("nodeName") { op_params["name"] = nname.clone(); }

            match bridge.send_operation("get_design_context", op_params, session_id).await {
                Ok(data) => ToolResult::text(serde_json::to_string(&data).unwrap_or_default()),
                Err(e) => ToolResult::error(e),
            }
        }

        "figma_export_asset" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma MCP Bridge' plugin in Figma Desktop first.");
            }
            let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("png").to_lowercase();
            let mut op_params = json!({});
            if let Some(nid) = args.get("nodeId") { op_params["id"] = nid.clone(); }
            if let Some(nname) = args.get("nodeName") { op_params["name"] = nname.clone(); }
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

        "figma_read" => {
            let operation = match args.get("operation").and_then(|v| v.as_str()) {
                Some(op) => op,
                None => return ToolResult::error("'operation' is required"),
            };

            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma MCP Bridge' plugin in Figma Desktop first.");
            }

            // Forward every argument through to the handler (nodeId/nodeName are
            // renamed to the plugin's id/name). A whitelist here silently
            // swallowed handler options such as maxNodes / absolute /
            // keepViewport / inlineIcons; handlers ignore what they don't use.
            let mut op_params = json!({});
            if let Some(obj) = args.as_object() {
                for (k, v) in obj {
                    match k.as_str() {
                        "operation" | "outputPath" | "sessionId" => {}
                        "nodeId" => { op_params["id"] = v.clone(); }
                        "nodeName" => { op_params["name"] = v.clone(); }
                        _ => { op_params[k] = v.clone(); }
                    }
                }
            }

            // Fast-path read from index cache if available for read-only catalog queries
            if ["get_styles", "get_variables", "get_local_components"].contains(&operation) {
                if let BridgeHandle::Direct(ref state) = bridge {
                    let sid = session_id.unwrap_or("_default");
                    let inner = state.inner.lock().await;
                    if let Some(session) = inner.sessions.get(sid) {
                        if let Some(ref idx) = session.index {
                            if idx.is_ready() {
                                if operation == "get_styles" {
                                    let styles_json = json!({
                                        "cached": true,
                                        "styles": idx.styles,
                                        "paintStyles": idx.styles.iter().filter(|s| s.style_type == "PAINT").collect::<Vec<_>>(),
                                        "textStyles": idx.styles.iter().filter(|s| s.style_type == "TEXT").collect::<Vec<_>>(),
                                        "effectStyles": idx.styles.iter().filter(|s| s.style_type == "EFFECT").collect::<Vec<_>>(),
                                    });
                                    return ToolResult::text(serde_json::to_string(&styles_json).unwrap_or_default());
                                } else if operation == "get_variables" {
                                    let vars_json = json!({
                                        "cached": true,
                                        "variables": idx.variables,
                                    });
                                    return ToolResult::text(serde_json::to_string(&vars_json).unwrap_or_default());
                                } else if operation == "get_local_components" {
                                    let comps_json = json!({
                                        "cached": true,
                                        "components": idx.components,
                                    });
                                    return ToolResult::text(serde_json::to_string(&comps_json).unwrap_or_default());
                                } else if operation == "search_nodes" {
                                    let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                                    let t = args.get("type").and_then(|v| v.as_str());
                                    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
                                    let matches = idx.search_nodes(q, t, limit);
                                    let res = json!({
                                        "cached": true,
                                        "query": q,
                                        "count": matches.len(),
                                        "nodes": matches
                                    });
                                    return ToolResult::text(serde_json::to_string(&res).unwrap_or_default());
                                } else if operation == "get_node_detail" {
                                    let node_id = args.get("nodeId").or_else(|| args.get("id")).and_then(|v| v.as_str());
                                    let node_name = args.get("nodeName").or_else(|| args.get("name")).and_then(|v| v.as_str());
                                    let matched = if let Some(id) = node_id {
                                        idx.get_node(id)
                                    } else if let Some(name) = node_name {
                                        idx.get_node_by_name(name)
                                    } else {
                                        None
                                    };
                                    if let Some(n) = matched {
                                        let mut detail = n.to_css_spec();
                                        if let Some(obj) = detail.as_object_mut() {
                                            obj.insert("cached".to_string(), json!(true));
                                        }
                                        return ToolResult::text(serde_json::to_string(&detail).unwrap_or_default());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if operation == "get_tokens" {
                let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("css");
                let collection = args.get("collection").and_then(|v| v.as_str());
                let mode = args.get("mode").and_then(|v| v.as_str());
                let prefix = args.get("prefix").and_then(|v| v.as_str());
                let output_path = args.get("outputPath").and_then(|v| v.as_str());

                let styles_fut = bridge.send_operation("get_styles", json!({}), session_id);
                let vars_fut = bridge.send_operation("get_variables", json!({}), session_id);
                let (styles_res, vars_res) = tokio::join!(styles_fut, vars_fut);

                let styles_data = styles_res.unwrap_or(json!({}));
                let vars_data = vars_res.unwrap_or(json!({}));

                match crate::mcp::tokens::generate_tokens(&styles_data, &vars_data, format, collection, mode, prefix) {
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
                                        "preview": content.lines().take(25).collect::<Vec<_>>().join("\n"),
                                    });
                                    return ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default());
                                }
                                Err(e) => return ToolResult::error(format!("Failed to write tokens to '{}': {}", out_path, e)),
                            }
                        } else {
                            return ToolResult::text(content);
                        }
                    }
                    Err(err) => return ToolResult::error(err),
                }
            }

            let output_path = args.get("outputPath").and_then(|v| v.as_str());
            match bridge.send_operation(operation, op_params, session_id).await {
                Ok(data) => {
                    if let Some(out_path) = output_path {
                        if ["export_image", "export_svg", "screenshot"].contains(&operation) {
                            match save_export_to_disk(out_path, &data, "png").await {
                                Ok(disk_res) => return ToolResult::text(serde_json::to_string(&disk_res).unwrap_or_default()),
                                Err(e) => return ToolResult::error(e),
                            }
                        }
                    }

                    if operation == "screenshot" {
                        if let Some(data_url) = data.get("dataUrl").and_then(|v| v.as_str()) {
                            let b64 = if let Some(idx) = data_url.find(',') {
                                &data_url[idx + 1..]
                            } else {
                                data_url
                            };
                            let mut meta = data.clone();
                            if let Some(obj) = meta.as_object_mut() {
                                obj.remove("dataUrl");
                            }
                            let meta_str = if meta.as_object().is_some_and(|o| !o.is_empty()) {
                                Some(serde_json::to_string(&meta).unwrap_or_default())
                            } else {
                                None
                            };
                            return ToolResult::image(b64, "image/png", meta_str);
                        }
                    }

                    ToolResult::text(serde_json::to_string(&data).unwrap_or_default())
                }
                Err(e) => ToolResult::error(e),
            }
        }

        "figma_write" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma MCP Bridge' plugin in Figma Desktop first.");
            }

            let code = match args.get("code").and_then(|v| v.as_str()) {
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

        "figma_rules" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma MCP Bridge' plugin in Figma Desktop first.");
            }

            // Fast-path: If index is cached and ready, build rules directly from memory (< 1ms!)
            if let BridgeHandle::Direct(ref state) = bridge {
                let sid = session_id.unwrap_or("_default");
                let inner = state.inner.lock().await;
                if let Some(session) = inner.sessions.get(sid) {
                    if let Some(ref idx) = session.index {
                        if idx.is_ready() {
                            let mut lines = vec![
                                "# Design System Rules (from fast-index)".to_string(),
                                "".to_string(),
                                "Use these tokens, styles, and components when writing code for this Figma file.".to_string(),
                                "".to_string(),
                            ];

                            // Paint styles
                            let paint_styles: Vec<_> = idx.styles.iter().filter(|s| s.style_type == "PAINT" && s.hex.is_some()).collect();
                            if !paint_styles.is_empty() {
                                lines.push("## Color Tokens (Paint Styles)".to_string());
                                lines.push("```".to_string());
                                for s in paint_styles {
                                    lines.push(format!("--{}: {};  /* {} */", s.name.replace('/', "-"), s.hex.as_deref().unwrap_or(""), s.name));
                                }
                                lines.push("```".to_string());
                                lines.push("".to_string());
                            }

                            // Variables / tokens
                            if !idx.variables.is_empty() {
                                lines.push("## Variables & Tokens".to_string());
                                lines.push("```".to_string());
                                for v in &idx.variables {
                                    lines.push(format!("{}.{} ({})", v.collection_name, v.name, v.resolved_type));
                                }
                                lines.push("```".to_string());
                                lines.push("".to_string());
                            }

                            // Text styles
                            let text_styles: Vec<_> = idx.styles.iter().filter(|s| s.style_type == "TEXT").collect();
                            if !text_styles.is_empty() {
                                lines.push("## Typography Styles".to_string());
                                lines.push("```".to_string());
                                for s in text_styles {
                                    let fam = s.font_family.as_deref().unwrap_or("Inter");
                                    let weight = s.font_weight.as_deref().unwrap_or("Regular");
                                    let size = s.font_size.unwrap_or(14.0);
                                    lines.push(format!("{}: {} {} {}px", s.name, fam, weight, size));
                                }
                                lines.push("```".to_string());
                                lines.push("".to_string());
                            }

                            // Components
                            if !idx.components.is_empty() {
                                lines.push("## Components".to_string());
                                for c in idx.components.iter().take(50) {
                                    let desc = c.description.as_deref().map_or("".to_string(), |d| format!(" — {}", d));
                                    let w = c.width.unwrap_or(0.0);
                                    let h = c.height.unwrap_or(0.0);
                                    lines.push(format!("- **{}** ({}×{}){}", c.name, w, h, desc));
                                }
                                if idx.components.len() > 50 {
                                    lines.push(format!("  …and {} more", idx.components.len() - 50));
                                }
                                lines.push("".to_string());
                            }

                            lines.push("---".to_string());
                            lines.push("_Generated by figma-mcp figma_rules (in-memory cached)._".to_string());
                            return ToolResult::text(lines.join("\n"));
                        }
                    }
                }
            }

            let styles_fut = bridge.send_operation("get_styles", json!({}), session_id);
            let vars_fut = bridge.send_operation("get_variables", json!({}), session_id);
            let comps_fut = bridge.send_operation("get_local_components", json!({}), session_id);

            let (styles_res, vars_res, comps_res) = tokio::join!(styles_fut, vars_fut, comps_fut);

            let styles_data = styles_res.unwrap_or(json!({}));
            let vars_data = vars_res.unwrap_or(json!({}));
            let comps_data = comps_res.unwrap_or(json!({}));

            let mut lines = vec![
                "# Design System Rules".to_string(),
                "".to_string(),
                "Use these tokens, styles, and components when writing code for this Figma file.".to_string(),
                "".to_string(),
            ];

            // Colors
            if let Some(paint_styles) = styles_data.get("paintStyles").and_then(|v| v.as_array()) {
                if !paint_styles.is_empty() {
                    lines.push("## Color Tokens (Paint Styles)".to_string());
                    lines.push("```".to_string());
                    for s in paint_styles {
                        if let (Some(name), Some(hex)) = (s.get("name").and_then(|v| v.as_str()), s.get("hex").and_then(|v| v.as_str())) {
                            lines.push(format!("--{}: {};  /* {} */", name.replace('/', "-"), hex, name));
                        }
                    }
                    lines.push("```".to_string());
                    lines.push("".to_string());
                }
            }

            // Variables
            if let Some(collections) = vars_data.get("collections").and_then(|v| v.as_array()) {
                for col in collections {
                    if let Some(vars) = col.get("variables").and_then(|v| v.as_array()) {
                        if !vars.is_empty() {
                            let col_name = col.get("name").and_then(|v| v.as_str()).unwrap_or("Tokens");
                            lines.push(format!("## Variables — {}", col_name));
                            if let Some(modes) = col.get("modes").and_then(|v| v.as_array()) {
                                if modes.len() > 1 {
                                    let mode_names: Vec<&str> = modes.iter().filter_map(|m| m.get("name").and_then(|v| v.as_str())).collect();
                                    lines.push(format!("Modes: {}", mode_names.join(" | ")));
                                }
                            }
                            lines.push("```".to_string());
                            for v in vars {
                                let v_name = v.get("name").and_then(|val| val.as_str()).unwrap_or_default();
                                let v_type = v.get("resolvedType").and_then(|val| val.as_str()).unwrap_or_default();
                                lines.push(format!("{} ({})", v_name, v_type));
                            }
                            lines.push("```".to_string());
                            lines.push("".to_string());
                        }
                    }
                }
            }

            // Typography
            if let Some(text_styles) = styles_data.get("textStyles").and_then(|v| v.as_array()) {
                if !text_styles.is_empty() {
                    lines.push("## Typography Styles".to_string());
                    lines.push("```".to_string());
                    for s in text_styles {
                        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                        let font_family = s.get("fontFamily").and_then(|v| v.as_str()).unwrap_or("Inter");
                        let font_weight = s.get("fontWeight").and_then(|v| v.as_str()).unwrap_or("Regular");
                        let font_size = s.get("fontSize").and_then(|v| v.as_f64()).unwrap_or(14.0);
                        let line_height_str = if let Some(lh) = s.get("lineHeight").and_then(|v| v.as_f64()) {
                            format!(" / {}px", lh)
                        } else {
                            "".to_string()
                        };
                        lines.push(format!("{}: {} {} {}px{}", name, font_family, font_weight, font_size, line_height_str));
                    }
                    lines.push("```".to_string());
                    lines.push("".to_string());
                }
            }

            // Component sets
            if let Some(comp_sets) = comps_data.get("componentSets").and_then(|v| v.as_array()) {
                if !comp_sets.is_empty() {
                    lines.push("## Component Sets (use with get_component_map)".to_string());
                    for s in comp_sets {
                        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                        let variant_count = s.get("variantCount").and_then(|v| v.as_i64()).unwrap_or(0);
                        let desc = s.get("description").and_then(|v| v.as_str()).map_or("".to_string(), |d| format!(" — {}", d));
                        lines.push(format!("- **{}** ({} variants){}", name, variant_count, desc));
                    }
                    lines.push("".to_string());
                }
            }

            // Standalone components
            if let Some(comps) = comps_data.get("components").and_then(|v| v.as_array()) {
                if !comps.is_empty() {
                    lines.push("## Standalone Components".to_string());
                    for c in comps.iter().take(40) {
                        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                        let w = c.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let h = c.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let desc = c.get("description").and_then(|v| v.as_str()).map_or("".to_string(), |d| format!(" — {}", d));
                        lines.push(format!("- **{}** ({}×{}){}", name, w, h, desc));
                    }
                    if comps.len() > 40 {
                        lines.push(format!("  …and {} more", comps.len() - 40));
                    }
                    lines.push("".to_string());
                }
            }

            lines.push("---".to_string());
            lines.push("_Generated by figma-mcp figma_rules. Re-run when design system changes._".to_string());

            ToolResult::text(lines.join("\n"))
        }

        "figma_index" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            let operation = match args.get("operation").and_then(|v| v.as_str()) {
                Some(op) => op,
                None => return ToolResult::error("'operation' is required (status, search_nodes, get_node, search_components, search_styles, search_variables, refresh)"),
            };

            match operation {
                "status" => {
                    let stats = bridge.get_index_stats(session_id).await;
                    let connected = bridge.is_plugin_connected(session_id).await;
                    match stats {
                        Some(st) => {
                            let out = json!({
                                "status": "ready",
                                "pluginConnected": connected,
                                "stats": st,
                            });
                            ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                        }
                        None => {
                            let out = json!({
                                "status": "not_indexed",
                                "pluginConnected": connected,
                                "hint": "File not yet indexed. Call with operation='refresh' to build index in background."
                            });
                            ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                        }
                    }
                }

                "get_node" => {
                    let node_id = match args.get("nodeId").and_then(|v| v.as_str()) {
                        Some(id) => id,
                        None => return ToolResult::error("'nodeId' is required for get_node"),
                    };

                    match bridge.get_index_node(session_id, node_id).await {
                        Some(node) => ToolResult::text(serde_json::to_string_pretty(&node).unwrap_or_default()),
                        None => {
                            // Fallback to direct bridge read if not indexed or node not in cache
                            match bridge.send_operation("get_node_detail", json!({ "id": node_id }), session_id).await {
                                Ok(data) => ToolResult::text(serde_json::to_string_pretty(&data).unwrap_or_default()),
                                Err(e) => ToolResult::error(format!("Node not found in index or canvas: {}", e)),
                            }
                        }
                    }
                }

                "search_nodes" => {
                    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    let node_type = args.get("nodeType").and_then(|v| v.as_str());
                    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize;

                    match bridge.search_index_nodes(session_id, query, node_type, limit).await {
                        Some(results) => {
                            let out = json!({
                                "query": query,
                                "nodeType": node_type,
                                "count": results.len(),
                                "cached": true,
                                "results": results,
                            });
                            ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                        }
                        None => {
                            // Fallback to bridge search_nodes
                            let mut op_params = json!({ "query": query, "limit": limit });
                            if let Some(t) = node_type { op_params["type"] = json!(t); }
                            match bridge.send_operation("search_nodes", op_params, session_id).await {
                                Ok(data) => ToolResult::text(serde_json::to_string_pretty(&data).unwrap_or_default()),
                                Err(e) => ToolResult::error(e),
                            }
                        }
                    }
                }

                "search_components" => {
                    let name = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize;

                    match bridge.search_index_components(session_id, name, limit).await {
                        Some(results) => {
                            let out = json!({
                                "query": name,
                                "count": results.len(),
                                "cached": true,
                                "components": results,
                            });
                            ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                        }
                        None => {
                            match bridge.send_operation("get_local_components", json!({}), session_id).await {
                                Ok(data) => ToolResult::text(serde_json::to_string_pretty(&data).unwrap_or_default()),
                                Err(e) => ToolResult::error(e),
                            }
                        }
                    }
                }

                "search_styles" => {
                    let name = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    let style_type = args.get("styleType").and_then(|v| v.as_str());

                    match bridge.search_index_styles(session_id, name, style_type).await {
                        Some(results) => {
                            let out = json!({
                                "query": name,
                                "styleType": style_type,
                                "count": results.len(),
                                "cached": true,
                                "styles": results,
                            });
                            ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                        }
                        None => {
                            match bridge.send_operation("get_styles", json!({}), session_id).await {
                                Ok(data) => ToolResult::text(serde_json::to_string_pretty(&data).unwrap_or_default()),
                                Err(e) => ToolResult::error(e),
                            }
                        }
                    }
                }

                "search_variables" => {
                    let name = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    let collection = args.get("collection").and_then(|v| v.as_str());

                    match bridge.search_index_variables(session_id, name, collection).await {
                        Some(results) => {
                            let out = json!({
                                "query": name,
                                "collection": collection,
                                "count": results.len(),
                                "cached": true,
                                "variables": results,
                            });
                            ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                        }
                        None => {
                            match bridge.send_operation("get_variables", json!({}), session_id).await {
                                Ok(data) => ToolResult::text(serde_json::to_string_pretty(&data).unwrap_or_default()),
                                Err(e) => ToolResult::error(e),
                            }
                        }
                    }
                }

                "refresh" => {
                    if !bridge.is_plugin_connected(session_id).await {
                        return ToolResult::error("Figma plugin not connected. Run the plugin in Figma first.");
                    }

                    // Trigger index_scan operation on the plugin
                    let start_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                    match bridge.send_operation("index_scan", json!({}), session_id).await {
                        Ok(data) => {
                            let page_nodes = data.get("pageNodes").unwrap_or(&Value::Null);
                            let styles = data.get("styles");
                            let vars = data.get("variables");
                            let comps = data.get("components");
                            let file_name = data.get("fileName").and_then(|v| v.as_str()).unwrap_or("unknown");
                            let sid = session_id.unwrap_or("_default");

                            let idx = crate::bridge::index::FigmaIndex::from_raw(
                                sid,
                                file_name,
                                page_nodes,
                                styles,
                                vars,
                                comps,
                                start_ms,
                            );

                            let stats = idx.stats.clone();
                            if let BridgeHandle::Direct(ref state) = bridge {
                                state.update_index(sid, idx).await;
                            }

                            let out = json!({
                                "success": true,
                                "stats": stats,
                                "message": format!("Indexed {} nodes, {} components, {} styles, {} variables in {}ms", stats.total_nodes, stats.total_components, stats.total_styles, stats.total_variables, stats.duration_ms)
                            });
                            ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                        }
                        Err(e) => ToolResult::error(format!("Index refresh failed: {}", e)),
                    }
                }

                _ => ToolResult::error(format!("Unknown figma_index operation: '{}'. Available: status, search_nodes, get_node, search_components, search_styles, search_variables, refresh", operation)),
            }
        }

        "figma_get_tokens" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma MCP Bridge' plugin in Figma Desktop first.");
            }

            let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("css");
            let collection = args.get("collection").and_then(|v| v.as_str());
            let mode = args.get("mode").and_then(|v| v.as_str());
            let prefix = args.get("prefix").and_then(|v| v.as_str());
            let output_path = args.get("outputPath").and_then(|v| v.as_str());

            // Fast-path: If index is cached in memory, use it directly!
            let (styles_data, vars_data) = if let BridgeHandle::Direct(ref state) = bridge {
                let sid = session_id.unwrap_or("_default");
                let inner = state.inner.lock().await;
                if let Some(session) = inner.sessions.get(sid) {
                    if let Some(ref idx) = session.index {
                        if idx.is_ready() {
                            let styles_json = json!({
                                "paintStyles": idx.styles.iter().filter(|s| s.style_type == "PAINT").collect::<Vec<_>>(),
                                "textStyles": idx.styles.iter().filter(|s| s.style_type == "TEXT").collect::<Vec<_>>(),
                                "effectStyles": idx.styles.iter().filter(|s| s.style_type == "EFFECT").collect::<Vec<_>>(),
                            });
                            let vars_json = json!({
                                "variables": idx.variables,
                            });
                            (Some(styles_json), Some(vars_json))
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            let (styles_val, vars_val) = match (styles_data, vars_data) {
                (Some(s), Some(v)) => (s, v),
                _ => {
                    let styles_fut = bridge.send_operation("get_styles", json!({}), session_id);
                    let vars_fut = bridge.send_operation("get_variables", json!({}), session_id);
                    let (styles_res, vars_res) = tokio::join!(styles_fut, vars_fut);
                    (styles_res.unwrap_or(json!({})), vars_res.unwrap_or(json!({})))
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
                return ToolResult::error("Figma plugin not connected. Run the 'Figma MCP Bridge' plugin in Figma Desktop first.");
            }

            let framework = args.get("framework").and_then(|v| v.as_str()).unwrap_or("react-tailwind");
            let component_name = args.get("componentName").and_then(|v| v.as_str());
            let output_path = args.get("outputPath").and_then(|v| v.as_str());

            let mut op_params = json!({});
            if let Some(nid) = args.get("nodeId") { op_params["id"] = nid.clone(); }
            if let Some(nname) = args.get("nodeName") { op_params["name"] = nname.clone(); }

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

        "figma_export_assets" => {
            let session_id = args.get("sessionId").and_then(|v| v.as_str());
            if !bridge.is_plugin_connected(session_id).await {
                return ToolResult::error("Figma plugin not connected. Run the 'Figma MCP Bridge' plugin in Figma Desktop first.");
            }

            let icon_dir = args.get("iconDir").and_then(|v| v.as_str());
            let image_dir = args.get("imageDir").and_then(|v| v.as_str());
            let create_barrel = args.get("createBarrel").and_then(|v| v.as_bool()).unwrap_or(true);

            let mut op_params = json!({});
            if let Some(nid) = args.get("nodeId") { op_params["id"] = nid.clone(); }

            match bridge.send_operation("export_assets", op_params, session_id).await {
                Ok(data) => {
                    let mut exported_icons = Vec::new();
                    let mut exported_images = Vec::new();
                    let mut barrel_lines = Vec::new();

                    // Save SVG icons
                    if let Some(icons) = data.get("icons").and_then(|v| v.as_array()) {
                        let target_icon_dir = icon_dir.unwrap_or("src/assets/icons");
                        let dir_path = std::path::Path::new(target_icon_dir);
                        let _ = tokio::fs::create_dir_all(dir_path).await;

                        for item in icons {
                            let file_name = item.get("fileName").and_then(|v| v.as_str()).unwrap_or("icon.svg");
                            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("icon");
                            let svg = item.get("svg").and_then(|v| v.as_str()).unwrap_or("");

                            let file_path = dir_path.join(file_name);
                            if tokio::fs::write(&file_path, svg.as_bytes()).await.is_ok() {
                                let abs = std::fs::canonicalize(&file_path).unwrap_or(file_path.clone());
                                exported_icons.push(json!({
                                    "name": name,
                                    "path": abs.to_string_lossy(),
                                    "file": file_name,
                                    "sizeBytes": svg.len(),
                                }));

                                let comp_name = name.split('-').map(|w| {
                                    let mut c = w.chars();
                                    match c.next() {
                                        None => String::new(),
                                        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                                    }
                                }).collect::<String>() + "Icon";
                                barrel_lines.push(format!("export {{ default as {} }} from './{}';", comp_name, file_name));
                            }
                        }

                        if create_barrel && !barrel_lines.is_empty() {
                            let barrel_path = dir_path.join("index.ts");
                            let _ = tokio::fs::write(barrel_path, barrel_lines.join("\n").as_bytes()).await;
                        }
                    }

                    // Save raster images
                    if let Some(images) = data.get("images").and_then(|v| v.as_array()) {
                        let target_img_dir = image_dir.unwrap_or("public/images");
                        let dir_path = std::path::Path::new(target_img_dir);
                        let _ = tokio::fs::create_dir_all(dir_path).await;

                        for item in images {
                            let file_name = item.get("fileName").and_then(|v| v.as_str()).unwrap_or("image.png");
                            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("image");

                            if let Some(data_url) = item.get("dataUrl").and_then(|v| v.as_str()) {
                                let b64 = if let Some(idx) = data_url.find(',') {
                                    &data_url[idx + 1..]
                                } else {
                                    data_url
                                };

                                if let Ok(bytes) = BASE64_STANDARD.decode(b64.trim()) {
                                    let file_path = dir_path.join(file_name);
                                    if tokio::fs::write(&file_path, &bytes).await.is_ok() {
                                        let abs = std::fs::canonicalize(&file_path).unwrap_or(file_path.clone());
                                        exported_images.push(json!({
                                            "name": name,
                                            "path": abs.to_string_lossy(),
                                            "file": file_name,
                                            "sizeBytes": bytes.len(),
                                        }));
                                    }
                                }
                            }
                        }
                    }

                    let out = json!({
                        "success": true,
                        "sourceNode": data.get("sourceNodeName").unwrap_or(&json!("canvas")),
                        "totalIconsExported": exported_icons.len(),
                        "totalImagesExported": exported_images.len(),
                        "iconDirectory": icon_dir.unwrap_or("src/assets/icons"),
                        "imageDirectory": image_dir.unwrap_or("public/images"),
                        "barrelGenerated": create_barrel,
                        "icons": exported_icons,
                        "images": exported_images,
                    });
                    ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                }
                Err(e) => ToolResult::error(e),
            }
        }

        _ => ToolResult::error(format!("Unknown tool: {}", call_params.name)),
    }
}
