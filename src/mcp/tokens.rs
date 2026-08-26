use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn generate_tokens(
    styles_data: &Value,
    vars_data: &Value,
    format: &str,
    collection_filter: Option<&str>,
    mode_filter: Option<&str>,
    prefix_opt: Option<&str>,
) -> Result<String, String> {
    let fmt = format.to_lowercase();
    let prefix = prefix_opt.unwrap_or("");

    match fmt.as_str() {
        "css" => Ok(generate_css_tokens(styles_data, vars_data, collection_filter, mode_filter, prefix)),
        "tailwind" => Ok(generate_tailwind_tokens(styles_data, vars_data, collection_filter, mode_filter)),
        "typescript" | "ts" => Ok(generate_ts_tokens(styles_data, vars_data, collection_filter, mode_filter)),
        "w3c" => Ok(generate_w3c_tokens(styles_data, vars_data, collection_filter, mode_filter)),
        "json" => Ok(generate_json_tokens(styles_data, vars_data, collection_filter, mode_filter)),
        _ => Err(format!(
            "Unsupported format: '{}'. Available formats: 'css', 'tailwind', 'typescript', 'w3c', 'json'",
            format
        )),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn sanitize_token_name(name: &str) -> String {
    let s = name
        .trim()
        .replace('/', "-")
        .replace(' ', "-")
        .replace('_', "-")
        .replace('.', "-");
    let clean: String = s
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect();
    clean.trim_matches('-').to_lowercase()
}

fn format_font_weight_number(weight_str: &str) -> &'static str {
    match weight_str.to_lowercase().as_str() {
        "thin" | "hairline" | "100" => "100",
        "extralight" | "ultralight" | "200" => "200",
        "light" | "300" => "300",
        "normal" | "regular" | "400" => "400",
        "medium" | "500" => "500",
        "semibold" | "demibold" | "600" => "600",
        "bold" | "700" => "700",
        "extrabold" | "ultrabold" | "800" => "800",
        "black" | "heavy" | "900" => "900",
        _ => "400",
    }
}

// ── CSS Variables Generator ──────────────────────────────────────────────────

fn generate_css_tokens(
    styles: &Value,
    vars: &Value,
    collection_filter: Option<&str>,
    mode_filter: Option<&str>,
    prefix: &str,
) -> String {
    let mut root_lines = Vec::new();
    let mut dark_lines = Vec::new();

    // 1. Process Variables (Collections & Modes)
    if let Some(collections) = vars.get("collections").and_then(|v| v.as_array()) {
        for col in collections {
            let col_name = col.get("name").and_then(|v| v.as_str()).unwrap_or("Tokens");
            if let Some(cf) = collection_filter {
                if !col_name.eq_ignore_ascii_case(cf) && !col_name.to_lowercase().contains(&cf.to_lowercase()) {
                    continue;
                }
            }

            let modes = col.get("modes").and_then(|v| v.as_array());
            let default_mode_id = modes.and_then(|m| m.first()).and_then(|m| m.get("id").and_then(|v| v.as_str())).unwrap_or("");
            let dark_mode_id = modes.and_then(|m| {
                m.iter().find(|mode| {
                    let name = mode.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                    name.contains("dark") || name.contains("night")
                }).and_then(|mode| mode.get("id").and_then(|v| v.as_str()))
            });

            if let Some(variables) = col.get("variables").and_then(|v| v.as_array()) {
                root_lines.push(format!("  /* Collection: {} */", col_name));
                for v in variables {
                    let v_name = v.get("name").and_then(|val| val.as_str()).unwrap_or_default();
                    let v_type = v.get("resolvedType").and_then(|val| val.as_str()).unwrap_or_default();
                    let clean_name = sanitize_token_name(v_name);
                    let var_prop = if prefix.is_empty() {
                        format!("--{}", clean_name)
                    } else {
                        format!("--{}{}", prefix.trim_start_matches('-'), clean_name)
                    };

                    let values_obj = v.get("values").and_then(|val| val.as_object());

                    // Default Mode Value
                    let default_val = if let Some(target_mode) = mode_filter {
                        let mode_id = modes.and_then(|m| {
                            m.iter().find(|mode| {
                                let name = mode.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                name.eq_ignore_ascii_case(target_mode)
                            }).and_then(|mode| mode.get("id").and_then(|v| v.as_str()))
                        }).unwrap_or(default_mode_id);

                        values_obj.and_then(|vo| vo.get(mode_id))
                    } else {
                        values_obj.and_then(|vo| vo.get(default_mode_id).or_else(|| vo.values().next()))
                    };

                    if let Some(val) = default_val {
                        let val_str = match v_type {
                            "COLOR" => val.as_str().unwrap_or("#000000").to_string(),
                            "FLOAT" => {
                                if let Some(num) = val.as_f64() {
                                    format!("{}px", num)
                                } else {
                                    val.to_string()
                                }
                            }
                            _ => val.as_str().unwrap_or(&val.to_string()).to_string(),
                        };
                        root_lines.push(format!("  {}: {};", var_prop, val_str));
                    }

                    // Dark Mode Value
                    if mode_filter.is_none() {
                        if let Some(dark_id) = dark_mode_id {
                            if let Some(dark_val) = values_obj.and_then(|vo| vo.get(dark_id)) {
                                let dark_val_str = match v_type {
                                    "COLOR" => dark_val.as_str().unwrap_or("#ffffff").to_string(),
                                    "FLOAT" => {
                                        if let Some(num) = dark_val.as_f64() {
                                            format!("{}px", num)
                                        } else {
                                            dark_val.to_string()
                                        }
                                    }
                                    _ => dark_val.as_str().unwrap_or(&dark_val.to_string()).to_string(),
                                };
                                dark_lines.push(format!("  {}: {};", var_prop, dark_val_str));
                            }
                        }
                    }
                }
                root_lines.push("".to_string());
            }
        }
    }

    // 2. Process Paint Styles (Colors)
    if let Some(paints) = styles.get("paintStyles").and_then(|v| v.as_array()) {
        if !paints.is_empty() {
            root_lines.push("  /* ── Color Styles ── */".to_string());
            for p in paints {
                if let (Some(name), Some(hex)) = (p.get("name").and_then(|v| v.as_str()), p.get("hex").and_then(|v| v.as_str())) {
                    let clean = sanitize_token_name(name);
                    root_lines.push(format!("  --color-{}: {};", clean, hex));
                }
            }
            root_lines.push("".to_string());
        }
    }

    // 3. Process Typography Styles
    if let Some(texts) = styles.get("textStyles").and_then(|v| v.as_array()) {
        if !texts.is_empty() {
            root_lines.push("  /* ── Typography Styles ── */".to_string());
            for t in texts {
                let name = t.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                let clean = sanitize_token_name(name);
                let family = t.get("fontFamily").and_then(|v| v.as_str()).unwrap_or("Inter");
                let weight = format_font_weight_number(t.get("fontWeight").and_then(|v| v.as_str()).unwrap_or("400"));
                let size = t.get("fontSize").and_then(|v| v.as_f64()).unwrap_or(16.0);
                let lh = t.get("lineHeight").and_then(|v| v.as_f64());

                if let Some(lh_val) = lh {
                    root_lines.push(format!("  --font-{}: {} {}px/{}px \"{}\", sans-serif;", clean, weight, size, lh_val, family));
                } else {
                    root_lines.push(format!("  --font-{}: {} {}px \"{}\", sans-serif;", clean, weight, size, family));
                }
            }
            root_lines.push("".to_string());
        }
    }

    // 4. Process Effect Styles (Shadows & Blurs)
    if let Some(effects) = styles.get("effectStyles").and_then(|v| v.as_array()) {
        if !effects.is_empty() {
            root_lines.push("  /* ── Elevation & Shadows ── */".to_string());
            for eff in effects {
                let name = eff.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                let clean = sanitize_token_name(name);
                let eff_type = eff.get("type").and_then(|v| v.as_str()).unwrap_or("DROP_SHADOW");
                let color = eff.get("color").and_then(|v| v.as_str()).unwrap_or("rgba(0, 0, 0, 0.1)");
                let radius = eff.get("radius").and_then(|v| v.as_f64()).unwrap_or(4.0);
                let offset_x = eff.get("offset").and_then(|o| o.get("x")).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let offset_y = eff.get("offset").and_then(|o| o.get("y")).and_then(|v| v.as_f64()).unwrap_or(2.0);
                let spread = eff.get("spread").and_then(|v| v.as_f64()).unwrap_or(0.0);

                if eff_type.contains("SHADOW") {
                    root_lines.push(format!("  --shadow-{}: {}px {}px {}px {}px {};", clean, offset_x, offset_y, radius, spread, color));
                } else if eff_type.contains("BLUR") {
                    root_lines.push(format!("  --blur-{}: blur({}px);", clean, radius));
                }
            }
            root_lines.push("".to_string());
        }
    }

    let mut out = String::new();
    out.push_str("/* ── Generated by Figma MCP (https://github.com/BuiHung1612/figma-mcp) ── */\n\n");
    out.push_str(":root {\n");
    out.push_str(&root_lines.join("\n"));
    out.push_str("\n}\n");

    if !dark_lines.is_empty() {
        out.push_str("\n/* ── Dark Theme Mode ── */\n");
        out.push_str(".dark, [data-theme=\"dark\"] {\n");
        out.push_str(&dark_lines.join("\n"));
        out.push_str("\n}\n");
    }

    out
}

// ── Tailwind Config Generator ────────────────────────────────────────────────

fn generate_tailwind_tokens(
    styles: &Value,
    vars: &Value,
    collection_filter: Option<&str>,
    mode_filter: Option<&str>,
) -> String {
    let mut colors: BTreeMap<String, Value> = BTreeMap::new();
    let mut spacing: BTreeMap<String, Value> = BTreeMap::new();
    let mut border_radius: BTreeMap<String, Value> = BTreeMap::new();
    let mut font_size: BTreeMap<String, Value> = BTreeMap::new();
    let mut box_shadow: BTreeMap<String, Value> = BTreeMap::new();

    // 1. Process Variables
    if let Some(collections) = vars.get("collections").and_then(|v| v.as_array()) {
        for col in collections {
            let col_name = col.get("name").and_then(|v| v.as_str()).unwrap_or("Tokens");
            if let Some(cf) = collection_filter {
                if !col_name.eq_ignore_ascii_case(cf) && !col_name.to_lowercase().contains(&cf.to_lowercase()) {
                    continue;
                }
            }

            let modes = col.get("modes").and_then(|v| v.as_array());
            let default_mode_id = modes.and_then(|m| m.first()).and_then(|m| m.get("id").and_then(|v| v.as_str())).unwrap_or("");

            if let Some(variables) = col.get("variables").and_then(|v| v.as_array()) {
                for v in variables {
                    let v_name = v.get("name").and_then(|val| val.as_str()).unwrap_or_default();
                    let v_type = v.get("resolvedType").and_then(|val| val.as_str()).unwrap_or_default();
                    let clean = sanitize_token_name(v_name);
                    let values_obj = v.get("values").and_then(|val| val.as_object());

                    let default_val = if let Some(target_mode) = mode_filter {
                        let mode_id = modes.and_then(|m| {
                            m.iter().find(|mode| {
                                let name = mode.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                name.eq_ignore_ascii_case(target_mode)
                            }).and_then(|mode| mode.get("id").and_then(|v| v.as_str()))
                        }).unwrap_or(default_mode_id);

                        values_obj.and_then(|vo| vo.get(mode_id))
                    } else {
                        values_obj.and_then(|vo| vo.get(default_mode_id).or_else(|| vo.values().next()))
                    };

                    if let Some(val) = default_val {
                        match v_type {
                            "COLOR" => {
                                if let Some(hex) = val.as_str() {
                                    insert_nested_key(&mut colors, &clean, json!(hex));
                                }
                            }
                            "FLOAT" => {
                                if let Some(num) = val.as_f64() {
                                    if clean.contains("radius") || clean.contains("rounded") {
                                        let k = clean.replace("radius-", "").replace("rounded-", "");
                                        border_radius.insert(k, json!(format!("{}px", num)));
                                    } else if clean.contains("spacing") || clean.contains("gap") || clean.contains("pad") {
                                        let k = clean.replace("spacing-", "").replace("gap-", "").replace("space-", "");
                                        spacing.insert(k, json!(format!("{}px", num)));
                                    } else {
                                        spacing.insert(clean, json!(format!("{}px", num)));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // 2. Process Paint Styles
    if let Some(paints) = styles.get("paintStyles").and_then(|v| v.as_array()) {
        for p in paints {
            if let (Some(name), Some(hex)) = (p.get("name").and_then(|v| v.as_str()), p.get("hex").and_then(|v| v.as_str())) {
                let clean = sanitize_token_name(name);
                insert_nested_key(&mut colors, &clean, json!(hex));
            }
        }
    }

    // 3. Process Typography Styles
    if let Some(texts) = styles.get("textStyles").and_then(|v| v.as_array()) {
        for t in texts {
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let clean = sanitize_token_name(name);
            let size = t.get("fontSize").and_then(|v| v.as_f64()).unwrap_or(16.0);
            let weight = format_font_weight_number(t.get("fontWeight").and_then(|v| v.as_str()).unwrap_or("400"));
            let lh = t.get("lineHeight").and_then(|v| v.as_f64());

            let mut font_props = json!({
                "fontWeight": weight
            });
            if let Some(lh_val) = lh {
                font_props["lineHeight"] = json!(format!("{}px", lh_val));
            }

            font_size.insert(clean, json!([format!("{}px", size), font_props]));
        }
    }

    // 4. Process Effect Styles
    if let Some(effects) = styles.get("effectStyles").and_then(|v| v.as_array()) {
        for eff in effects {
            let name = eff.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let clean = sanitize_token_name(name);
            let color = eff.get("color").and_then(|v| v.as_str()).unwrap_or("rgba(0, 0, 0, 0.1)");
            let radius = eff.get("radius").and_then(|v| v.as_f64()).unwrap_or(4.0);
            let offset_x = eff.get("offset").and_then(|o| o.get("x")).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let offset_y = eff.get("offset").and_then(|o| o.get("y")).and_then(|v| v.as_f64()).unwrap_or(2.0);
            let spread = eff.get("spread").and_then(|v| v.as_f64()).unwrap_or(0.0);

            box_shadow.insert(clean, json!(format!("{}px {}px {}px {}px {}", offset_x, offset_y, radius, spread, color)));
        }
    }

    let mut extend_map = json!({});
    if !colors.is_empty() { extend_map["colors"] = json!(colors); }
    if !spacing.is_empty() { extend_map["spacing"] = json!(spacing); }
    if !border_radius.is_empty() { extend_map["borderRadius"] = json!(border_radius); }
    if !font_size.is_empty() { extend_map["fontSize"] = json!(font_size); }
    if !box_shadow.is_empty() { extend_map["boxShadow"] = json!(box_shadow); }

    let json_pretty = serde_json::to_string_pretty(&json!({
        "theme": {
            "extend": extend_map
        }
    })).unwrap_or_default();

    format!(
        "/** @type {{import('tailwindcss').Config}} */\n// Generated by Figma MCP (https://github.com/BuiHung1612/figma-mcp)\nmodule.exports = {};\n",
        json_pretty
    )
}

fn insert_nested_key(map: &mut BTreeMap<String, Value>, key: &str, val: Value) {
    let parts: Vec<&str> = key.split('-').collect();
    if parts.len() == 2 {
        let parent = parts[0].to_string();
        let child = parts[1].to_string();
        if let Some(Value::Object(ref mut sub)) = map.get_mut(&parent) {
            sub.insert(child, val);
            return;
        } else if !map.contains_key(&parent) {
            let mut sub = serde_json::Map::new();
            sub.insert(child, val);
            map.insert(parent, Value::Object(sub));
            return;
        }
    }
    map.insert(key.to_string(), val);
}

// ── TypeScript Generator ─────────────────────────────────────────────────────

fn generate_ts_tokens(
    styles: &Value,
    vars: &Value,
    collection_filter: Option<&str>,
    mode_filter: Option<&str>,
) -> String {
    let mut colors = BTreeMap::new();
    let mut spacing = BTreeMap::new();
    let mut radius = BTreeMap::new();
    let mut typography = BTreeMap::new();
    let mut shadows = BTreeMap::new();

    if let Some(collections) = vars.get("collections").and_then(|v| v.as_array()) {
        for col in collections {
            let col_name = col.get("name").and_then(|v| v.as_str()).unwrap_or("Tokens");
            if let Some(cf) = collection_filter {
                if !col_name.eq_ignore_ascii_case(cf) && !col_name.to_lowercase().contains(&cf.to_lowercase()) {
                    continue;
                }
            }
            let modes = col.get("modes").and_then(|v| v.as_array());
            let default_mode_id = modes.and_then(|m| m.first()).and_then(|m| m.get("id").and_then(|v| v.as_str())).unwrap_or("");

            if let Some(variables) = col.get("variables").and_then(|v| v.as_array()) {
                for v in variables {
                    let v_name = v.get("name").and_then(|val| val.as_str()).unwrap_or_default();
                    let v_type = v.get("resolvedType").and_then(|val| val.as_str()).unwrap_or_default();
                    let clean = sanitize_token_name(v_name);
                    let values_obj = v.get("values").and_then(|val| val.as_object());

                    let default_val = if let Some(target_mode) = mode_filter {
                        let mode_id = modes.and_then(|m| {
                            m.iter().find(|mode| {
                                let name = mode.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                name.eq_ignore_ascii_case(target_mode)
                            }).and_then(|mode| mode.get("id").and_then(|v| v.as_str()))
                        }).unwrap_or(default_mode_id);

                        values_obj.and_then(|vo| vo.get(mode_id))
                    } else {
                        values_obj.and_then(|vo| vo.get(default_mode_id).or_else(|| vo.values().next()))
                    };

                    if let Some(val) = default_val {
                        match v_type {
                            "COLOR" => {
                                if let Some(hex) = val.as_str() {
                                    colors.insert(clean, json!(hex));
                                }
                            }
                            "FLOAT" => {
                                if let Some(num) = val.as_f64() {
                                    if clean.contains("radius") {
                                        radius.insert(clean, json!(num));
                                    } else {
                                        spacing.insert(clean, json!(num));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    if let Some(paints) = styles.get("paintStyles").and_then(|v| v.as_array()) {
        for p in paints {
            if let (Some(name), Some(hex)) = (p.get("name").and_then(|v| v.as_str()), p.get("hex").and_then(|v| v.as_str())) {
                colors.insert(sanitize_token_name(name), json!(hex));
            }
        }
    }

    if let Some(texts) = styles.get("textStyles").and_then(|v| v.as_array()) {
        for t in texts {
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let clean = sanitize_token_name(name);
            let size = t.get("fontSize").and_then(|v| v.as_f64()).unwrap_or(16.0);
            let weight = format_font_weight_number(t.get("fontWeight").and_then(|v| v.as_str()).unwrap_or("400"));
            let family = t.get("fontFamily").and_then(|v| v.as_str()).unwrap_or("Inter");
            let lh = t.get("lineHeight").and_then(|v| v.as_f64());

            typography.insert(clean, json!({
                "fontFamily": family,
                "fontSize": size,
                "fontWeight": weight,
                "lineHeight": lh
            }));
        }
    }

    if let Some(effects) = styles.get("effectStyles").and_then(|v| v.as_array()) {
        for eff in effects {
            let name = eff.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let clean = sanitize_token_name(name);
            let color = eff.get("color").and_then(|v| v.as_str()).unwrap_or("rgba(0, 0, 0, 0.1)");
            let radius_val = eff.get("radius").and_then(|v| v.as_f64()).unwrap_or(4.0);
            let offset_x = eff.get("offset").and_then(|o| o.get("x")).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let offset_y = eff.get("offset").and_then(|o| o.get("y")).and_then(|v| v.as_f64()).unwrap_or(2.0);
            let spread = eff.get("spread").and_then(|v| v.as_f64()).unwrap_or(0.0);

            shadows.insert(clean, json!(format!("{}px {}px {}px {}px {}", offset_x, offset_y, radius_val, spread, color)));
        }
    }

    let tokens_obj = json!({
        "colors": colors,
        "typography": typography,
        "spacing": spacing,
        "radius": radius,
        "shadows": shadows
    });

    let json_str = serde_json::to_string_pretty(&tokens_obj).unwrap_or_default();

    format!(
        "// Generated by Figma MCP (https://github.com/BuiHung1612/figma-mcp)\n\nexport const tokens = {} as const;\n\nexport type DesignTokens = typeof tokens;\nexport type ColorToken = keyof typeof tokens.colors;\nexport type TypographyToken = keyof typeof tokens.typography;\nexport type SpacingToken = keyof typeof tokens.spacing;\nexport type RadiusToken = keyof typeof tokens.radius;\nexport type ShadowToken = keyof typeof tokens.shadows;\n",
        json_str
    )
}

// ── W3C DTCG Format Generator ────────────────────────────────────────────────

fn generate_w3c_tokens(
    styles: &Value,
    vars: &Value,
    collection_filter: Option<&str>,
    mode_filter: Option<&str>,
) -> String {
    let mut w3c_root = json!({
        "$schema": "https://design-tokens.github.io/community-group/format/v1/schema.json",
        "color": {},
        "dimension": {},
        "typography": {},
        "shadow": {}
    });

    // Color Styles & Variables
    if let Some(collections) = vars.get("collections").and_then(|v| v.as_array()) {
        for col in collections {
            let col_name = col.get("name").and_then(|v| v.as_str()).unwrap_or("Tokens");
            if let Some(cf) = collection_filter {
                if !col_name.eq_ignore_ascii_case(cf) && !col_name.to_lowercase().contains(&cf.to_lowercase()) {
                    continue;
                }
            }

            let modes = col.get("modes").and_then(|v| v.as_array());
            let default_mode_id = modes.and_then(|m| m.first()).and_then(|m| m.get("id").and_then(|v| v.as_str())).unwrap_or("");

            if let Some(variables) = col.get("variables").and_then(|v| v.as_array()) {
                for v in variables {
                    let v_name = v.get("name").and_then(|val| val.as_str()).unwrap_or_default();
                    let v_type = v.get("resolvedType").and_then(|val| val.as_str()).unwrap_or_default();
                    let clean = sanitize_token_name(v_name);
                    let desc = v.get("description").and_then(|val| val.as_str()).unwrap_or("");
                    let values_obj = v.get("values").and_then(|val| val.as_object());

                    let default_val = if let Some(target_mode) = mode_filter {
                        let mode_id = modes.and_then(|m| {
                            m.iter().find(|mode| {
                                let name = mode.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                name.eq_ignore_ascii_case(target_mode)
                            }).and_then(|mode| mode.get("id").and_then(|v| v.as_str()))
                        }).unwrap_or(default_mode_id);

                        values_obj.and_then(|vo| vo.get(mode_id))
                    } else {
                        values_obj.and_then(|vo| vo.get(default_mode_id).or_else(|| vo.values().next()))
                    };

                    if let Some(val) = default_val {
                        match v_type {
                            "COLOR" => {
                                if let Some(hex) = val.as_str() {
                                    w3c_root["color"][&clean] = json!({
                                        "$value": hex,
                                        "$type": "color",
                                        "$description": desc
                                    });
                                }
                            }
                            "FLOAT" => {
                                if let Some(num) = val.as_f64() {
                                    w3c_root["dimension"][&clean] = json!({
                                        "$value": format!("{}px", num),
                                        "$type": "dimension",
                                        "$description": desc
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    if let Some(paints) = styles.get("paintStyles").and_then(|v| v.as_array()) {
        for p in paints {
            if let (Some(name), Some(hex)) = (p.get("name").and_then(|v| v.as_str()), p.get("hex").and_then(|v| v.as_str())) {
                let clean = sanitize_token_name(name);
                let desc = p.get("description").and_then(|v| v.as_str()).unwrap_or("");
                w3c_root["color"][&clean] = json!({
                    "$value": hex,
                    "$type": "color",
                    "$description": desc
                });
            }
        }
    }

    if let Some(texts) = styles.get("textStyles").and_then(|v| v.as_array()) {
        for t in texts {
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let clean = sanitize_token_name(name);
            let size = t.get("fontSize").and_then(|v| v.as_f64()).unwrap_or(16.0);
            let weight = format_font_weight_number(t.get("fontWeight").and_then(|v| v.as_str()).unwrap_or("400"));
            let family = t.get("fontFamily").and_then(|v| v.as_str()).unwrap_or("Inter");
            let lh = t.get("lineHeight").and_then(|v| v.as_f64());

            w3c_root["typography"][&clean] = json!({
                "$value": {
                    "fontFamily": family,
                    "fontSize": format!("{}px", size),
                    "fontWeight": weight,
                    "lineHeight": lh.map(|l| format!("{}px", l)).unwrap_or_else(|| "normal".to_string())
                },
                "$type": "typography"
            });
        }
    }

    serde_json::to_string_pretty(&w3c_root).unwrap_or_default()
}

// ── Raw JSON Generator ───────────────────────────────────────────────────────

fn generate_json_tokens(
    styles: &Value,
    vars: &Value,
    _collection_filter: Option<&str>,
    _mode_filter: Option<&str>,
) -> String {
    let out = json!({
        "variables": vars,
        "styles": styles
    });
    serde_json::to_string_pretty(&out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_generation_css_and_tailwind() {
        let styles = json!({
            "paintStyles": [
                { "name": "Primary/Default", "hex": "#6366f1" },
                { "name": "Neutral/900", "hex": "#0f172a" }
            ],
            "textStyles": [
                { "name": "Heading 1", "fontFamily": "Inter", "fontSize": 32.0, "fontWeight": "Bold", "lineHeight": 40.0 }
            ],
            "effectStyles": [
                { "name": "Elevation/Card", "type": "DROP_SHADOW", "color": "rgba(0, 0, 0, 0.1)", "radius": 4.0, "offset": { "x": 0.0, "y": 2.0 }, "spread": 0.0 }
            ]
        });

        let vars = json!({
            "collections": [
                {
                    "name": "Semantic",
                    "modes": [{ "id": "m1", "name": "Light" }, { "id": "m2", "name": "Dark" }],
                    "variables": [
                        {
                            "name": "bg-canvas",
                            "resolvedType": "COLOR",
                            "values": { "m1": "#ffffff", "m2": "#0d0e12" }
                        },
                        {
                            "name": "radius-card",
                            "resolvedType": "FLOAT",
                            "values": { "m1": 12.0, "m2": 12.0 }
                        }
                    ]
                }
            ]
        });

        // Test CSS
        let css = generate_tokens(&styles, &vars, "css", None, None, None).unwrap();
        assert!(css.contains("--color-primary-default: #6366f1;"));
        assert!(css.contains("--bg-canvas: #ffffff;"));
        assert!(css.contains("--radius-card: 12px;"));
        assert!(css.contains(".dark, [data-theme=\"dark\"]"));
        assert!(css.contains("--bg-canvas: #0d0e12;"));

        // Test Tailwind
        let tw = generate_tokens(&styles, &vars, "tailwind", None, None, None).unwrap();
        assert!(tw.contains("module.exports"));
        assert!(tw.contains("#6366f1"));
        assert!(tw.contains("12px"));

        // Test TypeScript
        let ts = generate_tokens(&styles, &vars, "typescript", None, None, None).unwrap();
        assert!(ts.contains("export const tokens ="));
        assert!(ts.contains("export type DesignTokens"));

        // Test W3C
        let w3c = generate_tokens(&styles, &vars, "w3c", None, None, None).unwrap();
        assert!(w3c.contains("$schema"));
        assert!(w3c.contains("$value"));
    }
}
