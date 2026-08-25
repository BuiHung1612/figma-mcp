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

        _ => ToolResult::error(format!("Unknown tool: {}", call_params.name)),
    }
}
