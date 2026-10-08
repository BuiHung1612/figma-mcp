//! `figma_read`, `figma_rules` and `figma_index`, split out of `server::handle_tool_call_inner`.

use super::protocol::ToolResult;
use super::server::{is_supported_read_operation, save_export_to_disk};
use crate::bridge::BridgeHandle;
use serde_json::{json, Value};

pub(super) async fn figma_read(bridge: BridgeHandle, args: Value) -> ToolResult {
    let raw_operation = match args.get("operation")
        .or_else(|| args.get("op"))
        .or_else(|| args.get("action"))
        .or_else(|| args.get("command"))
        .and_then(|v| v.as_str()) {
        Some(op) => op,
        None => return ToolResult::error("'operation' is required"),
    };

    let operation = match raw_operation {
        "get_design" | "get_node_detail" | "get_design_context" | "read_nodes" | "inspect_node" | "inspect" | "get_node_info" | "node_detail" => "read_nodes",
        "get_tokens" | "tokens" if args.get("format").is_some() => "get_tokens",
        "get_tokens" | "tokens" => "get_variable_tokens",
        "export_icons" => "export_assets",
        other => other,
    };

    if !is_supported_read_operation(operation) {
        return ToolResult::error(format!(
            "Unknown figma_read operation '{}'. Available: get_selection, get_design, get_page_nodes, screenshot, export_svg, get_styles, get_local_components, get_viewport, get_variables, get_tokens, get_node_detail, get_css, get_design_context, get_component_map, get_unmapped_components, export_image, export_assets, search_nodes, scan_design",
            raw_operation
        ));
    }

    let session_id = args.get("sessionId").and_then(|v| v.as_str());
    if !bridge.is_plugin_connected(session_id).await {
        return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
    }

    // Forward every argument through to the handler (nodeId/nodeName are
    // renamed to the plugin's id/name). A whitelist here silently
    // swallowed handler options such as maxNodes / absolute /
    // keepViewport / inlineIcons; handlers ignore what they don't use.
    let mut op_params = json!({});
    if let Some(obj) = args.as_object() {
        for (k, v) in obj {
            match k.as_str() {
                "operation" | "op" | "action" | "command" | "outputPath" | "sessionId" => {}
                "nodeId" | "node_id" | "targetId" | "target_id" => { op_params["id"] = v.clone(); }
                "nodeName" | "node_name" => { op_params["name"] = v.clone(); }
                _ => { op_params[k] = v.clone(); }
            }
        }
    }

    if operation == "read_nodes" {
        if op_params.get("cursor").is_none() {
            if raw_operation == "get_node_detail" || raw_operation == "get_node_info" || raw_operation == "node_detail" {
                op_params["depth"] = json!(0);
            }
            if op_params.get("fields").is_none() && raw_operation != "read_nodes" {
                op_params["fields"] = json!(["geometry", "content", "text", "style", "layout", "tokens", "component"]);
            }
        }
        return match bridge.send_operation("read_nodes", op_params, session_id).await {
            Ok(data) => ToolResult::text(data.to_string()),
            Err(error) => ToolResult::error(error),
        };
    }
    if operation == "search_nodes" {
        let query = args["query"].as_str().unwrap_or("");
        let node_type = args.get("type").or_else(|| args.get("nodeType")).and_then(Value::as_str);
        let limit = args["limit"].as_u64().unwrap_or(30) as usize;
        let results = bridge.search_index_nodes(session_id, query, node_type, limit).await;
        let stats = bridge.get_index_stats(session_id).await;
        let nodes = results.unwrap_or_default();
        return ToolResult::text(json!({"query":query,"nodes":nodes,"count":nodes.len(),
            "cached":true,"scope":stats.as_ref().map(|stats| &stats.scopes),
            "complete":stats.as_ref().is_some_and(|stats| stats.complete),
            "hint":"Search covers indexed scopes; refresh nodeId to expand a frame."}).to_string());
    }


    // Fast-path read from index cache if available for read-only catalog queries
    if ["get_styles", "get_variables", "get_local_components"].contains(&operation) {
        if let BridgeHandle::Direct(ref state) = bridge {
            let inner = state.inner.lock().await;
            let sid = crate::bridge::server::BridgeState::resolve_session_id(&inner, session_id);
            if let Some(session) = inner.sessions.get(&sid) {
                if let Some(ref idx) = session.index {
                    if idx.is_ready() {
                        if operation == "get_styles" || operation == "get_variables" {
                            if let Some((styles, vars)) = idx.token_snapshot() {
                                let mut data = if operation == "get_styles" { styles } else { vars };
                                data["cached"] = json!(true);
                                return ToolResult::text(serde_json::to_string(&data).unwrap_or_default());
                            }
                        } else if operation == "get_local_components" && idx.stats.components_indexed {
                            if let Some(data) = &idx.raw_components {
                                let mut data = data.clone();
                                data["cached"] = json!(true);
                                return ToolResult::text(serde_json::to_string(&data).unwrap_or_default());
                            }
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
                            if let Some(n) = matched.filter(|node| node.has_details()) {
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

        let (styles_data, vars_data) = match (styles_res, vars_res) {
            (Ok(s), Ok(v)) => (s, v),
            (Err(e), _) | (_, Err(e)) => return ToolResult::error(format!("Token extraction failed: {e}")),
        };

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

            if operation == "get_design" || operation == "get_selection" {
                let optimized = crate::mcp::semantic_optimizer::compress_tree(&data, args.get("detail").and_then(|v| v.as_str()).unwrap_or("full") != "full");
                return ToolResult::text(serde_json::to_string(&optimized).unwrap_or_default());
            }

            ToolResult::text(serde_json::to_string(&data).unwrap_or_default())
        }
        Err(e) => ToolResult::error(e),
    }
}

pub(super) async fn figma_rules(bridge: BridgeHandle, args: Value) -> ToolResult {
    let session_id = args.get("sessionId").and_then(|v| v.as_str());
    if !bridge.is_plugin_connected(session_id).await {
        return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
    }

    // Fast-path: If index is cached and ready, build rules directly from memory (< 1ms!)
    if let BridgeHandle::Direct(ref state) = bridge {
        let inner = state.inner.lock().await;
        let sid = crate::bridge::server::BridgeState::resolve_session_id(&inner, session_id);
        if let Some(session) = inner.sessions.get(&sid) {
            if let Some(ref idx) = session.index {
                if idx.is_ready() && idx.stats.components_indexed {
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
                    lines.push("_Generated by figma-rust-mcp figma_rules (in-memory cached)._".to_string());
                    return ToolResult::text(lines.join("\n"));
                }
            }
        }
    }

    let styles_fut = bridge.send_operation("get_styles", json!({}), session_id);
    let vars_fut = bridge.send_operation("get_variables", json!({}), session_id);
    let comps_fut = bridge.send_operation("get_local_components", json!({}), session_id);

    let (styles_res, vars_res, comps_res) = tokio::join!(styles_fut, vars_fut, comps_fut);

    let styles_data = match styles_res {
        Ok(data) => data,
        Err(e) => return ToolResult::error(format!("Failed to load Figma styles: {}", e)),
    };
    let vars_data = match vars_res {
        Ok(data) => data,
        Err(e) => return ToolResult::error(format!("Failed to load Figma variables: {}", e)),
    };
    let comps_data = match comps_res {
        Ok(data) => data,
        Err(e) => return ToolResult::error(format!("Failed to load Figma components: {}", e)),
    };

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
    lines.push("_Generated by figma-rust-mcp figma_rules. Re-run when design system changes._".to_string());

    ToolResult::text(lines.join("\n"))
}

pub(super) async fn figma_index(bridge: BridgeHandle, args: Value) -> ToolResult {
    let session_id = args.get("sessionId").and_then(|v| v.as_str());
    let operation = match args.get("operation").and_then(|v| v.as_str()) {
        Some(op) => op,
        None => return ToolResult::error("'operation' is required (status, search_nodes, get_node, search_components, search_styles, search_variables, refresh)"),
    };

    match operation {
        "subtree" | "typography" => {
            let mut params = json!({});
            if let Some(cursor) = args.get("cursor") { params["cursor"] = cursor.clone(); }
            else {
                let Some(id) = args["nodeId"].as_str() else { return ToolResult::error("nodeId is required for the first page"); };
                params["id"] = json!(id);
                params["depth"] = if operation == "typography" { json!("full") } else { json!(2) };
                params["fields"] = if operation == "typography" { json!(["geometry","text"]) } else { json!(["geometry","content"]) };
                for key in ["fields","depth","includeHidden","expandInstances"] {
                    if let Some(value) = args.get(key) { params[key] = value.clone(); }
                }
            }
            if let Some(limit) = args.get("limit") { params["limit"] = limit.clone(); }
            if args.get("maxNodes").is_some() { return ToolResult::error("Use limit (1..500) and nextCursor instead of maxNodes"); }
            return match bridge.send_operation("read_nodes", params, session_id).await {
                Ok(data) => ToolResult::text(data.to_string()),
                Err(error) => ToolResult::error(error),
            };
        }
        "status" => {
            let stats = bridge.get_index_stats(session_id).await;
            let connected = bridge.is_plugin_connected(session_id).await;
            let cache = if let BridgeHandle::Direct(state) = &bridge {
                let inner = state.inner.lock().await;
                let sid = crate::bridge::server::BridgeState::resolve_session_id(&inner, session_id);
                inner.sessions.get(&sid).map(|s| json!({"hits":s.cache_hits,"misses":s.cache_misses,
                    "bridgeCalls":s.bridge_calls,"bridgeTimeMs":s.bridge_time_ms,"entries":s.read_cache.len(),
                    "indexReady":s.index.as_ref().is_some_and(|idx| idx.is_ready())}))
            } else { None };
            let tool_timings = if let BridgeHandle::Direct(state) = &bridge {
                let inner = state.inner.lock().await;
                Some(inner.tool_timings.iter().map(|(name,(calls,total))| (name.clone(),json!({"calls":calls,"totalMs":total,"avgMs":total / calls.max(&1)}))).collect::<serde_json::Map<String,Value>>())
            } else { None };
            match stats {
                Some(st) => {
                    let out = json!({
                        "status": if cache.as_ref().is_some_and(|c| c["indexReady"] == false) { "stale" } else { "ready" },
                        "pluginConnected": connected,
                        "stats": st,
                        "cache": cache,
                        "toolTimings": tool_timings,
                    });
                    ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                }
                None => {
                    let out = json!({
                        "status": "not_indexed",
                        "cache": cache,
                        "toolTimings": tool_timings,
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

            let mut params = json!({"id":node_id, "depth":0,
                "fields":["geometry","content","text","style","layout","tokens","component"]});
            for key in ["fields", "includeHidden", "expandInstances"] {
                if let Some(value) = args.get(key) { params[key] = value.clone(); }
            }
            match bridge.send_operation("read_nodes", params, session_id).await {
                Ok(data) => ToolResult::text(data.to_string()),
                Err(error) => ToolResult::error(error),
            }
        }

        "search_nodes" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let node_type = args.get("nodeType").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize;

            match bridge.search_index_nodes(session_id, query, node_type, limit).await {
                Some(results) => {
                    let stats = bridge.get_index_stats(session_id).await;
                    let out = json!({
                        "query": query,
                        "nodeType": node_type,
                        "count": results.len(),
                        "cached": true,
                        "results": results,
                        "scope": stats.as_ref().map(|stats| &stats.scopes),
                        "complete": stats.as_ref().is_some_and(|stats| stats.complete),
                    });
                    ToolResult::text(serde_json::to_string_pretty(&out).unwrap_or_default())
                }
                None => {
                    ToolResult::text(json!({"results":[], "count":0, "scope":[], "complete":false,
                        "status":"not_indexed", "hint":"Refresh the active page, then read or refresh one frame to expand its scope."}).to_string())
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
            let mut params = json!({"deferComponents": args["includeComponents"] != true});
            for key in ["depth", "includeHidden", "expandInstances"] {
                if let Some(value) = args.get(key) { params[key] = value.clone(); }
            }
            if let Some(id) = args.get("nodeId") { params["id"] = id.clone(); }
            match bridge.send_operation("index_scan", params, session_id).await {
                Ok(data) => {
                    if data["cancelled"] == true {
                        return ToolResult::error("Index refresh cancelled because the active page changed.");
                    }
                    let page_nodes = data.get("pageNodes").unwrap_or(&Value::Null);
                    let styles = data.get("styles");
                    let vars = data.get("variables");
                    let comps = data.get("components");
                    let file_name = data.get("fileName").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let sid = match &bridge {
                        BridgeHandle::Direct(state) => state.resolved_session_id(session_id).await,
                        BridgeHandle::Proxy(_) => session_id.unwrap_or("_default").to_string(),
                    };

                    let stats = if data["nodesStreamed"] == true {
                        // index-update arrived before this reply; keep its streamed descendants.
                        match bridge.get_index_stats(session_id).await {
                            Some(stats) => stats,
                            None => return ToolResult::error("Streamed index was not received; retry refresh."),
                        }
                    } else {
                        let idx = crate::bridge::index::FigmaIndex::from_raw(
                            &sid,
                            file_name,
                            page_nodes,
                            styles,
                            vars,
                            comps,
                            start_ms,
                        );
                        let stats = idx.stats.clone();
                        if let BridgeHandle::Direct(ref state) = bridge {
                            state.update_index(&sid, idx).await;
                        }
                        stats
                    };

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
