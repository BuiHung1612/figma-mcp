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
            let mut errors = Vec::new();

            // Save SVG icons
            if let Some(icons) = data.get("icons").and_then(|v| v.as_array()) {
                let target_icon_dir = icon_dir.unwrap_or("src/assets/icons");
                let dir_path = std::path::Path::new(target_icon_dir);
                if let Err(e) = tokio::fs::create_dir_all(dir_path).await { errors.push(format!("{target_icon_dir}: {e}")); }

                for item in icons {
                    let file_name = item.get("fileName").and_then(|v| v.as_str()).unwrap_or("icon.svg");
                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("icon");
                    let svg = item.get("svg").and_then(|v| v.as_str()).unwrap_or("");

                    let file_path = dir_path.join(file_name);
                    if let Err(e) = tokio::fs::write(&file_path, svg.as_bytes()).await {
                        errors.push(format!("{}: {e}", file_path.display()));
                    } else {
                        let abs = std::fs::canonicalize(&file_path).unwrap_or(file_path.clone());
                        exported_icons.push(json!({
                            "name": name,
                            "path": abs.to_string_lossy(),
                            "file": file_name,
                            "sizeBytes": svg.len(),
                        }));

                        let comp_name = name.split(|c: char| !c.is_ascii_alphanumeric()).map(|w| {
                            let mut c = w.chars();
                            match c.next() {
                                None => String::new(),
                                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                            }
                        }).collect::<String>() + "Icon";
                        let comp_name = if comp_name.starts_with(|c: char| c.is_ascii_digit()) { format!("_{comp_name}") } else { comp_name };
                        barrel_lines.push(format!("export {{ default as {} }} from './{}';", comp_name, file_name));
                    }
                }

                if create_barrel && !barrel_lines.is_empty() {
                    // Append to an existing barrel: it may hold earlier batches or hand-written exports.
                    let barrel_path = dir_path.join("index.ts");
                    let barrel = merge_barrel(tokio::fs::read_to_string(&barrel_path).await.unwrap_or_default(), &barrel_lines);
                    if let Err(e) = tokio::fs::write(&barrel_path, barrel).await { errors.push(format!("{}: {e}", barrel_path.display())); }
                }
            }

            // Save raster images
            if let Some(images) = data.get("images").and_then(|v| v.as_array()) {
                let target_img_dir = image_dir.unwrap_or("public/images");
                let dir_path = std::path::Path::new(target_img_dir);
                if let Err(e) = tokio::fs::create_dir_all(dir_path).await { errors.push(format!("{target_img_dir}: {e}")); }

                for item in images {
                    let file_name = item.get("fileName").and_then(|v| v.as_str()).unwrap_or("image.png");
                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("image");

                    if let Some(data_url) = item.get("dataUrl").and_then(|v| v.as_str()) {
                        let b64 = if let Some(idx) = data_url.find(',') {
                            &data_url[idx + 1..]
                        } else {
                            data_url
                        };

                        let bytes = match BASE64_STANDARD.decode(b64.trim()) {
                            Ok(bytes) => bytes,
                            Err(e) => { errors.push(format!("{file_name}: invalid image data ({e})")); continue; }
                        };
                        {
                            let file_path = dir_path.join(file_name);
                            if let Err(e) = tokio::fs::write(&file_path, &bytes).await {
                                errors.push(format!("{}: {e}", file_path.display()));
                            } else {
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
                "success": errors.is_empty(),
                "errors": errors,
                "pluginFailures": data.get("failures"),
                "truncated": data.get("truncated"),
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

/// Adds the export lines an icon barrel is missing, keeping everything already in it.
fn merge_barrel(mut barrel: String, lines: &[String]) -> String {
    for line in lines {
        if !barrel.lines().any(|existing| existing.trim() == line) {
            if !barrel.is_empty() && !barrel.ends_with('\n') { barrel.push('\n'); }
            barrel.push_str(line);
            barrel.push('\n');
        }
    }
    barrel
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

/// Most variants fetched per component set; the rest are reported, not guessed.
const MAX_VARIANTS: usize = 24;

/// Picks the local components to scaffold, grouping set variants under one name,
/// deduplicated by generated name, skipping names the codebase already has.
/// Each entry is (name, [(node id, variant label)]); standalone components have one empty label.
#[allow(clippy::type_complexity)]
fn scaffold_plan(local: &Value, existing: &crate::mcp::component_matcher::ComponentScanResult, overwrite: bool, limit: usize) -> (Vec<(String, Vec<(String, String)>)>, Vec<Value>) {
    let mut decided = std::collections::HashMap::<String, Option<usize>>::new();
    let (mut picked, mut skipped) = (Vec::<(String, Vec<(String, String)>)>::new(), Vec::new());
    for c in local["components"].as_array().into_iter().flatten() {
        let figma_name = c["setName"].as_str().or(c["name"].as_str()).unwrap_or("Component");
        let name = crate::mcp::codegen::sanitize_component_name(figma_name);
        let variant = (c["id"].as_str().unwrap_or_default().to_string(), if c["setName"].is_string() { c["name"].as_str().unwrap_or_default().to_string() } else { String::new() });
        if let Some(slot) = decided.get(&name) {
            // Same set: another variant. A different component with a clashing name is dropped.
            if let Some(i) = slot.filter(|_| c["setName"].is_string() && !variant.1.is_empty()) { picked[i].1.push(variant); }
            continue;
        }
        let slot = if let Some(found) = existing.components.get(&name.to_lowercase()).filter(|_| !overwrite) {
            skipped.push(json!({"name": name, "reason": format!("already in codebase: {}", found.file_path)}));
            None
        } else if picked.len() >= limit {
            skipped.push(json!({"name": name, "reason": format!("limit {limit} reached")}));
            None
        } else {
            picked.push((name.clone(), vec![variant]));
            Some(picked.len() - 1)
        };
        decided.insert(name, slot);
    }
    (picked, skipped)
}

/// "Size=sm, State=Hover" → [("size", "sm"), ("state", "Hover")].
fn variant_props(label: &str) -> Vec<(String, String)> {
    label.split(',').filter_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        let pascal = crate::mcp::codegen::sanitize_component_name(key.trim());
        let mut chars = pascal.chars();
        let prop = chars.next().map(|f| f.to_lowercase().collect::<String>() + chars.as_str())?;
        Some((prop, value.trim().to_string()))
    }).collect()
}

/// Joins per-variant React modules into one file with a wrapper that picks the variant from props.
/// `parts` are (variant label, component name, generated module); the first is the default.
fn compose_variants(name: &str, parts: &[(String, String, String)]) -> String {
    let mut imports = Vec::<String>::new();
    let mut bodies = String::new();
    for (_, _, code) in parts {
        for line in code.lines() {
            if line.starts_with("import ") {
                if !imports.iter().any(|seen| seen == line) { imports.push(line.to_string()); }
            } else if !line.starts_with("export default ") {
                bodies.push_str(line);
                bodies.push('\n');
            }
        }
    }
    let defaults = variant_props(&parts[0].0);
    let props: Vec<&String> = defaults.iter().map(|(prop, _)| prop).collect();
    let key = |label: &str| {
        let values: std::collections::HashMap<_, _> = variant_props(label).into_iter().collect();
        props.iter().map(|p| format!("{p}={}", values.get(*p).map_or("", String::as_str))).collect::<Vec<_>>().join("|")
    };
    let quote = |v: &str| serde_json::to_string(v).unwrap_or_default();
    let unions = props.iter().map(|prop| {
        let mut values: Vec<String> = parts.iter().filter_map(|(label, _, _)| variant_props(label).into_iter().find(|(p, _)| p == *prop).map(|(_, v)| quote(&v))).collect();
        values.dedup();
        let mut seen = std::collections::HashSet::new();
        values.retain(|v| seen.insert(v.clone()));
        format!("  {prop}?: {};\n", values.join(" | "))
    }).collect::<String>();
    let table = parts.iter().map(|(label, component, _)| format!("  {}: {component},\n", quote(&key(label)))).collect::<String>();
    let params = defaults.iter().map(|(prop, value)| format!("{prop} = {}, ", quote(value))).collect::<String>();
    let lookup = props.iter().map(|p| format!("`{p}=${{{p}}}`")).collect::<Vec<_>>().join(", ");
    format!(
        "{imports}\n\n{bodies}\nexport interface {name}Props {{\n{unions}  className?: string;\n}}\n\nconst variants: Record<string, React.FC<{{ className?: string }}>> = {{\n{table}}};\n\nexport const {name}: React.FC<{name}Props> = ({{ {params}className = '' }}) => {{\n  const Variant = variants[[{lookup}].join('|')] ?? {default};\n  return <Variant className={{className}} />;\n}};\n\nexport default {name};\n",
        imports = imports.join("\n"),
        default = parts[0].1,
    )
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
    for (name, variants) in &plan {
        let path = component_dir.join(format!("{name}.{ext}"));
        if !overwrite && path.exists() {
            record(&path, Ok(false), &mut written, &mut skipped, &mut errors);
            continue;
        }
        // Each variant is generated from its own node; vue/html keep the default variant only.
        let take = if ext == "tsx" { variants.len().min(MAX_VARIANTS) } else { 1 };
        if variants.len() > take {
            skipped.push(json!({"name": name, "reason": format!("{} of {} variants generated", take, variants.len())}));
        }
        let mut parts = Vec::new();
        for (id, label) in &variants[..take] {
            let component = if take > 1 { format!("{name}{}", crate::mcp::codegen::sanitize_component_name(label)) } else { name.clone() };
            match bridge.send_operation("get_design_context", json!({"id": id, "expandInstances": true}), session_id).await
                .and_then(|ctx| crate::mcp::codegen::generate_code_from_context(&ctx, framework, Some(&component))) {
                Ok(code) => parts.push((label.clone(), component, code)),
                Err(e) => errors.push(json!(format!("{name} ({id}): {e}"))),
            }
        }
        let code = match parts.len() {
            0 => continue,
            1 => parts.remove(0).2,
            _ => compose_variants(name, &parts),
        };
        let r = write_scaffold_file(&path, &code, overwrite).await;
        record(&path, r, &mut written, &mut skipped, &mut errors);
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
    #[test]
    fn icon_barrel_keeps_earlier_exports() {
        let existing = "export { default as HomeIcon } from './home.svg';\nexport * from './custom';".to_string();
        let lines = ["export { default as HomeIcon } from './home.svg';".to_string(), "export { default as _2faIcon } from './2fa.svg';".to_string()];
        let merged = super::merge_barrel(existing, &lines);
        assert_eq!(merged, "export { default as HomeIcon } from './home.svg';\nexport * from './custom';\nexport { default as _2faIcon } from './2fa.svg';\n");
    }

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
        assert_eq!(picked[0], ("Button".to_string(), vec![("1:1".to_string(), "Size=sm".to_string()), ("1:2".to_string(), "Size=lg".to_string())]));
        assert_eq!(picked[1], ("IconButton".to_string(), vec![("3:1".to_string(), String::new())]));
        assert_eq!(skipped[0]["name"], "Card");
        assert_eq!(skipped[1]["reason"], "limit 2 reached");

        let (picked, _) = scaffold_plan(&local, &existing, true, 10);
        assert_eq!(picked.len(), 4);
    }

    #[test]
    fn variants_compose_into_one_component_selected_by_props() {
        let module = |n: &str| format!("import React from 'react';\n\nexport interface {n}Props {{ className?: string }}\n\nexport const {n}: React.FC<{n}Props> = ({{\n  className = '',\n}}) => {{  return (\n    <div />\n  );\n}};\n\nexport default {n};\n");
        let parts = vec![
            ("Size=sm, Has Icon=false".to_string(), "ButtonSizeSmHasIconFalse".to_string(), module("ButtonSizeSmHasIconFalse")),
            ("Size=lg, Has Icon=true".to_string(), "ButtonSizeLgHasIconTrue".to_string(), module("ButtonSizeLgHasIconTrue")),
        ];
        let code = super::compose_variants("Button", &parts);
        assert_eq!(code.matches("import React from 'react';").count(), 1);
        assert_eq!(code.matches("export default ").count(), 1);
        assert!(code.contains("  size?: \"sm\" | \"lg\";\n  hasIcon?: \"false\" | \"true\";"), "{code}");
        assert!(code.contains("\"size=lg|hasIcon=true\": ButtonSizeLgHasIconTrue,"), "{code}");
        assert!(code.contains("({ size = \"sm\", hasIcon = \"false\", className = '' })"), "{code}");
        assert!(code.contains("variants[[`size=${size}`, `hasIcon=${hasIcon}`].join('|')] ?? ButtonSizeSmHasIconFalse"), "{code}");
    }
}
