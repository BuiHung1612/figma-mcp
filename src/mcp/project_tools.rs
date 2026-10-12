//! `figma_export_assets`, `figma_verify_ui`, `figma_prepare_design` and `figma_scaffold_project`, split out of `server::handle_tool_call_inner`.

use super::protocol::ToolResult;
use base64::prelude::*;
use crate::bridge::BridgeHandle;
use serde_json::{json, Value};

pub(super) async fn figma_export_assets(bridge: BridgeHandle, args: Value) -> ToolResult {
    let session_id = args.get("sessionId").and_then(|v| v.as_str());
    if !bridge.is_plugin_connected(session_id).await {
        return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
    }

    let icon_dir = args.get("iconDir").and_then(|v| v.as_str());
    let image_dir = args.get("imageDir").and_then(|v| v.as_str());
    let create_barrel = args.get("createBarrel").and_then(|v| v.as_bool()).unwrap_or(true);

    let mut op_params = json!({});
    let node_id = args.get("nodeId")
        .or_else(|| args.get("id"))
        .or_else(|| args.get("node_id"))
        .or_else(|| args.get("targetId"))
        .or_else(|| args.get("target_id"));
    if let Some(nid) = node_id { op_params["id"] = nid.clone(); }

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

pub(super) async fn figma_verify_ui(bridge: BridgeHandle, args: Value) -> ToolResult {
    let session_id = args.get("sessionId").and_then(|v| v.as_str());
    let node_id = args.get("nodeId").and_then(|v| v.as_str());
    let node_name = args.get("nodeName").and_then(|v| v.as_str());
    let target_url = args.get("url").and_then(|v| v.as_str());
    let target_selector = args.get("selector").and_then(|v| v.as_str());

    // 1. Resolve target Figma design spec
    let mut figma_spec: Option<Value> = None;
    let mut resolved_node_id = node_id.map(String::from);
    let mut resolved_node_name = node_name.map(String::from).unwrap_or_else(|| "Unknown".to_string());

    // Try resolving from In-Memory Index first
    if let BridgeHandle::Direct(ref state) = bridge {
        let inner = state.inner.lock().await;
        let sid = crate::bridge::server::BridgeState::resolve_session_id(&inner, session_id);
        if let Some(session) = inner.sessions.get(&sid) {
            if let Some(ref idx) = session.index {
                if idx.is_ready() {
                    let matched = if let Some(ref id) = resolved_node_id {
                        idx.get_node(id)
                    } else if let Some(name) = node_name {
                        idx.get_node_by_name(name)
                    } else if let Some(ref active) = session.active_selection {
                        if let Some(first) = active.selection.first() {
                            resolved_node_id = first.get("id").and_then(|v| v.as_str()).map(String::from);
                            if let Some(ref id) = resolved_node_id {
                                idx.get_node(id)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(node) = matched {
                        resolved_node_id = Some(node.id.clone());
                        resolved_node_name = node.name.clone();
                        if node.has_details() { figma_spec = Some(node.to_css_spec()); }
                    }
                }
            }
        }
    }

    // Fallback to Live Plugin Query if not found in index
    if figma_spec.is_none() {
        if !bridge.is_plugin_connected(session_id).await {
            return ToolResult::error("Figma plugin not connected and node not found in memory index. Please connect the plugin or provide a valid indexed nodeId.");
        }

        let mut op_params = json!({});
        if let Some(ref id) = resolved_node_id { op_params["id"] = json!(id); }
        if let Some(name) = node_name { op_params["name"] = json!(name); }

        op_params["expandInstances"] = json!(true);
        match bridge.send_operation("get_design_context", op_params, session_id).await {
            Ok(data) => {
                resolved_node_name = data.get("name").and_then(|v| v.as_str()).unwrap_or(&resolved_node_name).to_string();
                if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                    resolved_node_id = Some(id.to_string());
                }
                figma_spec = Some(data);
            }
            Err(e) => return ToolResult::error(format!("Failed to retrieve Figma design spec: {}", e)),
        }
    }

    let spec = match figma_spec {
        Some(s) => s,
        None => return ToolResult::error("Could not resolve target Figma node spec for verification."),
    };

    // 2. Parse computed styles
    let mut computed_map = std::collections::HashMap::new();
    if let Some(comp_val) = args.get("computedStyles").and_then(|v| v.as_object()) {
        for (k, v) in comp_val {
            if let Some(str_val) = v.as_str() {
                computed_map.insert(k.clone(), str_val.to_string());
            } else {
                computed_map.insert(k.clone(), v.to_string());
            }
        }
    }

    // 3. Perform comparison metrics
    let (layout_diffs, style_diffs, fixes, percentage) = crate::mcp::verify::compare_design_metrics(&spec, &computed_map);

    let mut report = crate::mcp::verify::VerificationReport::new(
        resolved_node_id.as_deref().unwrap_or("unknown"),
        &resolved_node_name,
    );
    report.target_url = target_url.map(String::from);
    report.target_selector = target_selector.map(String::from);
    report.match_percentage = (percentage * 10.0).round() / 10.0;
    report.layout_discrepancies = layout_diffs;
    report.style_discrepancies = style_diffs;
    report.actionable_fixes = fixes;
    report.visual_summary = format!(
        "UI Verification: {:.1}% match with Figma node '{}' ({})",
        report.match_percentage, report.node_name, report.node_id
    );

    ToolResult::text(serde_json::to_string_pretty(&report).unwrap_or_default())
}

pub(super) async fn figma_prepare_design(bridge: BridgeHandle, args: Value) -> ToolResult {
    let session_id = args.get("sessionId").and_then(|v| v.as_str());
    let node_id = args.get("nodeId").and_then(|v| v.as_str());
    let icon_dir = args.get("iconDir").and_then(|v| v.as_str()).unwrap_or("src/assets/icons");
    let project_dir = args.get("projectDir").and_then(|v| v.as_str()).unwrap_or(".");

    if !bridge.is_plugin_connected(session_id).await {
        return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
    }

    let mut params = json!({"depth":"full","expandInstances":true,"limit":500,
        "fields":["geometry","content","text","style","layout","tokens","component"]});
    if let Some(id) = node_id { params["id"] = json!(id); }
    let design_context = match super::spec::read_all(&bridge, params, session_id).await {
        Ok(data) => data,
        Err(e) => return ToolResult::error(format!("Failed to retrieve design context: {}", e)),
    };
    if design_context["scope"]["id"] == design_context["pageId"] || design_context["nodes"].as_array().is_none_or(Vec::is_empty) {
        return ToolResult::error("Select a frame or provide nodeId before preparing a design.");
    }
    let root = &design_context["nodes"][0];
    let resolved_id = root["id"].as_str().unwrap_or("").to_string();
    let resolved_name = root["name"].as_str().unwrap_or("Screen").to_string();
    let all_texts: Vec<_> = design_context["nodes"].as_array().into_iter().flatten()
        .flat_map(crate::mcp::design_pack::extract_all_text_elements).collect();
    let text_coverage = super::spec::coverage(&design_context);

    // 3. Batch export all vector icons to local project assets folder
    let mut export_params = json!({});
    if !resolved_id.is_empty() {
        export_params["nodeId"] = json!(resolved_id);
    }
    let mut warnings = Vec::new();
    if text_coverage["subtreeComplete"] != true { warnings.push("Text traversal is incomplete; read remaining scope before implementation.".to_string()); }
    let raw_assets = match bridge.send_operation("export_assets", export_params, session_id).await {
        Ok(data) => data,
        Err(e) => {
            warnings.push(format!("Asset export failed: {}", e));
            json!({})
        }
    };
    if raw_assets.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false) {
        warnings.push(format!(
            "Asset export was capped: discovered {}, inspected {}.",
            raw_assets.get("discovered").and_then(|v| v.as_u64()).unwrap_or(0),
            raw_assets.get("inspected").and_then(|v| v.as_u64()).unwrap_or(0)
        ));
    }
    if let Some(failures) = raw_assets.get("failures").and_then(|v| v.as_array()) {
        for failure in failures {
            let name = failure.get("name").and_then(|v| v.as_str()).unwrap_or("asset");
            let error = failure.get("error").and_then(|v| v.as_str()).unwrap_or("unknown export error");
            warnings.push(format!("Asset '{}' export failed: {}", name, error));
        }
    }
    
    let mut exported_icons = Vec::new();
    if let Some(icons_arr) = raw_assets.get("icons").and_then(|v| v.as_array()) {
        let dir_path = std::path::Path::new(icon_dir);
        if let Err(e) = tokio::fs::create_dir_all(dir_path).await { return ToolResult::error(e.to_string()); }
        let mut saved = Vec::new();
        for item in icons_arr {
            let file_name = item.get("fileName").and_then(|v| v.as_str()).unwrap_or("icon.svg");
            if let Some(svg_content) = item.get("svg").and_then(|v| v.as_str()) {
                match tokio::fs::write(dir_path.join(file_name), svg_content).await {
                    Ok(()) => saved.push(item.clone()),
                    Err(e) => warnings.push(format!("Could not save {}: {}", file_name, e)),
                }
            }
        }
        exported_icons = crate::mcp::design_pack::generate_icon_specs(&saved, args["_importIconDir"].as_str().unwrap_or(icon_dir));
    }

    // 4. Capture Canvas Visual Screenshot
    let mut snap_params = json!({ "format": "PNG", "scale": 1.5 });
    if !resolved_id.is_empty() {
        snap_params["id"] = json!(resolved_id);
    }
    let screenshot_res = match bridge.send_operation("screenshot", snap_params, session_id).await {
        Ok(data) => Some(data),
        Err(e) => {
            warnings.push(format!("Screenshot capture failed: {}", e));
            None
        }
    };
    let screenshot_data_url = screenshot_res.as_ref().and_then(|v| v.get("dataUrl")).and_then(|v| v.as_str());
    
    let mut local_screenshot_path = None;
    if let Some(data_url) = screenshot_data_url {
        let b64 = if let Some(idx) = data_url.find(',') {
            &data_url[idx + 1..]
        } else {
            data_url
        };
        if let Ok(bytes) = BASE64_STANDARD.decode(b64.trim()) {
            let temp_path = std::env::temp_dir().join(format!("figma_preview_{}.png", resolved_id.replace([':', ';', '/'], "_")));
            if tokio::fs::write(&temp_path, &bytes).await.is_ok() {
                local_screenshot_path = Some(temp_path.to_string_lossy().to_string());
            }
        }
    }

    // 5. Scan codebase for component reuse
    let scan_result = crate::mcp::component_matcher::scan_project_components(project_dir).await;
    let mut matched_components = std::collections::HashMap::new();
    for comp in scan_result.components.values() {
        matched_components.insert(comp.name.clone(), comp.import_path.clone());
    }

    // 6. Fetch resolved design tokens & color palette
    let mut resolved_color_tokens = std::collections::HashMap::new();
    let mut color_palette = Vec::new();
    if let Ok(token_data) = bridge.send_operation("get_variable_tokens", json!({}), session_id).await {
        if let Some(res_map) = token_data.get("resolvedTokens").and_then(|v| v.as_object()) {
            for (k, v) in res_map {
                if let Some(val_str) = v.as_str() {
                    resolved_color_tokens.insert(k.clone(), val_str.to_string());
                    if val_str.starts_with('#') {
                        color_palette.push(json!({
                            "token": k,
                            "hex": val_str
                        }));
                    }
                }
            }
        }
    }

    // 7. Build Implementation Checklist for AI
    let mut checklist = Vec::new();
    checklist.push(format!("Build layout container for '{}' (dimensions & padding)", resolved_name));
    if !exported_icons.is_empty() {
        checklist.push(format!("Import and render {} extracted SVG icons from '{}'", exported_icons.len(), icon_dir));
    }
    if !all_texts.is_empty() {
        checklist.push(format!("Verify {} extracted text elements; check coverage.text.subtreeComplete before claiming all text is covered", all_texts.len()));
    }
    if !color_palette.is_empty() {
        checklist.push(format!("Use {} exact resolved design token colors (no guessing)", color_palette.len()));
    }

    let result_pack = json!({
        "success": true,
        "coverage": {"text":text_coverage,"assets":{"discovered":raw_assets["discovered"],"inspected":raw_assets["inspected"],"exportedIcons":exported_icons.len(),"failures":raw_assets["failures"],"complete":raw_assets.get("icons").and_then(Value::as_array).is_some_and(|icons| icons.len() == exported_icons.len()) && raw_assets.get("truncated").and_then(Value::as_bool) == Some(false) && raw_assets.get("failures").and_then(Value::as_array).is_some_and(Vec::is_empty)}},
        "nodeId": resolved_id,
        "nodeName": resolved_name,
        "screenshotPreviewPath": local_screenshot_path,
        "totalVisibleTexts": all_texts.len(),
        "allVisibleTexts": all_texts,
        "totalExportedIcons": exported_icons.len(),
        "exportedIcons": exported_icons,
        "resolvedColorTokens": resolved_color_tokens,
        "colorPalette": color_palette,
        "discoveredCodebaseComponents": matched_components,
        "implementationChecklist": checklist,
        "warnings": warnings,
        "instructionForAI": "DO NOT guess or invent icons or colors. Use the exact exported SVG components listed in 'exportedIcons'. Refer to 'resolvedColorTokens' for exact semantic-to-hex mappings. Ensure every text in 'allVisibleTexts' is accounted for."
    });

    let mut result_pack = result_pack;
    if args["detail"].as_str() != Some("full") {
        result_pack.as_object_mut().unwrap().remove("colorPalette");
    }
    if let Some(path) = args["outputPath"].as_str() {
        let path = std::path::Path::new(path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            if let Err(e) = tokio::fs::create_dir_all(parent).await { return ToolResult::error(e.to_string()); }
        }
        if let Err(e) = tokio::fs::write(path, json!({"pack":result_pack,"design":design_context}).to_string()).await { return ToolResult::error(e.to_string()); }
        return ToolResult::text(json!({"savedTo":std::fs::canonicalize(path).unwrap_or(path.to_path_buf()),"coverage":result_pack["coverage"],"nodeId":resolved_id,"screenshotPreviewPath":local_screenshot_path,"warnings":result_pack["warnings"]}).to_string());
    }
    ToolResult::text(result_pack.to_string())
}

/// Picks the local components to scaffold: one entry per component set (its first
/// variant), deduplicated by generated name, skipping names the codebase already has.
fn scaffold_plan(local: &Value, existing: &crate::mcp::component_matcher::ComponentScanResult, overwrite: bool, limit: usize) -> (Vec<(String, String)>, Vec<Value>) {
    let mut seen = std::collections::HashSet::new();
    let (mut picked, mut skipped) = (Vec::new(), Vec::new());
    for c in local["components"].as_array().into_iter().flatten() {
        let figma_name = c["setName"].as_str().or(c["name"].as_str()).unwrap_or("Component");
        let name = crate::mcp::codegen::sanitize_component_name(figma_name);
        if !seen.insert(name.clone()) { continue; }
        if let Some(found) = existing.components.get(&name.to_lowercase()).filter(|_| !overwrite) {
            skipped.push(json!({"name": name, "reason": format!("already in codebase: {}", found.file_path)}));
        } else if picked.len() >= limit {
            skipped.push(json!({"name": name, "reason": format!("limit {limit} reached")}));
        } else {
            picked.push((c["id"].as_str().unwrap_or_default().to_string(), name));
        }
    }
    (picked, skipped)
}

/// Writes `content` unless the file exists and `overwrite` is false. Returns whether it wrote.
async fn write_scaffold_file(path: &std::path::Path, content: &str, overwrite: bool) -> Result<bool, String> {
    if !overwrite && path.exists() { return Ok(false); }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        tokio::fs::create_dir_all(parent).await.map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    tokio::fs::write(path, content).await.map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

pub(super) async fn figma_scaffold_project(bridge: BridgeHandle, args: Value) -> ToolResult {
    let session_id = args.get("sessionId").and_then(|v| v.as_str());
    if !bridge.is_plugin_connected(session_id).await {
        return ToolResult::error("Figma plugin not connected. Run the 'Figma Rust MCP Bridge' plugin in Figma Desktop first.");
    }
    let framework = args["framework"].as_str().unwrap_or("react-tailwind");
    let ext = match framework {
        "react-tailwind" | "react-shadcn" => "tsx",
        "vue-tailwind" => "vue",
        "html" => "html",
        _ => return ToolResult::error(format!("Unsupported framework '{framework}'. Available: react-tailwind, react-shadcn, vue-tailwind, html")),
    };
    let project = std::path::Path::new(args["projectDir"].as_str().unwrap_or("."));
    let at = |key: &str, default: &str| project.join(args[key].as_str().unwrap_or(default));
    let (component_dir, styles_path, tailwind_path) = (at("componentDir", "src/components/ui"), at("stylesPath", "src/styles/globals.css"), at("tailwindConfigPath", "tailwind.config.ts"));
    let overwrite = args["overwrite"].as_bool().unwrap_or(false);
    let limit = args["limit"].as_u64().unwrap_or(30).clamp(1, 100) as usize;

    let (mut written, mut skipped, mut errors) = (Vec::new(), Vec::new(), Vec::new());
    fn record(path: &std::path::Path, result: Result<bool, String>, written: &mut Vec<Value>, skipped: &mut Vec<Value>, errors: &mut Vec<Value>) {
        match result {
            Ok(true) => written.push(json!(path.to_string_lossy())),
            Ok(false) => skipped.push(json!({"path": path.to_string_lossy(), "reason": "file exists (pass overwrite: true)"})),
            Err(e) => errors.push(json!(e)),
        }
    }

    // Tokens: globals.css (CSS variables + Tailwind layers) and tailwind config.
    match super::server::token_sources(&bridge, session_id).await {
        Ok((styles, vars)) => {
            match crate::mcp::tokens::generate_tokens(&styles, &vars, "css", None, None, None) {
                Ok(css) => {
                    let content = format!("@tailwind base;\n@tailwind components;\n@tailwind utilities;\n\n{css}");
                    let r = write_scaffold_file(&styles_path, &content, overwrite).await;
                    record(&styles_path, r, &mut written, &mut skipped, &mut errors);
                }
                Err(e) => errors.push(json!(format!("css tokens: {e}"))),
            }
            if ext != "html" || args.get("tailwindConfigPath").is_some() {
                match crate::mcp::tokens::generate_tokens(&styles, &vars, "tailwind", None, None, None) {
                    Ok(config) => {
                        let content = config.replacen("module.exports = {", "export default {\n  \"content\": [\"./index.html\", \"./src/**/*.{js,ts,jsx,tsx,vue,html}\"],", 1);
                        let r = write_scaffold_file(&tailwind_path, &content, overwrite).await;
                        record(&tailwind_path, r, &mut written, &mut skipped, &mut errors);
                    }
                    Err(e) => errors.push(json!(format!("tailwind tokens: {e}"))),
                }
            }
        }
        Err(e) => errors.push(json!(e)),
    }

    // Components: one file per local component / component set.
    let local = match bridge.send_operation("get_local_components", json!({}), session_id).await {
        Ok(v) => v,
        Err(e) => return ToolResult::error(format!("Listing local components failed: {e}")),
    };
    let existing = crate::mcp::component_matcher::scan_project_components(&project.to_string_lossy()).await;
    let (plan, plan_skipped) = scaffold_plan(&local, &existing, overwrite, limit);
    skipped.extend(plan_skipped);
    for (id, name) in &plan {
        let path = component_dir.join(format!("{name}.{ext}"));
        if !overwrite && path.exists() {
            record(&path, Ok(false), &mut written, &mut skipped, &mut errors);
            continue;
        }
        let code = bridge.send_operation("get_design_context", json!({"id": id, "expandInstances": true}), session_id).await
            .and_then(|ctx| crate::mcp::codegen::generate_code_from_context(&ctx, framework, Some(name)));
        match code {
            Ok(code) => {
                let r = write_scaffold_file(&path, &code, overwrite).await;
                record(&path, r, &mut written, &mut skipped, &mut errors);
            }
            Err(e) => errors.push(json!(format!("{name} ({id}): {e}"))),
        }
    }

    ToolResult::text(serde_json::to_string_pretty(&json!({
        "success": errors.is_empty(),
        "framework": framework,
        "localComponents": local["components"].as_array().map_or(0, Vec::len),
        "written": written,
        "skipped": skipped,
        "errors": errors,
    })).unwrap_or_default())
}

#[cfg(test)]
mod scaffold_tests {
    use super::scaffold_plan;
    use crate::mcp::component_matcher::{ComponentScanResult, DiscoveredComponent};
    use serde_json::json;

    #[test]
    fn plan_groups_variants_skips_existing_and_respects_limit() {
        let local = json!({"components": [
            {"id": "1:1", "name": "Size=sm", "setName": "Button"},
            {"id": "1:2", "name": "Size=lg", "setName": "Button"},
            {"id": "2:1", "name": "card"},
            {"id": "3:1", "name": "Icon Button"},
            {"id": "4:1", "name": "Badge"},
        ]});
        let mut existing = ComponentScanResult::default();
        existing.components.insert("card".into(), DiscoveredComponent { name: "Card".into(), file_path: "src/components/Card.tsx".into(), import_path: "@/components/Card".into(), is_default_export: true, named_exports: vec![] });

        let (picked, skipped) = scaffold_plan(&local, &existing, false, 2);
        assert_eq!(picked, vec![("1:1".to_string(), "Button".to_string()), ("3:1".to_string(), "IconButton".to_string())]);
        assert_eq!(skipped[0]["name"], "Card");
        assert_eq!(skipped[1]["reason"], "limit 2 reached");

        let (picked, _) = scaffold_plan(&local, &existing, true, 10);
        assert_eq!(picked.len(), 4);
    }
}
