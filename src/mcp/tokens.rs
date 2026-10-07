use super::color::{number, Rgba};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

#[derive(Default)]
struct TokenSet {
    colors: BTreeMap<String, Value>,
    spacing: BTreeMap<String, Value>,
    radius: BTreeMap<String, Value>,
    numbers: BTreeMap<String, Value>,
    strings: BTreeMap<String, Value>,
    booleans: BTreeMap<String, Value>,
    typography: BTreeMap<String, Value>,
    backgrounds: BTreeMap<String, Value>,
    shadows: BTreeMap<String, Value>,
    filters: BTreeMap<String, Value>,
    backdrops: BTreeMap<String, Value>,
    css: BTreeMap<String, String>,
    dtcg: Value,
    diagnostics: Vec<String>,
    names: HashSet<String>,
}

fn sanitize_token_name(name: &str) -> String {
    name.trim()
        .replace(['/', ' ', '_', '.'], "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect::<String>()
        .trim_matches('-')
        .to_lowercase()
}

fn font_weight(value: &Value) -> Result<u16, String> {
    if let Some(n) = value.as_u64() {
        return u16::try_from(n).map_err(|_| "Invalid font weight".into());
    }
    let s = value
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .replace([' ', '-'], "");
    match s.as_str() {
        "thin" | "hairline" | "100" => Ok(100),
        "extralight" | "ultralight" | "200" => Ok(200),
        "light" | "300" => Ok(300),
        "normal" | "regular" | "400" => Ok(400),
        "medium" | "500" => Ok(500),
        "semibold" | "demibold" | "demi" | "600" => Ok(600),
        "bold" | "700" => Ok(700),
        "extrabold" | "ultrabold" | "800" => Ok(800),
        "black" | "heavy" | "900" => Ok(900),
        _ => Err(format!("Unknown font weight {value}")),
    }
}

fn dim(v: f64) -> Value {
    json!({"value": v, "unit": "px"})
}
fn dtcg_color(c: Rgba) -> Value {
    json!({"colorSpace": "srgb", "components": &c.0[..3], "alpha": c.0[3]})
}
fn field(v: &Value, key: &str) -> Result<f64, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("Missing numeric {key}"))
}
fn mode_id<'a>(col: &'a Value, mode: Option<&str>) -> Result<&'a str, String> {
    let modes = col["modes"]
        .as_array()
        .ok_or("Collection is missing modes")?;
    if let Some(wanted) = mode {
        return modes
            .iter()
            .find(|m| {
                m["id"].as_str() == Some(wanted)
                    || m["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
            })
            .and_then(|m| m["id"].as_str())
            .ok_or_else(|| format!("Mode '{wanted}' not found in {}", col["name"]));
    }
    col["defaultModeId"]
        .as_str()
        .or_else(|| modes.first().and_then(|m| m["id"].as_str()))
        .ok_or_else(|| "Missing default mode".into())
}

impl TokenSet {
    fn claim(&mut self, name: &str) -> Result<String, String> {
        let clean = sanitize_token_name(name);
        if clean.is_empty() || !self.names.insert(clean.clone()) {
            return Err(format!(
                "Empty or colliding token name after sanitizing '{name}' ({clean})"
            ));
        }
        Ok(clean)
    }
    fn token(&mut self, group: &str, name: &str, kind: &str, value: Value, raw: &Value) {
        if !self.dtcg[group].is_object() {
            self.dtcg[group] = json!({});
        }
        self.dtcg[group][name] = json!({"$type": kind, "$value": value,
            "$extensions": {"io.github.figma-rust-mcp": {"source": raw}}});
    }
    fn export_value(&self) -> Value {
        json!({"colors": self.colors, "spacing": self.spacing, "radius": self.radius,
            "numbers": self.numbers, "strings": self.strings, "booleans": self.booleans,
            "typography": self.typography, "backgrounds": self.backgrounds, "shadows": self.shadows,
            "filters": self.filters, "backdropFilters": self.backdrops})
    }
}

// Figma's transform maps normalized node coordinates to gradient coordinates.
// Project the first row into CSS's gradient line; include translation and node
// aspect ratio instead of converting only atan2(matrix) into a guessed angle.
pub fn paint_css(p: &Value, width: f64, height: f64) -> Result<String, String> {
    if p["blendMode"].as_str().is_some_and(|m| m != "NORMAL") {
        return Err("Paint blend mode requires SVG compositing".into());
    }
    let opacity = p["opacity"].as_f64().unwrap_or(1.0);
    match p["type"].as_str() {
        Some("SOLID") => {
            let color = Rgba::parse(p.get("rgba").unwrap_or(&p["color"]))?;
            let opacity = if p["alphaIncluded"] == true && p.get("rgba").is_none() {
                1.0
            } else {
                opacity
            };
            Ok(color.with_opacity(opacity)?.css())
        }
        Some("GRADIENT_LINEAR") => {
            let row = p["gradientTransform"][0]
                .as_array()
                .ok_or("Missing gradientTransform")?;
            if row.len() != 3 || width <= 0.0 || height <= 0.0 {
                return Err("Invalid gradient geometry".into());
            }
            let a = row[0].as_f64().ok_or("Invalid gradient transform")?;
            let b = row[1].as_f64().ok_or("Invalid gradient transform")?;
            let c = row[2].as_f64().ok_or("Invalid gradient transform")?;
            let qx = a / width;
            let qy = b / height;
            let len = qx.hypot(qy);
            if len <= 1e-12 {
                return Err("Singular gradient transform".into());
            }
            let nx = qx / len;
            let ny = qy / len;
            let span = nx.abs() * width + ny.abs() * height;
            let min = c + a.min(0.0) + b.min(0.0);
            let angle = nx.atan2(-ny).to_degrees().rem_euclid(360.0);
            let stops = p["gradientStops"]
                .as_array()
                .filter(|s| !s.is_empty())
                .ok_or("Missing gradient stops")?;
            let stops = stops
                .iter()
                .map(|s| {
                    let color =
                        Rgba::parse(s.get("rgba").unwrap_or(&s["color"]))?.with_opacity(opacity)?;
                    Ok(format!(
                        "{} {}%",
                        color.css(),
                        number((field(s, "position")? - min) / (len * span) * 100.0)
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(format!(
                "linear-gradient({}deg, {})",
                number(angle),
                stops.join(", ")
            ))
        }
        other => Err(format!(
            "Paint {} requires SVG; original data retained",
            other.unwrap_or("UNKNOWN")
        )),
    }
}

pub fn background_css(
    paints: &[Value],
    width: f64,
    height: f64,
) -> Result<Option<(&'static str, String)>, String> {
    let visible = paints
        .iter()
        .filter(|p| p["visible"] != false)
        .collect::<Vec<_>>();
    if visible.is_empty() {
        return Ok(None);
    }
    if visible.len() == 1 && visible[0]["type"] == "SOLID" {
        return Ok(Some((
            "background-color",
            paint_css(visible[0], width, height)?,
        )));
    }
    let layers = visible
        .iter()
        .map(|p| {
            let css = paint_css(p, width, height)?;
            Ok(if p["type"] == "SOLID" {
                format!("linear-gradient({css}, {css})")
            } else {
                css
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Some(("background", layers.join(", "))))
}

pub fn effect_css(effects: &[Value]) -> Result<BTreeMap<String, String>, String> {
    let mut parts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in effects.iter().filter(|e| e["visible"] != false) {
        if e["blendMode"].as_str().is_some_and(|m| m != "NORMAL") {
            return Err("Effect blend mode requires SVG".into());
        }
        let (property, value) = match e["type"].as_str() {
            Some("DROP_SHADOW" | "INNER_SHADOW") => {
                let color = Rgba::parse(&e["color"])?;
                let x = e["offset"]["x"]
                    .as_f64()
                    .or_else(|| e["offsetX"].as_f64())
                    .ok_or("Missing shadow offsetX")?;
                let y = e["offset"]["y"]
                    .as_f64()
                    .or_else(|| e["offsetY"].as_f64())
                    .ok_or("Missing shadow offsetY")?;
                (
                    "box-shadow",
                    format!(
                        "{}{}px {}px {}px {}px {}",
                        if e["type"] == "INNER_SHADOW" {
                            "inset "
                        } else {
                            ""
                        },
                        number(x),
                        number(y),
                        number(field(e, "radius")?),
                        number(e["spread"].as_f64().unwrap_or(0.0)),
                        color.css()
                    ),
                )
            }
            Some("LAYER_BLUR" | "BACKGROUND_BLUR") => {
                if e["blurType"].as_str().is_some_and(|t| t != "NORMAL") {
                    return Err("Progressive blur requires SVG".into());
                }
                (
                    if e["type"] == "LAYER_BLUR" {
                        "filter"
                    } else {
                        "backdrop-filter"
                    },
                    format!("blur({}px)", number(field(e, "radius")?)),
                )
            }
            _ => return Err(format!("Unsupported effect {}", e["type"])),
        };
        parts.entry(property.to_string()).or_default().push(value);
    }
    Ok(parts
        .into_iter()
        .map(|(key, values)| {
            let separator = if key == "box-shadow" { ", " } else { " " };
            (key, values.join(separator))
        })
        .collect())
}

fn build_tokens(
    styles: &Value,
    vars: &Value,
    collection: Option<&str>,
    mode: Option<&str>,
) -> Result<TokenSet, String> {
    for data in [styles, vars] {
        if let Some(error) = data["error"].as_str() {
            return Err(error.into());
        }
    }
    let mut set = TokenSet {
        dtcg: json!({"$extensions": {"io.github.figma-rust-mcp": {"formatVersion": "2025.10"}}}),
        ..Default::default()
    };
    if let Some(diags) = vars["diagnostics"].as_array() {
        set.diagnostics.extend(diags.iter().map(Value::to_string));
    }
    if let Some(cols) = vars["collections"].as_array() {
        for col in cols {
            let name = col["name"].as_str().unwrap_or("");
            if collection.is_some_and(|f| !name.to_lowercase().contains(&f.to_lowercase())) {
                continue;
            }
            let selected = mode_id(col, mode)?;
            let variables = col["variables"]
                .as_array()
                .ok_or("Collection is missing variables")?;
            for var in variables {
                let name = var["name"].as_str().ok_or("Variable is missing name")?;
                let clean = set.claim(name)?;
                let value = var
                    .get("values")
                    .or_else(|| var.get("valuesByMode"))
                    .and_then(|v| v.get(selected))
                    .ok_or_else(|| format!("Missing mode value for {name}"))?;
                let value = if value["type"] == "VARIABLE_ALIAS" {
                    value
                        .get("resolvedValue")
                        .filter(|v| !v.is_null())
                        .ok_or_else(|| format!("Unresolved alias for {name} in mode {selected}"))?
                } else {
                    value
                };
                match var["resolvedType"].as_str() {
                    Some("COLOR") => {
                        let color = Rgba::parse(value)?;
                        set.colors.insert(clean.clone(), json!(color.css()));
                        set.css.insert(clean.clone(), color.css());
                        set.token("color", &clean, "color", dtcg_color(color), var);
                    }
                    Some("FLOAT") => {
                        let n = value
                            .as_f64()
                            .ok_or_else(|| format!("Invalid FLOAT {name}"))?;
                        let scopes = var["scopes"].as_array();
                        let unitless = scopes.is_some_and(|s| {
                            s.iter().any(|s| s == "OPACITY" || s == "FONT_WEIGHT")
                        }) || clean.contains("opacity")
                            || clean.contains("weight");
                        let dimensional = scopes.is_some_and(|s| {
                            s.iter().any(|scope| {
                                matches!(
                                    scope.as_str(),
                                    Some(
                                        "WIDTH_HEIGHT"
                                            | "GAP"
                                            | "CORNER_RADIUS"
                                            | "STROKE_FLOAT"
                                            | "FONT_SIZE"
                                            | "LINE_HEIGHT"
                                            | "LETTER_SPACING"
                                    )
                                )
                            })
                        }) || [
                            "spacing", "space-", "gap", "padding", "radius", "rounded", "width",
                            "height", "size", "border", "stroke",
                        ]
                        .iter()
                        .any(|hint| clean.contains(hint));
                        if unitless || !dimensional {
                            if !unitless && !dimensional {
                                set.diagnostics.push(format!("{name}: FLOAT has no dimension scope; preserved as unitless number"));
                            }
                            set.numbers.insert(clean.clone(), json!(n));
                            set.css.insert(clean.clone(), number(n));
                            set.token("number", &clean, "number", json!(n), var);
                        } else {
                            if clean.contains("radius") || clean.contains("rounded") {
                                set.radius.insert(clean.clone(), json!(n));
                            } else {
                                set.spacing.insert(clean.clone(), json!(n));
                            }
                            set.css.insert(clean.clone(), format!("{}px", number(n)));
                            set.token("dimension", &clean, "dimension", dim(n), var);
                        }
                    }
                    Some("STRING") => {
                        let s = value.as_str().ok_or("Invalid STRING token")?;
                        set.strings.insert(clean.clone(), json!(s));
                        set.css.insert(clean, serde_json::to_string(s).unwrap());
                    }
                    Some("BOOLEAN") => {
                        let b = value.as_bool().ok_or("Invalid BOOLEAN token")?;
                        set.booleans.insert(clean.clone(), json!(b));
                        set.css.insert(clean, b.to_string());
                    }
                    _ => return Err(format!("Unknown variable type for {name}")),
                }
            }
        }
    } else if vars.get("variables").is_some() {
        return Err(
            "Incomplete legacy variable cache: collections/modes required; refresh index".into(),
        );
    }
    for p in styles["paintStyles"].as_array().into_iter().flatten() {
        let name = p["name"].as_str().ok_or("Paint style missing name")?;
        let clean = set.claim(name)?;
        let legacy;
        let paints = if let Some(paints) = p["paints"].as_array() {
            paints
        } else if p["hex"].is_string() {
            legacy = vec![json!({"type": "SOLID", "color": p["hex"]})];
            &legacy
        } else {
            set.diagnostics
                .push(format!("{name}: missing paint data; refresh plugin"));
            continue;
        };
        let visible = paints
            .iter()
            .filter(|p| p["visible"] != false)
            .collect::<Vec<_>>();
        if visible.is_empty() {
            continue;
        }
        if visible.len() == 1 && visible[0]["type"] == "SOLID" {
            let css = paint_css(visible[0], 1.0, 1.0)?;
            set.colors.insert(clean.clone(), json!(css));
            set.css.insert(format!("color-{clean}"), css.clone());
            set.token(
                "color",
                &clean,
                "color",
                dtcg_color(Rgba::parse(&json!(css))?),
                p,
            );
        } else {
            // Styles have no consuming-node dimensions. Diagonal affine gradients
            // cannot be converted once for arbitrary aspect ratios; keep raw data.
            let mut layers = Vec::new();
            let mut convertible = true;
            for paint in &visible {
                if paint["type"] == "GRADIENT_LINEAR" {
                    let row = &paint["gradientTransform"][0];
                    if row[0].as_f64().unwrap_or(0.0).abs() > 1e-12
                        && row[1].as_f64().unwrap_or(0.0).abs() > 1e-12
                    {
                        set.diagnostics.push(format!("{name}: diagonal gradient needs target width/height; use get_css on the consuming node"));
                        convertible = false;
                    }
                }
                match paint_css(paint, 1.0, 1.0) {
                    Ok(css) => layers.push(if paint["type"] == "SOLID" {
                        format!("linear-gradient({css}, {css})")
                    } else {
                        css
                    }),
                    Err(e) => {
                        set.diagnostics.push(format!("{name}: {e}"));
                        convertible = false;
                    }
                }
            }
            if convertible {
                let css = layers.join(", ");
                set.backgrounds.insert(clean.clone(), json!(css));
                set.css.insert(format!("background-{clean}"), css);
            }
            if visible.len() == 1
                && visible[0]["type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("GRADIENT_"))
            {
                let paint = visible[0];
                let stops = paint["gradientStops"].as_array().ok_or("Missing gradient stops")?.iter().map(|stop| {
                    let color = Rgba::parse(stop.get("rgba").unwrap_or(&stop["color"]))?.with_opacity(paint["opacity"].as_f64().unwrap_or(1.0))?;
                    Ok(json!({"color": dtcg_color(color), "position": field(stop, "position")?}))
                }).collect::<Result<Vec<_>, String>>()?;
                set.token("gradient", &clean, "gradient", json!(stops), p);
            }
        }
    }
    for style in styles["effectStyles"].as_array().into_iter().flatten() {
        let name = style["name"].as_str().ok_or("Effect style missing name")?;
        let clean = set.claim(name)?;
        let legacy;
        let effects = if let Some(effects) = style["effects"].as_array() {
            effects
        } else if style["type"] == "DROP_SHADOW" || style["type"] == "INNER_SHADOW" {
            legacy = vec![style.clone()];
            &legacy
        } else {
            set.diagnostics
                .push(format!("{name}: missing effect stack; refresh plugin"));
            continue;
        };
        let mut shadows = Vec::new();
        let mut dtcg = Vec::new();
        let mut filters = Vec::new();
        let mut backdrops = Vec::new();
        for e in effects.iter().filter(|e| e["visible"] != false) {
            if e["blendMode"].as_str().is_some_and(|m| m != "NORMAL") {
                set.diagnostics
                    .push(format!("{name}: effect blend mode requires SVG"));
                continue;
            }
            match e["type"].as_str() {
                Some("DROP_SHADOW" | "INNER_SHADOW") => {
                    let color = Rgba::parse(&e["color"])?;
                    let x = field(&e["offset"], "x")?;
                    let y = field(&e["offset"], "y")?;
                    let blur = field(e, "radius")?;
                    let spread = e["spread"].as_f64().unwrap_or(0.0);
                    let inset = e["type"] == "INNER_SHADOW";
                    shadows.push(format!(
                        "{}{}px {}px {}px {}px {}",
                        if inset { "inset " } else { "" },
                        number(x),
                        number(y),
                        number(blur),
                        number(spread),
                        color.css()
                    ));
                    dtcg.push(json!({"color": dtcg_color(color), "offsetX": dim(x), "offsetY": dim(y), "blur": dim(blur), "spread": dim(spread), "inset": inset}));
                }
                Some("LAYER_BLUR" | "BACKGROUND_BLUR") => {
                    if e["blurType"].as_str().is_some_and(|t| t != "NORMAL") {
                        set.diagnostics
                            .push(format!("{name}: progressive blur requires SVG"));
                        continue;
                    }
                    let css = format!("blur({}px)", number(field(e, "radius")?));
                    if e["type"] == "LAYER_BLUR" {
                        filters.push(css);
                    } else {
                        backdrops.push(css);
                    }
                }
                _ => set.diagnostics.push(format!(
                    "{name}: unsupported effect {}; raw data retained",
                    e["type"]
                )),
            }
        }
        if !shadows.is_empty() {
            let css = shadows.join(", ");
            set.shadows.insert(clean.clone(), json!(css.clone()));
            set.css.insert(format!("shadow-{clean}"), css);
            set.token("shadow", &clean, "shadow", json!(dtcg), style);
        }
        if !filters.is_empty() {
            let css = filters.join(" ");
            set.filters.insert(clean.clone(), json!(css.clone()));
            set.css.insert(format!("filter-{clean}"), css);
        }
        if !backdrops.is_empty() {
            let css = backdrops.join(" ");
            set.backdrops.insert(clean.clone(), json!(css.clone()));
            set.css.insert(format!("backdrop-filter-{clean}"), css);
        }
    }
    for t in styles["textStyles"].as_array().into_iter().flatten() {
        let name = t["name"].as_str().ok_or("Text style missing name")?;
        let clean = set.claim(name)?;
        let size = field(t, "fontSize")?;
        let family = t["fontFamily"].as_str().ok_or("Missing font family")?;
        let weight = font_weight(&t["fontWeight"])?;
        let lh = t["lineHeight"].as_f64();
        let unit = t["lineHeightUnit"].as_str().unwrap_or("PIXELS");
        let line_height = match (unit, lh) {
            ("AUTO", _) | (_, None) => "normal".to_string(),
            ("PERCENT", Some(v)) => number(v / 100.0),
            ("PIXELS", Some(v)) => format!("{}px", number(v)),
            _ => return Err(format!("Invalid line-height unit for {name}")),
        };
        let spacing = t["letterSpacing"].as_f64().unwrap_or(0.0);
        let spacing_css = if t["letterSpacingUnit"] == "PERCENT" {
            format!("{}em", number(spacing / 100.0))
        } else {
            format!("{}px", number(spacing))
        };
        let family_quoted = serde_json::to_string(family).unwrap();
        set.css.insert(
            format!("font-{clean}"),
            format!(
                "{weight} {}px/{line_height} {family_quoted}, sans-serif",
                number(size)
            ),
        );
        set.css
            .insert(format!("letter-spacing-{clean}"), spacing_css.clone());
        set.typography.insert(clean.clone(), json!({"fontFamily": family, "fontSize": size, "fontWeight": weight, "lineHeight": line_height, "letterSpacing": spacing_css}));
        // AUTO has no exact DTCG numeric line-height; retain raw style, don't invent one.
        if let Some(lh) = lh.filter(|_| unit != "AUTO") {
            if size > 0.0 {
                let ratio = if unit == "PERCENT" {
                    lh / 100.0
                } else {
                    lh / size
                };
                set.token("typography", &clean, "typography", json!({"fontFamily": family, "fontSize": dim(size), "fontWeight": weight,
                    "lineHeight": ratio, "letterSpacing": dim(if t["letterSpacingUnit"] == "PERCENT" { spacing * size / 100.0 } else { spacing })}), t);
            }
        } else {
            set.diagnostics.push(format!(
                "{name}: automatic line height retained in raw style; no exact DTCG numeric value"
            ));
        }
    }
    Ok(set)
}

pub fn generate_tokens(
    styles: &Value,
    vars: &Value,
    format: &str,
    collection: Option<&str>,
    mode: Option<&str>,
    prefix: Option<&str>,
) -> Result<String, String> {
    let set = build_tokens(styles, vars, collection, mode)?;
    let diagnostics = set
        .diagnostics
        .iter()
        .map(|d| format!("// {}\n", d.replace(['\n', '\r'], " ")))
        .collect::<String>();
    match format.to_lowercase().as_str() {
        "css" => {
            let prefix = prefix.unwrap_or("").trim_start_matches('-');
            let declarations = |s: &TokenSet| s.css.iter().map(|(name, value)| format!("  --{prefix}{name}: {value};")).collect::<Vec<_>>().join("\n");
            let warnings = set.diagnostics.iter().map(|d| format!("/* {} */\n", d.replace("*/", "* /"))).collect::<String>();
            let mut out = format!("/* Generated by Figma Rust MCP — token schema v2 */\n{warnings}:root {{\n{}\n}}\n", declarations(&set));
            if mode.is_none() {
                let cols = vars["collections"].as_array().cloned().unwrap_or_default();
                let mut modes = BTreeMap::<String, Value>::new();
                for col in &cols {
                    if collection.is_some_and(|f| !col["name"].as_str().unwrap_or("").to_lowercase().contains(&f.to_lowercase())) { continue; }
                    let default = mode_id(col, None)?;
                    for m in col["modes"].as_array().into_iter().flatten().filter(|m| m["id"].as_str() != Some(default)) {
                        let name = m["name"].as_str().unwrap_or("");
                        let entry = modes.entry(name.to_string()).or_insert_with(|| json!({"collections": []}));
                        let mut c = col.clone(); c["defaultModeId"] = m["id"].clone();
                        entry["collections"].as_array_mut().unwrap().push(c);
                    }
                }
                for (name, mode_vars) in modes {
                    let s = build_tokens(&json!({}), &mode_vars, collection, None)?;
                    let clean = sanitize_token_name(&name);
                    let selector = if clean.contains("dark") || clean.contains("night") { format!(".dark, [data-theme=\"{clean}\"]") } else { format!("[data-theme=\"{clean}\"]") };
                    out.push_str(&format!("\n{selector} {{\n{}\n}}\n", declarations(&s)));
                }
            }
            Ok(out)
        }
        "tailwind" => {
            let mut extend = json!({"colors": set.colors, "backgroundImage": set.backgrounds, "boxShadow": set.shadows, "spacing": {}, "borderRadius": {}, "fontSize": {}});
            for (name, value) in &set.spacing { extend["spacing"][name] = json!(format!("{}px", number(value.as_f64().unwrap()))); }
            for (name, value) in &set.radius { extend["borderRadius"][name] = json!(format!("{}px", number(value.as_f64().unwrap()))); }
            for (name, t) in &set.typography { extend["fontSize"][name] = json!([format!("{}px", number(t["fontSize"].as_f64().unwrap())), {"fontWeight": t["fontWeight"], "lineHeight": t["lineHeight"], "letterSpacing": t["letterSpacing"]}]); }
            Ok(format!("// Generated by Figma Rust MCP\n{diagnostics}module.exports = {};\n", serde_json::to_string_pretty(&json!({"theme": {"extend": extend}, "figmaTokens": set.export_value()})).unwrap()))
        }
        "typescript" | "ts" => Ok(format!("// Generated by Figma Rust MCP\n{diagnostics}export const tokens = {} as const;\nexport type DesignTokens = typeof tokens;\nexport type ColorToken = keyof typeof tokens.colors;\nexport type TypographyToken = keyof typeof tokens.typography;\nexport type SpacingToken = keyof typeof tokens.spacing;\nexport type RadiusToken = keyof typeof tokens.radius;\nexport type ShadowToken = keyof typeof tokens.shadows;\nexport const figmaSource = {} as const;\n",
            serde_json::to_string_pretty(&set.export_value()).unwrap(), serde_json::to_string(&json!({"styles": styles, "variables": vars, "diagnostics": set.diagnostics})).unwrap())),
        "w3c" => {
            let mut dtcg = set.dtcg;
            dtcg["$extensions"]["io.github.figma-rust-mcp"]["diagnostics"] = json!(set.diagnostics);
            dtcg["$extensions"]["io.github.figma-rust-mcp"]["source"] = json!({"styles": styles, "variables": vars});
            Ok(serde_json::to_string_pretty(&dtcg).unwrap())
        }
        "json" => Ok(serde_json::to_string_pretty(&json!({"schemaVersion": 2, "tokens": set.export_value(), "diagnostics": set.diagnostics, "variables": vars, "styles": styles})).unwrap()),
        _ => Err(format!("Unsupported format: '{format}'. Available: css, tailwind, typescript, w3c, json")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_payload_exports_all_formats_without_alpha_loss() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/tokens-v2.json")).unwrap();
        let (styles, vars) = (&fixture["styles"], &fixture["variables"]);
        for format in ["css", "tailwind", "typescript", "w3c", "json"] {
            let out = generate_tokens(styles, vars, format, None, None, Some("app-")).unwrap();
            assert!(!out.contains("NaN"));
            if format != "w3c" {
                assert!(out.contains("rgba(255, 0, 0, 0.5)"), "{format}");
                assert!(
                    out.contains("inset 0px 0px 0px 1px rgba(255, 0, 0, 0)"),
                    "{format}"
                );
            }
        }
        let css = generate_tokens(styles, vars, "css", None, None, Some("app-")).unwrap();
        assert!(css.contains("--app-bg-canvas: rgba(255, 255, 255, 0.5);"));
        assert!(css.contains("--app-opacity-disabled: 0;"));
        assert!(css.contains("--app-enabled: false;"));
        assert!(css.contains("--app-color-brand-overlay: rgba(255, 0, 0, 0.5);"));
        assert!(css.contains("--app-background-brand-fade: linear-gradient(90deg"));
        assert!(css.contains("rgba(255, 0, 0, 0.25) 0%"));
        assert!(css.contains("--app-backdrop-filter-elevation-card: blur(12px);"));
        assert!(css.contains("600 16px/1.5 \"Inter\""));
        assert!(css.contains("--app-letter-spacing-text-body: 0.02em;"));
        assert!(css.contains("GRADIENT_DIAMOND requires SVG"));
        assert!(!css.contains("--app-background-brand-diamond:"));
        let dtcg: Value =
            serde_json::from_str(&generate_tokens(styles, vars, "w3c", None, None, None).unwrap())
                .unwrap();
        assert_eq!(
            dtcg["shadow"]["elevation-card"]["$value"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(dtcg["shadow"]["elevation-card"]["$value"][1]["inset"], true);
        assert_eq!(dtcg["color"]["brand-overlay"]["$value"]["alpha"], 0.5);
        assert_eq!(
            dtcg["gradient"]["brand-fade"]["$value"][0]["color"]["alpha"],
            0.25
        );
    }

    #[test]
    fn missing_values_and_collisions_fail_instead_of_fabricating_tokens() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/tokens-v2.json")).unwrap();
        let mut vars = fixture["variables"].clone();
        vars["collections"][0]["variables"][3]["values"]["z-light"]["resolvedValue"] = Value::Null;
        assert!(generate_tokens(&json!({}), &vars, "css", None, None, None)
            .unwrap_err()
            .contains("Unresolved alias"));
        assert!(generate_tokens(
            &json!({}),
            &fixture["variables"],
            "css",
            None,
            Some("Missing"),
            None
        )
        .is_err());
        assert!(generate_tokens(
            &json!({}),
            &json!({"variables": []}),
            "css",
            None,
            None,
            None
        )
        .is_err());
        let styles = json!({"paintStyles": [{"name": "A/B", "hex": "#fff"}, {"name": "a-b", "hex": "#000"}]});
        assert!(
            generate_tokens(&styles, &json!({}), "css", None, None, None)
                .unwrap_err()
                .contains("colliding")
        );
        let style = json!({"effectStyles": [{"name": "Card", "type": "EFFECT", "effects": 2}]});
        let css = generate_tokens(&style, &json!({}), "css", None, None, None).unwrap();
        assert!(css.contains("missing effect stack"));
        assert!(!css.contains("--shadow-card:"));
    }

    #[test]
    fn gradient_geometry_and_multilayer_order_are_preserved() {
        let p = json!({"type": "GRADIENT_LINEAR", "gradientTransform": [[1,1,0],[0,1,0]], "gradientStops": [
            {"position":0,"color":"#ff000080"},{"position":1,"color":"#00f"}]});
        let css = paint_css(&p, 200.0, 100.0).unwrap();
        assert!(css.contains("153.434949deg"));
        assert!(css.contains("#0000ff 50%"));
        let layers = vec![
            json!({"type":"SOLID","color":"#f008"}),
            json!({"type":"SOLID","color":"#00f"}),
        ];
        let (_, css) = background_css(&layers, 100.0, 100.0).unwrap().unwrap();
        assert!(css.starts_with("linear-gradient(rgba(255, 0, 0"));
        let set = build_tokens(
            &json!({"paintStyles": [{"name":"Diagonal","paints":[p]}]}),
            &json!({}),
            None,
            None,
        )
        .unwrap();
        assert!(set.backgrounds.is_empty());
        assert!(set
            .diagnostics
            .iter()
            .any(|d| d.contains("target width/height")));
    }

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
        assert!(w3c.contains("$extensions"));
        assert!(w3c.contains("$value"));
    }
}
