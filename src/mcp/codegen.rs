use crate::mcp::color::{number, Rgba};
use crate::mcp::verify::{box4, px, solid_fill};
use serde_json::{json, Value};

pub fn generate_code_from_context(
    context: &Value,
    framework: &str,
    component_name: Option<&str>,
) -> Result<String, String> {
    // get_design_context wraps the tree as {nodeId, name, context, summary}.
    let context = &normalize(context.get("context").unwrap_or(context), None);
    let name = component_name
        .map(sanitize_component_name)
        .unwrap_or_else(|| {
            let n = context.get("name").and_then(|v| v.as_str()).unwrap_or("Component");
            sanitize_component_name(n)
        });

    let fmt = framework.to_lowercase();
    match fmt.as_str() {
        "react-tailwind" | "react" | "next" | "tailwind" => Ok(generate_react_tailwind(context, &name)),
        "react-shadcn" | "shadcn" | "shadcn-ui" => Ok(generate_shadcn_react(context, &name)),
        "react-native" | "rn" => Ok(generate_react_native(context, &name)),
        "vue" | "vue-tailwind" => Ok(generate_vue_tailwind(context, &name)),
        "html" | "html-tailwind" => Ok(generate_html_tailwind(context)),
        "swiftui" => Ok(generate_swiftui(context, &name)),
        "clean-spec" | "clean" | "spec" | "yaml-spec" => Ok(generate_clean_spec(context)),
        _ => Err(format!(
            "Unsupported framework: '{}'. Available: 'react-tailwind', 'react-shadcn', 'react-native', 'vue-tailwind', 'html', 'swiftui', 'clean-spec'",
            framework
        )),
    }
}

/// Fold the plugin's nested shapes into the flat fields the renderers read and
/// record each child's parent flow, so FILL sizing maps to flex-1 or self-stretch.
fn normalize(node: &Value, parent_flow: Option<&str>) -> Value {
    let Some(obj) = node.as_object() else { return node.clone() };
    let flow = match node.pointer("/layout/display").and_then(Value::as_str) {
        Some("grid") => "grid",
        Some(_) if node.pointer("/layout/flexDirection").and_then(Value::as_str) == Some("row") => "row",
        Some(_) => "column",
        None => "none",
    };
    let mut out = serde_json::Map::new();
    for (k, v) in obj {
        let v = if k == "children" {
            Value::Array(v.as_array().into_iter().flatten().map(|c| normalize(c, Some(flow))).collect())
        } else {
            v.clone()
        };
        out.insert(k.clone(), v);
    }
    if !out.contains_key("characters") {
        if let Some(t) = node.pointer("/text/content").and_then(Value::as_str) {
            out.insert("characters".into(), t.into());
        }
    }
    if let Some(p) = parent_flow {
        out.insert("_parentFlow".into(), p.into());
    }
    Value::Object(out)
}

fn sanitize_component_name(name: &str) -> String {
    let words: String = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map(|f| f.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        })
        .collect();
    match words.chars().next() {
        None => "Component".to_string(),
        // Identifiers cannot start with a digit ("404 Page").
        Some(c) if c.is_ascii_digit() => format!("Figma{words}"),
        Some(_) => words,
    }
}

/// Lowercase words of a layer name, splitting camelCase: "IconButton" → [icon, button].
fn name_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if (!c.is_alphanumeric() || (c.is_uppercase() && prev_lower)) && !cur.is_empty() {
            words.push(std::mem::take(&mut cur).to_lowercase());
        }
        if c.is_alphanumeric() { cur.push(c); }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
    }
    if !cur.is_empty() { words.push(cur.to_lowercase()); }
    words
}

fn name_has(node: &Value, any: &[&str]) -> bool {
    name_words(node["name"].as_str().unwrap_or("")).iter().any(|w| any.contains(&w.as_str()))
}

#[derive(Clone, Copy, PartialEq)]
enum Dialect { Jsx, Html, Vue }

fn escape_text(s: &str, d: Dialect) -> String {
    match d {
        // A JS string literal keeps braces, angle brackets, quotes and whitespace exact.
        Dialect::Jsx if s.contains(['{', '}', '<', '>', '&', '\n', '"', '\'']) || s.starts_with(' ') || s.ends_with(' ') || s.contains("  ") =>
            format!("{{{}}}", serde_json::to_string(s).unwrap()),
        Dialect::Jsx => s.to_string(),
        Dialect::Html | Dialect::Vue => s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('{', "&#123;"),
    }
}

fn node_size(node: &Value) -> (f64, f64) {
    let get = |k: &str| node[k].as_f64().or_else(|| node["size"][k].as_f64()).unwrap_or(0.0);
    (get("width"), get("height"))
}

// ── Tailwind Helpers ─────────────────────────────────────────────────────────

fn px_to_tailwind_spacing(px: f64) -> String {
    const SCALE: [(f64, &str); 22] = [
        (0.0, "0"), (1.0, "px"), (2.0, "0.5"), (4.0, "1"), (6.0, "1.5"), (8.0, "2"), (10.0, "2.5"), (12.0, "3"),
        (14.0, "3.5"), (16.0, "4"), (20.0, "5"), (24.0, "6"), (28.0, "7"), (32.0, "8"), (36.0, "9"), (40.0, "10"),
        (44.0, "11"), (48.0, "12"), (56.0, "14"), (64.0, "16"), (80.0, "20"), (96.0, "24"),
    ];
    SCALE.iter().find(|(v, _)| (px - v).abs() < 0.01).map(|(_, c)| c.to_string())
        .unwrap_or_else(|| format!("[{}px]", number(px)))
}

/// Exact Tailwind radius: scale names only on exact matches, otherwise arbitrary
/// values. A radius of at least half the short side renders as a pill.
fn radius_class(r: [f64; 4], width: f64, height: f64) -> Option<String> {
    if r.iter().all(|v| *v <= 0.0) { return None; }
    if r.iter().any(|v| (v - r[0]).abs() > 0.01) {
        return Some(format!("rounded-[{}]", r.map(|v| format!("{}px", number(v))).join("_")));
    }
    let v = r[0];
    let short = width.min(height);
    if v >= 9999.0 || (short > 0.0 && v >= short / 2.0) { return Some("rounded-full".into()); }
    const SCALE: [(f64, &str); 7] = [(2.0, "rounded-sm"), (4.0, "rounded"), (6.0, "rounded-md"), (8.0, "rounded-lg"), (12.0, "rounded-xl"), (16.0, "rounded-2xl"), (24.0, "rounded-3xl")];
    Some(SCALE.iter().find(|(s, _)| (v - s).abs() < 0.01).map(|(_, c)| c.to_string()).unwrap_or_else(|| format!("rounded-[{}px]", number(v))))
}

fn hex_to_tailwind_color(hex_or_var: &str, prefix: &str) -> String {
    let s = hex_or_var.trim();
    if s.starts_with("var(") {
        // The `color:` hint stops Tailwind reading text-[var(..)] as a font size.
        return format!("{}-[color:{}]", prefix, s);
    }
    let canonical = Rgba::parse(&json!(s)).map(|c| c.css()).unwrap_or_else(|_| s.to_string());
    let lower = canonical.to_lowercase();
    if lower.starts_with("rgba(") {
        let clean = lower.replace(' ', "");
        for (rgb, name) in [("rgba(0,0,0,", "black"), ("rgba(255,255,255,", "white")] {
            if let Some(alpha) = clean.strip_prefix(rgb).and_then(|a| a.strip_suffix(')')).and_then(|a| a.parse::<f64>().ok()) {
                return format!("{}-{}/{}", prefix, name, number(alpha * 100.0));
            }
        }
        return format!("{}-[{}]", prefix, clean);
    }
    match lower.as_str() {
        "#ffffff" | "#fff" => format!("{}-white", prefix),
        "#000000" | "#000" => format!("{}-black", prefix),
        _ => format!("{}-[{}]", prefix, canonical.replace(' ', "_")),
    }
}

fn color_with_opacity(color: &str, prefix: &str, opacity: Option<f64>) -> String {
    match opacity {
        Some(op) if (0.0..1.0).contains(&op) && Rgba::parse(&json!(color)).is_ok_and(|c| c.0[3] >= 1.0) =>
            format!("{}/{}", hex_to_tailwind_color(color, prefix), number(op * 100.0)),
        _ => hex_to_tailwind_color(color, prefix),
    }
}

fn padding_classes(pad: [f64; 4], classes: &mut Vec<String>) {
    let [top, right, bottom, left] = pad;
    if (top - bottom).abs() < 0.01 && (left - right).abs() < 0.01 {
        if (top - left).abs() < 0.01 {
            if top > 0.0 { classes.push(format!("p-{}", px_to_tailwind_spacing(top))); }
        } else {
            if top > 0.0 { classes.push(format!("py-{}", px_to_tailwind_spacing(top))); }
            if left > 0.0 { classes.push(format!("px-{}", px_to_tailwind_spacing(left))); }
        }
    } else {
        for (side, v) in [("t", top), ("r", right), ("b", bottom), ("l", left)] {
            if v > 0.0 { classes.push(format!("p{side}-{}", px_to_tailwind_spacing(v))); }
        }
    }
}

fn layout_classes(layout: &Value, classes: &mut Vec<String>) {
    let gap_class = |prefix: &str, v: &Value| px(v).filter(|g| *g > 0.0).map(|g| format!("{prefix}-{}", px_to_tailwind_spacing(g)));
    if layout["display"] == "grid" {
        classes.push("grid".into());
        for (key, prefix) in [("columns", "grid-cols"), ("rows", "grid-rows")] {
            if let Some(n) = layout[key].as_u64() {
                classes.push(if n <= 12 { format!("{prefix}-{n}") } else { format!("{prefix}-[repeat({n},minmax(0,1fr))]") });
            }
        }
        classes.extend(gap_class("gap-x", &layout["columnGap"]));
        classes.extend(gap_class("gap-y", &layout["rowGap"]));
    } else {
        classes.push("flex".into());
        classes.push(if layout["flexDirection"] == "column" { "flex-col" } else { "flex-row" }.into());
        let wraps = layout["wrap"] == "wrap" || layout["layoutWrap"] == "WRAP" || layout["wrap"] == true;
        if wraps { classes.push("flex-wrap".into()); }
        match (gap_class("gap-x", &layout["gap"]), wraps.then(|| gap_class("gap-y", &layout["rowGap"])).flatten()) {
            (Some(x), Some(y)) => { classes.push(x); classes.push(y); }
            (Some(_), None) => classes.extend(gap_class("gap", &layout["gap"])),
            (None, Some(y)) => classes.push(y),
            (None, None) => {}
        }
        if let Some(items) = layout["alignItems"].as_str() {
            match items {
                "center" => classes.push("items-center".into()),
                "flex-start" => classes.push("items-start".into()),
                "flex-end" => classes.push("items-end".into()),
                "stretch" => classes.push("items-stretch".into()),
                "baseline" => classes.push("items-baseline".into()),
                _ => {}
            }
        }
        if let Some(justify) = layout["justifyContent"].as_str() {
            match justify {
                "center" => classes.push("justify-center".into()),
                "flex-start" => classes.push("justify-start".into()),
                "flex-end" => classes.push("justify-end".into()),
                "space-between" => classes.push("justify-between".into()),
                _ => {}
            }
        }
    }
    if let Some(pad) = layout.get("padding").and_then(box4) { padding_classes(pad, classes); }
}

const LEAF_SHAPES: [&str; 8] = ["VECTOR", "RECTANGLE", "ELLIPSE", "LINE", "STAR", "POLYGON", "BOOLEAN_OPERATION", "IMAGE"];

/// How a node is sized inside its parent: FIXED → exact px, FILL → flex-1 on the
/// parent's main axis / self-stretch on the cross axis, HUG → intrinsic.
fn sizing_classes(node: &Value, classes: &mut Vec<String>) {
    let (w, h) = node_size(node);
    let parent = node["_parentFlow"].as_str();
    let flex_parent = matches!(parent, Some("row" | "column"));
    let node_type = node["type"].as_str().unwrap_or("");
    let explicit_box = LEAF_SHAPES.contains(&node_type)
        || node["position"]["type"] == "absolute"
        // Frames without auto-layout hold absolutely positioned children.
        || (node.get("layout").is_none() && node["children"].as_array().is_some_and(|c| !c.is_empty()));
    let sizing = node.get("sizing");
    for (axis, size, prefix) in [("horizontal", w, "w"), ("vertical", h, "h")] {
        let main = matches!((axis, parent), ("horizontal", Some("row")) | ("vertical", Some("column")));
        match sizing.and_then(|s| s[axis].as_str()) {
            Some("FILL") if flex_parent && main => {
                classes.push("flex-1".into());
                classes.push(format!("min-{prefix}-0"));
            }
            Some("FILL") if flex_parent => classes.push("self-stretch".into()),
            Some("FILL") => classes.push(format!("{prefix}-full")),
            Some("FIXED") if size > 0.0 => classes.push(format!("{prefix}-[{}px]", number(size))),
            Some(_) => {}
            None if explicit_box && size > 0.0 => classes.push(format!("{prefix}-[{}px]", number(size))),
            None => {}
        }
    }
    if sizing.is_none() {
        if node["layoutGrow"].as_f64().is_some_and(|g| g > 0.0) { classes.push("flex-1".into()); }
        if node["layoutAlign"] == "STRETCH" { classes.push("self-stretch".into()); }
    }
    let limits = node.get("constraints").unwrap_or(node);
    for (key, prefix) in [("minWidth", "min-w"), ("maxWidth", "max-w"), ("minHeight", "min-h"), ("maxHeight", "max-h")] {
        if let Some(v) = limits[key].as_f64().filter(|v| *v > 0.0) {
            classes.push(format!("{prefix}-[{}px]", number(v)));
        }
    }
}

fn stroke_classes(node: &Value, classes: &mut Vec<String>) {
    let Some(stroke) = node.get("stroke") else { return };
    let Some(color) = stroke.as_str().or_else(|| stroke["color"].as_str()) else { return };
    let opacity = stroke["opacity"].as_f64().or_else(|| node["strokeOpacity"].as_f64());
    let weight = &stroke["weight"];
    let sides = weight.as_object().map(|_| box4(weight)).unwrap_or(None);
    let uniform = weight.as_f64().unwrap_or(1.0);
    let dashed = stroke["dashed"] == true;
    // OUTSIDE strokes don't take layout space in Figma; an outline doesn't either.
    if stroke["align"] == "OUTSIDE" && sides.is_none() {
        classes.push(if (uniform - 1.0).abs() < 0.01 { "outline".into() } else { format!("outline outline-[{}px]", number(uniform)) });
        if dashed { classes.push("outline-dashed".into()); }
        classes.push(color_with_opacity(color, "outline", opacity));
        return;
    }
    match sides {
        Some(s) => {
            for (side, v) in ["t", "r", "b", "l"].iter().zip(s) {
                if v > 0.0 { classes.push(if (v - 1.0).abs() < 0.01 { format!("border-{side}") } else { format!("border-{side}-[{}px]", number(v)) }); }
            }
        }
        None if (uniform - 1.0).abs() < 0.01 => classes.push("border".into()),
        None if uniform > 0.0 => classes.push(format!("border-[{}px]", number(uniform))),
        None => return,
    }
    if dashed { classes.push("border-dashed".into()); }
    classes.push(color_with_opacity(color, "border", opacity));
}

fn typography_classes(node: &Value, typo: &Value, classes: &mut Vec<String>) {
    if let Some(size) = typo.get("fontSize").and_then(px) {
        classes.push(format!("text-[{}px]", number(size)));
    }
    if let Some(family) = typo["fontFamily"].as_str().filter(|f| !f.is_empty()) {
        classes.push(format!("font-['{}']", family.replace(['\'', '"'], "").replace(' ', "_")));
    }
    let style = typo["fontWeight"].as_str().unwrap_or("").to_lowercase().replace([' ', '-'], "");
    if style.contains("italic") { classes.push("italic".into()); }
    let weight = typo["fontWeightNumeric"].as_f64().or_else(|| typo["fontWeight"].as_f64()).map(|w| (w as i64).to_string())
        .unwrap_or_else(|| style.replace("italic", ""));
    match weight.as_str() {
        "thin" | "100" => classes.push("font-thin".into()),
        "extralight" | "ultralight" | "200" => classes.push("font-extralight".into()),
        "light" | "300" => classes.push("font-light".into()),
        "regular" | "normal" | "400" | "" if typo.get("fontWeight").is_some_and(|v| !v.is_null()) => classes.push("font-normal".into()),
        "medium" | "500" => classes.push("font-medium".into()),
        "semibold" | "demibold" | "600" => classes.push("font-semibold".into()),
        "bold" | "700" => classes.push("font-bold".into()),
        "extrabold" | "ultrabold" | "800" => classes.push("font-extrabold".into()),
        "black" | "heavy" | "900" => classes.push("font-black".into()),
        w if w.parse::<u16>().is_ok_and(|w| (1..=1000).contains(&w)) => classes.push(format!("font-[{w}]")),
        _ => {}
    }
    match &typo["lineHeight"] {
        Value::String(s) if s.ends_with('%') => {
            if let Ok(p) = s.trim_end_matches('%').parse::<f64>() { classes.push(format!("leading-[{}]", number(p / 100.0))); }
        }
        v => if let Some(lh) = px(v) { classes.push(format!("leading-[{}px]", number(lh))); },
    }
    if let Some(ls) = typo.get("letterSpacing").and_then(px).filter(|v| *v != 0.0) {
        classes.push(format!("tracking-[{}px]", number(ls)));
    }
    match typo["align"].as_str() {
        Some("center") => classes.push("text-center".into()),
        Some("right") => classes.push("text-right".into()),
        Some("justified") => classes.push("text-justify".into()),
        _ => {}
    }
    match typo["textCase"].as_str().or_else(|| typo["textTransform"].as_str()) {
        Some("UPPER" | "uppercase") => classes.push("uppercase".into()),
        Some("LOWER" | "lowercase") => classes.push("lowercase".into()),
        Some("TITLE" | "capitalize") => classes.push("capitalize".into()),
        _ => {}
    }
    match typo["textDecoration"].as_str() {
        Some("UNDERLINE") => classes.push("underline".into()),
        Some("STRIKETHROUGH") => classes.push("line-through".into()),
        _ => {}
    }
    match typo["truncate"].as_u64() {
        Some(1) => classes.push("truncate".into()),
        Some(n) if n > 1 => classes.push(format!("line-clamp-{n}")),
        _ => {}
    }
    let nowrap = typo["autoResize"] == "WIDTH_AND_HEIGHT";
    let multiline = node["characters"].as_str().is_some_and(|c| c.contains('\n'));
    match (nowrap, multiline) {
        (true, true) => classes.push("whitespace-pre".into()),
        (true, false) => classes.push("whitespace-nowrap".into()),
        (false, true) => classes.push("whitespace-pre-line".into()),
        _ => {}
    }
}

fn node_to_tailwind_classes(node: &Value) -> Vec<String> {
    let mut classes = Vec::new();
    let node_type = node["type"].as_str().unwrap_or("");
    let (width, height) = node_size(node);
    let children = node["children"].as_array();

    if let Some(pos) = node.get("position").filter(|p| p["type"] == "absolute") {
        classes.push("absolute".into());
        classes.push(format!("left-[{}px]", number(pos["x"].as_f64().unwrap_or(0.0))));
        classes.push(format!("top-[{}px]", number(pos["y"].as_f64().unwrap_or(0.0))));
    }
    if children.is_some_and(|c| c.iter().any(|c| c["position"]["type"] == "absolute")) {
        classes.push("relative".into());
    }
    if let Some(layout) = node.get("layout") { layout_classes(layout, &mut classes); }
    sizing_classes(node, &mut classes);

    if let Some(op) = node["opacity"].as_f64().filter(|o| *o < 1.0) {
        classes.push(format!("opacity-[{}]", number(op)));
    }
    if let Some(mode) = node["blendMode"].as_str() {
        let css = mode.to_lowercase().replace('_', "-");
        const BLENDS: [&str; 15] = ["multiply", "screen", "overlay", "darken", "lighten", "color-dodge", "color-burn", "hard-light", "soft-light", "difference", "exclusion", "hue", "saturation", "color", "luminosity"];
        if BLENDS.contains(&css.as_str()) { classes.push(format!("mix-blend-{css}")); }
    }
    if node["clipsContent"] == true || children.is_some_and(|c| c.iter().any(|c| c["isMask"] == true)) {
        classes.push("overflow-hidden".into());
    }
    if let Some(r) = node["rotation"].as_f64().filter(|r| r.abs() > 0.01) {
        // Figma rotates counter-clockwise around the top-left corner.
        classes.push(format!("origin-top-left rotate-[{}deg]", number(-r)));
    }

    // Fill: a token reference wins; otherwise the exact paint stack.
    let fill = node["fill"].as_str();
    if let Some(fill) = fill {
        let prefix = if node_type == "TEXT" { "text" } else { "bg" };
        classes.push(color_with_opacity(fill, prefix, node["fillOpacity"].as_f64()));
    }
    if node_type != "TEXT" && !fill.is_some_and(|f| f.starts_with("var(")) {
        if let Some(paints) = node.get("paintData").or_else(|| node.get("fills")).and_then(Value::as_array) {
            let (images, others): (Vec<Value>, Vec<Value>) = paints.iter().filter(|p| p["visible"] != false).cloned().partition(|p| p["type"] == "IMAGE");
            if let Ok(Some((property, value))) = crate::mcp::tokens::background_css(&others, width, height) {
                classes.retain(|class| !class.starts_with("bg-"));
                if property == "background-color" { classes.push(hex_to_tailwind_color(&value, "bg")); }
                else { classes.push(format!("[background:{}]", value.replace(' ', "_"))); }
            }
            if let Some(img) = images.last() {
                classes.push(match img["scaleMode"].as_str() {
                    Some("FIT") => "bg-contain bg-center bg-no-repeat",
                    Some("TILE") => "bg-repeat",
                    _ => "bg-cover bg-center",
                }.into());
            }
        }
    }

    stroke_classes(node, &mut classes);

    if let Some(r) = node.get("borderRadius").and_then(box4) {
        let r = if node_type == "ELLIPSE" { [9999.0; 4] } else { r };
        classes.extend(radius_class(r, width, height));
    } else if node_type == "ELLIPSE" {
        classes.push("rounded-full".into());
    }

    // Keep every supported effect; one unsupported effect no longer drops the stack.
    if let Some(effects) = node["effects"].as_array() {
        let supported: Vec<Value> = effects.iter().filter(|e| crate::mcp::tokens::effect_css(std::slice::from_ref(*e)).is_ok()).cloned().collect();
        if let Ok(css) = crate::mcp::tokens::effect_css(&supported) {
            for (property, value) in css {
                let value = value.replace(' ', "_");
                classes.push(if property == "box-shadow" { format!("shadow-[{value}]") } else { format!("[{property}:{value}]") });
            }
        }
    }

    if let Some(typo) = node.get("typography")
        .or_else(|| node.get("text").filter(|v| v.is_object()))
        .or_else(|| (node_type == "TEXT").then_some(node)) {
        typography_classes(node, typo, &mut classes);
        if fill.is_none() && node_type == "TEXT" {
            if let Some(color) = typo["color"].as_str() { classes.push(hex_to_tailwind_color(color, "text")); }
        }
    }

    classes
}

fn image_attr(node: &Value) -> String {
    node["paintData"].as_array().into_iter().flatten()
        .rfind(|p| p["type"] == "IMAGE" && p["visible"] != false)
        .and_then(|p| p["imageHash"].as_str())
        .map(|h| format!(" data-figma-image=\"{h}\""))
        .unwrap_or_default()
}

fn not_loaded_comment(node: &Value, d: Dialect) -> Option<String> {
    let count = node["childCount"].as_u64().filter(|c| *c > 0)?;
    let id = node["id"].as_str().unwrap_or("");
    let text = format!("{count} children not loaded; read node {id}");
    Some(if d == Dialect::Jsx { format!("{{/* {text} */}}") } else { format!("<!-- {text} -->") })
}

fn text_markup(node: &Value, d: Dialect, fallback: &str) -> String {
    let content = node["characters"].as_str().unwrap_or(fallback);
    let segments = node.pointer("/text/segments").and_then(Value::as_array).filter(|s| s.len() > 1);
    let Some(segments) = segments else { return escape_text(content, d) };
    let attr = if d == Dialect::Jsx { "className" } else { "class" };
    segments.iter().map(|seg| {
        let mut probe = json!({"type": "TEXT", "text": seg});
        if let Some(fill) = seg["fill"].as_str() { probe["fill"] = json!(fill); }
        let classes = node_to_tailwind_classes(&probe);
        let text = escape_text(seg["text"].as_str().unwrap_or(""), d);
        if classes.is_empty() { text } else { format!("<span {attr}=\"{}\">{text}</span>", classes.join(" ")) }
    }).collect()
}

// ── React + Tailwind Generator ───────────────────────────────────────────────

fn generate_react_tailwind(context: &Value, component_name: &str) -> String {
    // Code generation consumes the original tree, never shared-style references.

    // 2. Infer dynamic interactive states, repeaters, and typescript props interface
    let logic = crate::mcp::state_engine::infer_component_logic(context, component_name);

    let mut jsx_buffer = String::new();
    render_markup(context, &mut jsx_buffer, 2, Dialect::Jsx);

    let mut state_decls = String::new();
    for st in &logic.states {
        state_decls.push_str(&format!(
            "  const [{}, {}] = React.useState<{}>(Boolean({}));\n",
            st.state_name, st.setter_name, st.state_type, st.default_value
        ));
    }

    let state_block = if state_decls.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", state_decls)
    };

    format!(
        "import React from 'react';\n\n{interface}\n\nexport const {name}: React.FC<{name}Props> = ({{\n  className = '',\n}}) => {{{state_block}  return (\n{jsx}\n  );\n}};\n\nexport default {name};\n",
        interface = logic.props_interface,
        name = component_name,
        state_block = state_block,
        jsx = jsx_buffer
    )
}

fn render_markup(node: &Value, out: &mut String, indent_level: usize, d: Dialect) {
    // Mask layers clip their siblings (parent gets overflow-hidden); they don't paint.
    if node["isMask"] == true { return; }
    let indent = "  ".repeat(indent_level);
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("FRAME");
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");

    let tag = if node_type == "TEXT" {
        if name_has(node, &["h1", "title", "heading", "headline"]) { "h2" } else { "span" }
    } else if node["role"] == "button" || name_has(node, &["button", "btn"]) {
        "button"
    } else {
        "div"
    };

    let classes = node_to_tailwind_classes(node);
    let attr = if d == Dialect::Jsx { "className" } else { "class" };
    let class_attr = if classes.is_empty() { String::new() } else { format!(" {attr}=\"{}\"", classes.join(" ")) };
    let attrs = format!("{class_attr}{}", image_attr(node));

    if node_type == "TEXT" {
        out.push_str(&format!("{indent}<{tag}{attrs}>{}</{tag}>\n", text_markup(node, d, name)));
        return;
    }

    let children = node["children"].as_array().filter(|c| !c.is_empty());
    let note = not_loaded_comment(node, d);
    if children.is_some() || note.is_some() {
        out.push_str(&format!("{indent}<{tag}{attrs}>\n"));
        for child in children.into_iter().flatten() {
            render_markup(child, out, indent_level + 1, d);
        }
        if let Some(note) = note { out.push_str(&format!("{indent}  {note}\n")); }
        out.push_str(&format!("{indent}</{tag}>\n"));
        return;
    }

    if d == Dialect::Jsx {
        out.push_str(&format!("{indent}<{tag}{attrs} />\n"));
    } else {
        out.push_str(&format!("{indent}<{tag}{attrs}></{tag}>\n"));
    }
}

// ── React + Shadcn/UI Component Generator ─────────────────────────────────────

#[derive(Default)]
struct ShadcnImports {
    button: bool,
    badge: bool,
    card: bool,
    input: bool,
    avatar: bool,
    switch: bool,
    checkbox: bool,
    separator: bool,
}

fn generate_shadcn_react(context: &Value, component_name: &str) -> String {
    let mut imports = ShadcnImports::default();
    let mut jsx_buffer = String::new();
    render_shadcn_node(context, &mut jsx_buffer, 2, &mut imports);

    let mut import_lines = vec!["import React from 'react';".to_string()];
    if imports.button {
        import_lines.push("import { Button } from '@/components/ui/button';".to_string());
    }
    if imports.badge {
        import_lines.push("import { Badge } from '@/components/ui/badge';".to_string());
    }
    if imports.card {
        import_lines.push("import { Card } from '@/components/ui/card';".to_string());
    }
    if imports.input {
        import_lines.push("import { Input } from '@/components/ui/input';".to_string());
    }
    if imports.avatar {
        import_lines.push("import { Avatar, AvatarFallback } from '@/components/ui/avatar';".to_string());
    }
    if imports.switch {
        import_lines.push("import { Switch } from '@/components/ui/switch';".to_string());
    }
    if imports.checkbox {
        import_lines.push("import { Checkbox } from '@/components/ui/checkbox';".to_string());
    }
    if imports.separator {
        import_lines.push("import { Separator } from '@/components/ui/separator';".to_string());
    }

    format!(
        "{imports}\n\ninterface {name}Props {{\n  className?: string;\n}}\n\nexport const {name}: React.FC<{name}Props> = ({{\n  className = '',\n}}) => {{\n  return (\n{jsx}\n  );\n}};\n\nexport default {name};\n",
        imports = import_lines.join("\n"),
        name = component_name,
        jsx = jsx_buffer
    )
}

fn extract_variant_string(node: &Value) -> Option<String> {
    if let Some(v) = node.get("variant") {
        if let Some(s) = v.as_str() {
            return Some(s.to_lowercase());
        }
        if let Some(map) = v.as_object() {
            for (_, val) in map {
                if let Some(s) = val.as_str() {
                    let s_low = s.to_lowercase();
                    if s_low.contains("destructive") || s_low.contains("secondary") || s_low.contains("outline") || s_low.contains("ghost") || s_low.contains("link") || s_low.contains("primary") {
                        return Some(s_low);
                    }
                }
            }
        }
    }
    if let Some(lbl) = node.get("variantLabel").and_then(|v| v.as_str()) {
        return Some(lbl.to_lowercase());
    }
    if let Some(tok) = node.get("fillToken").and_then(|v| v.as_str()) {
        return Some(tok.to_lowercase());
    }
    None
}

fn extract_prop_string(node: &Value, prop_name: &str) -> Option<String> {
    if let Some(props) = node.get("props").and_then(|p| p.as_object()) {
        for (k, v) in props {
            if k.to_lowercase() == prop_name.to_lowercase() {
                if let Some(s) = v.as_str() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

fn is_light_or_empty(fill: &str) -> bool {
    fill.is_empty() || fill == "transparent" || Rgba::parse(&json!(fill)).is_ok_and(|c| c.0[3] == 0.0 || c.0[..3].iter().all(|v| *v >= 0.99))
}

fn render_shadcn_node(node: &Value, out: &mut String, indent_level: usize, imports: &mut ShadcnImports) {
    let indent = "  ".repeat(indent_level);
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("FRAME");
    let role = node.get("role").and_then(|v| v.as_str()).unwrap_or("");
    let is_container = node_type != "TEXT";
    let label_of = |fallback: &str| escape_text(&extract_prop_string(node, "label")
        .or_else(|| extract_prop_string(node, "text"))
        .or_else(|| find_child_text(node))
        .unwrap_or_else(|| fallback.to_string()), Dialect::Jsx);

    // 1. Detect Button
    if is_container && (role == "button" || name_has(node, &["button", "btn"])) {
        imports.button = true;
        let variant_hint = extract_variant_string(node).unwrap_or_default();
        let fill = node.get("fill").and_then(|v| v.as_str()).unwrap_or("");
        let has_stroke = node.get("stroke").is_some();
        let variant = if variant_hint.contains("destructive") || variant_hint.contains("danger") {
            "destructive"
        } else if variant_hint.contains("outline") || (has_stroke && is_light_or_empty(fill)) {
            "outline"
        } else if variant_hint.contains("secondary") {
            "secondary"
        } else if variant_hint.contains("ghost") || (fill.is_empty() && !has_stroke) {
            "ghost"
        } else if variant_hint.contains("link") {
            "link"
        } else {
            "default"
        };
        let label = label_of("Button");
        if variant == "default" {
            out.push_str(&format!("{}<Button>{}</Button>\n", indent, label));
        } else {
            out.push_str(&format!("{}<Button variant=\"{}\">{}</Button>\n", indent, variant, label));
        }
        return;
    }

    // 2. Detect Badge
    if is_container && (role == "badge" || name_has(node, &["badge", "tag", "pill", "chip"])) {
        imports.badge = true;
        let label = label_of("Badge");
        let variant_hint = extract_variant_string(node).unwrap_or_default();
        let variant = ["secondary", "destructive", "outline"].into_iter().find(|v| variant_hint.contains(v))
            .or_else(|| variant_hint.contains("danger").then_some("destructive"));
        match variant {
            Some(v) => out.push_str(&format!("{}<Badge variant=\"{}\">{}</Badge>\n", indent, v, label)),
            None => out.push_str(&format!("{}<Badge>{}</Badge>\n", indent, label)),
        }
        return;
    }

    // 3. Detect Avatar (no invented image URL; the fallback shows initials)
    if is_container && (role == "avatar" || name_has(node, &["avatar", "userpic"])) {
        imports.avatar = true;
        let fallback = escape_text(&extract_prop_string(node, "initials").or_else(|| find_child_text(node)).unwrap_or_default(), Dialect::Jsx);
        out.push_str(&format!("{indent}<Avatar>\n{indent}  <AvatarFallback>{fallback}</AvatarFallback>\n{indent}</Avatar>\n"));
        return;
    }

    // 4. Detect Switch
    if is_container && (role == "switch" || name_has(node, &["switch", "toggle"])) {
        imports.switch = true;
        out.push_str(&format!("{}<Switch />\n", indent));
        return;
    }

    // 5. Detect Checkbox
    if is_container && (role == "checkbox" || name_has(node, &["checkbox"])) {
        imports.checkbox = true;
        out.push_str(&format!("{}<Checkbox />\n", indent));
        return;
    }

    // 6. Detect Divider / Separator
    if role == "divider" || name_has(node, &["divider", "separator"]) {
        imports.separator = true;
        out.push_str(&format!("{}<Separator />\n", indent));
        return;
    }

    // 7. Detect Input
    if is_container && (role == "input" || name_has(node, &["input", "textfield", "searchbar"])) {
        imports.input = true;
        let placeholder = extract_prop_string(node, "placeholder")
            .or_else(|| extract_prop_string(node, "label"))
            .or_else(|| find_child_text(node))
            .unwrap_or_default();
        out.push_str(&format!("{}<Input placeholder={} />\n", indent, serde_json::to_string(&placeholder).unwrap()));
        return;
    }

    let classes = node_to_tailwind_classes(node);
    let class_attr = if classes.is_empty() { String::new() } else { format!(" className=\"{}\"", classes.join(" ")) };

    // 8. Detect Card
    let tag = if (role == "card" || name_has(node, &["card"])) && (node_type == "FRAME" || node_type == "INSTANCE") {
        imports.card = true;
        "Card"
    } else if node_type == "TEXT" {
        "span"
    } else {
        "div"
    };

    if node_type == "TEXT" {
        let name = node["name"].as_str().unwrap_or("");
        out.push_str(&format!("{indent}<{tag}{class_attr}>{}</{tag}>\n", text_markup(node, Dialect::Jsx, name)));
        return;
    }

    let children = node["children"].as_array().filter(|c| !c.is_empty());
    let note = not_loaded_comment(node, Dialect::Jsx);
    if children.is_some() || note.is_some() {
        out.push_str(&format!("{indent}<{tag}{class_attr}>\n"));
        for child in children.into_iter().flatten() {
            render_shadcn_node(child, out, indent_level + 1, imports);
        }
        if let Some(note) = note { out.push_str(&format!("{indent}  {note}\n")); }
        out.push_str(&format!("{indent}</{tag}>\n"));
        return;
    }

    out.push_str(&format!("{indent}<{tag}{class_attr} />\n"));
}

fn find_child_text(node: &Value) -> Option<String> {
    if let Some(c) = node.get("characters").and_then(|v| v.as_str()) {
        return Some(c.to_string());
    }
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            if let Some(txt) = find_child_text(child) {
                return Some(txt);
            }
        }
    }
    None
}

// ── Clean Spec / Token-Pruned AST Generator ──────────────────────────────────

pub fn prune_ast_node(node: &Value) -> Value {
    if let Some(obj) = node.as_object() {
        let mut pruned = serde_json::Map::new();

        for (k, v) in obj {
            match k.as_str() {
                "visible" => { if v.as_bool() == Some(false) { pruned.insert(k.clone(), v.clone()); } }
                "opacity" => { if let Some(op) = v.as_f64() { if (op - 1.0).abs() > 0.01 { pruned.insert(k.clone(), v.clone()); } } }
                "blendMode" => { if v.as_str() != Some("PASS_THROUGH") { pruned.insert(k.clone(), v.clone()); } }
                "padding" => {
                    if let Some(pad_str) = v.as_str() {
                        if pad_str != "0px 0px 0px 0px" && pad_str != "0 0 0 0" {
                            pruned.insert(k.clone(), v.clone());
                        }
                    } else if !v.is_null() {
                        pruned.insert(k.clone(), v.clone());
                    }
                }
                "borderRadius" => {
                    if let Some(r_str) = v.as_str() {
                        if r_str != "0px" && r_str != "0" {
                            pruned.insert(k.clone(), v.clone());
                        }
                    }
                }
                "children" => {
                    if let Some(arr) = v.as_array() {
                        if !arr.is_empty() {
                            let pruned_children: Vec<Value> = arr.iter().map(prune_ast_node).collect();
                            pruned.insert(k.clone(), Value::Array(pruned_children));
                        }
                    }
                }
                _ => {
                    if !v.is_null() {
                        pruned.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        return Value::Object(pruned);
    }
    node.clone()
}

fn generate_clean_spec(context: &Value) -> String {
    let mut out = String::new();
    render_clean_spec_node(context, &mut out, 0);
    out
}

fn render_clean_spec_node(node: &Value, out: &mut String, indent_level: usize) {
    let indent = "  ".repeat(indent_level);
    let name = escape_text(node.get("name").and_then(|v| v.as_str()).unwrap_or("Layer"), Dialect::Html).replace('"', "&quot;");
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("FRAME");

    let mut attrs = Vec::new();
    if let Some(fill) = node.get("fill").and_then(|v| v.as_str()) {
        attrs.push(format!("fill=\"{}\"", fill));
    }
    if let Some(layout) = node.get("layout") {
        if let Some(dir) = layout.get("flexDirection").and_then(|v| v.as_str()) {
            attrs.push(format!("flex=\"{}\"", dir));
        }
        if let Some(gap) = layout.get("gap").and_then(|v| v.as_str()) {
            if gap != "0px" { attrs.push(format!("gap=\"{}\"", gap)); }
        }
    }
    if let Some(radius) = node.get("borderRadius").and_then(|v| v.as_str()) {
        if radius != "0px" { attrs.push(format!("radius=\"{}\"", radius)); }
    }

    let attr_str = if attrs.is_empty() { String::new() } else { format!(" {}", attrs.join(" ")) };

    if node_type == "TEXT" {
        let content = escape_text(node["characters"].as_str().unwrap_or(&name), Dialect::Html);
        out.push_str(&format!("{}<Text name=\"{}\"{}>{}</Text>\n", indent, name, attr_str, content));
        return;
    }

    let children = node.get("children").and_then(|v| v.as_array());
    if let Some(child_nodes) = children {
        if !child_nodes.is_empty() {
            out.push_str(&format!("{}<{} name=\"{}\"{}>\n", indent, node_type, name, attr_str));
            for child in child_nodes {
                render_clean_spec_node(child, out, indent_level + 1);
            }
            out.push_str(&format!("{}</{}>\n", indent, node_type));
            return;
        }
    }

    out.push_str(&format!("{}<{} name=\"{}\"{} />\n", indent, node_type, name, attr_str));
}

/// Concrete colour for platforms without CSS variables (RN, SwiftUI).
fn concrete_fill(node: &Value) -> Option<Rgba> {
    let color = if node["type"] == "TEXT" {
        solid_fill(node).or_else(|| node.pointer("/text/color").and_then(Value::as_str).filter(|c| !c.starts_with("var(")).map(str::to_string))
    } else {
        solid_fill(node)
    }?;
    Rgba::parse(&json!(color)).ok()
}

fn numeric_weight(typo: &Value) -> Option<u16> {
    typo["fontWeightNumeric"].as_f64().map(|w| w as u16).or_else(|| {
        let s = typo["fontWeight"].as_str()?.to_lowercase().replace([' ', '-'], "").replace("italic", "");
        Some(match s.as_str() {
            "thin" => 100, "extralight" | "ultralight" => 200, "light" => 300, "medium" => 500,
            "semibold" | "demibold" => 600, "bold" => 700, "extrabold" | "ultrabold" => 800, "black" | "heavy" => 900,
            _ => 400,
        })
    })
}

fn line_height_px(typo: &Value) -> Option<f64> {
    match &typo["lineHeight"] {
        Value::String(s) if s.ends_with('%') => Some(s.trim_end_matches('%').parse::<f64>().ok()? / 100.0 * px(&typo["fontSize"])?),
        v => px(v),
    }
}

// ── React Native Generator ───────────────────────────────────────────────────

fn generate_react_native(context: &Value, component_name: &str) -> String {
    let mut buffer = String::new();
    render_rn_node(context, &mut buffer, 2);

    format!(
        "import React from 'react';\nimport {{ View, Text, TouchableOpacity }} from 'react-native';\n\ninterface {name}Props {{\n  style?: any;\n}}\n\nexport const {name}: React.FC<{name}Props> = ({{\n  style,\n}}) => {{\n  return (\n{content}\n  );\n}};\n\nexport default {name};\n",
        name = component_name,
        content = buffer
    )
}

fn rn_style(node: &Value) -> Vec<String> {
    let mut s = Vec::new();
    let mut push = |k: &str, v: String| s.push(format!("{k}: {v}"));
    let quote = |v: &str| format!("'{v}'");
    let (w, h) = node_size(node);
    if let Some(layout) = node.get("layout").filter(|l| l["display"] == "flex") {
        if layout["flexDirection"] == "row" { push("flexDirection", quote("row")); }
        if layout["wrap"] == "wrap" { push("flexWrap", quote("wrap")); }
        if let Some(g) = px(&layout["gap"]).filter(|g| *g > 0.0) { push("gap", number(g)); }
        if let Some(g) = px(&layout["rowGap"]).filter(|g| *g > 0.0) { push("rowGap", number(g)); }
        for key in ["alignItems", "justifyContent"] {
            if let Some(v) = layout[key].as_str().filter(|v| *v != "flex-start") { push(key, quote(v)); }
        }
        if let Some([t, r, b, l]) = layout.get("padding").and_then(box4) {
            for (k, v) in [("paddingTop", t), ("paddingRight", r), ("paddingBottom", b), ("paddingLeft", l)] {
                if v > 0.0 { push(k, number(v)); }
            }
        }
    }
    if let Some(pos) = node.get("position").filter(|p| p["type"] == "absolute") {
        push("position", quote("absolute"));
        push("left", number(pos["x"].as_f64().unwrap_or(0.0)));
        push("top", number(pos["y"].as_f64().unwrap_or(0.0)));
    }
    let parent = node["_parentFlow"].as_str();
    let explicit_box = LEAF_SHAPES.contains(&node["type"].as_str().unwrap_or("")) || node["position"]["type"] == "absolute";
    for (axis, size, key) in [("horizontal", w, "width"), ("vertical", h, "height")] {
        let main = matches!((axis, parent), ("horizontal", Some("row")) | ("vertical", Some("column")));
        match node.pointer(&format!("/sizing/{axis}")).and_then(Value::as_str) {
            Some("FILL") if main => push("flex", "1".into()),
            Some("FILL") if parent.is_some() => push("alignSelf", quote("stretch")),
            Some("FILL") => push(key, quote("100%")),
            Some("FIXED") => push(key, number(size)),
            None if explicit_box && size > 0.0 => push(key, number(size)),
            _ => {}
        }
    }
    if node["type"] != "TEXT" {
        if let Some(c) = concrete_fill(node) { push("backgroundColor", quote(&c.css())); }
    }
    if let Some(r) = node.get("borderRadius").and_then(box4) {
        if r.iter().all(|v| (v - r[0]).abs() < 0.01) {
            if r[0] > 0.0 { push("borderRadius", number(r[0])); }
        } else {
            for (k, v) in ["borderTopLeftRadius", "borderTopRightRadius", "borderBottomRightRadius", "borderBottomLeftRadius"].iter().zip(r) {
                push(k, number(v));
            }
        }
    }
    if let Some(stroke) = node.get("stroke") {
        if let Some(c) = stroke["color"].as_str() {
            push("borderWidth", number(stroke["weight"].as_f64().unwrap_or(1.0)));
            push("borderColor", quote(c));
        }
    }
    if let Some(op) = node["opacity"].as_f64().filter(|o| *o < 1.0) { push("opacity", number(op)); }
    if node["clipsContent"] == true { push("overflow", quote("hidden")); }
    if let Some(t) = node.get("text").filter(|t| t.is_object()) {
        if let Some(fs) = px(&t["fontSize"]) { push("fontSize", number(fs)); }
        if let Some(wt) = numeric_weight(t) { push("fontWeight", quote(&wt.to_string())); }
        if let Some(f) = t["fontFamily"].as_str() { push("fontFamily", serde_json::to_string(f).unwrap()); }
        if let Some(lh) = line_height_px(t) { push("lineHeight", number(lh)); }
        if let Some(ls) = px(&t["letterSpacing"]).filter(|v| *v != 0.0) { push("letterSpacing", number(ls)); }
        if let Some(c) = concrete_fill(node) { push("color", quote(&c.css())); }
        if let Some(a) = t["align"].as_str().filter(|a| *a != "left") { push("textAlign", quote(if a == "justified" { "justify" } else { a })); }
        if let Some(tt) = t["textTransform"].as_str() { push("textTransform", quote(tt)); }
        match t["textDecoration"].as_str() {
            Some("UNDERLINE") => push("textDecorationLine", quote("underline")),
            Some("STRIKETHROUGH") => push("textDecorationLine", quote("line-through")),
            _ => {}
        }
    }
    s
}

fn render_rn_node(node: &Value, out: &mut String, indent_level: usize) {
    if node["isMask"] == true { return; }
    let indent = "  ".repeat(indent_level);
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("FRAME");
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");

    let tag = if node_type == "TEXT" {
        "Text"
    } else if node["role"] == "button" || name_has(node, &["button", "btn"]) {
        "TouchableOpacity"
    } else {
        "View"
    };
    let style = rn_style(node);
    let style_attr = if style.is_empty() { String::new() } else { format!(" style={{{{ {} }}}}", style.join(", ")) };

    if node_type == "TEXT" {
        let content = node["characters"].as_str().unwrap_or(name);
        let lines = node.pointer("/text/truncate").and_then(Value::as_u64).map(|n| format!(" numberOfLines={{{n}}}")).unwrap_or_default();
        out.push_str(&format!("{indent}<{tag}{style_attr}{lines}>{}</{tag}>\n", escape_text(content, Dialect::Jsx)));
        return;
    }

    let children = node["children"].as_array().filter(|c| !c.is_empty());
    let note = not_loaded_comment(node, Dialect::Jsx);
    if children.is_some() || note.is_some() {
        out.push_str(&format!("{indent}<{tag}{style_attr}>\n"));
        for child in children.into_iter().flatten() {
            render_rn_node(child, out, indent_level + 1);
        }
        if let Some(note) = note { out.push_str(&format!("{indent}  {note}\n")); }
        out.push_str(&format!("{indent}</{tag}>\n"));
        return;
    }

    out.push_str(&format!("{indent}<{tag}{style_attr} />\n"));
}

// ── Vue 3 SFC Generator ──────────────────────────────────────────────────────

fn generate_vue_tailwind(context: &Value, _component_name: &str) -> String {
    let mut template_buffer = String::new();
    render_markup(context, &mut template_buffer, 1, Dialect::Vue);

    format!(
        "<script setup lang=\"ts\">\n// Generated by Figma Rust MCP (https://github.com/BuiHung1612/figma-mcp)\n</script>\n\n<template>\n{template}</template>\n",
        template = template_buffer
    )
}

// ── Plain HTML Generator ─────────────────────────────────────────────────────

fn generate_html_tailwind(context: &Value) -> String {
    let mut buffer = String::new();
    render_markup(context, &mut buffer, 0, Dialect::Html);
    buffer
}

// ── SwiftUI Generator ────────────────────────────────────────────────────────

fn generate_swiftui(context: &Value, component_name: &str) -> String {
    let mut body_buffer = String::new();
    render_swiftui_node(context, &mut body_buffer, 2);

    format!(
        "import SwiftUI\n\n// Generated by Figma Rust MCP (https://github.com/BuiHung1612/figma-mcp)\nstruct {name}: View {{\n    var body: some View {{\n{body}    }}\n}}\n\n#Preview {{\n    {name}()\n}}\n",
        name = component_name,
        body = body_buffer
    )
}

fn swift_color(c: Rgba) -> String {
    let [r, g, b, a] = c.0;
    format!("Color(red: {}, green: {}, blue: {}, opacity: {})", number(r), number(g), number(b), number(a))
}

fn swift_modifiers(node: &Value, indent: &str, out: &mut String) {
    let (w, h) = node_size(node);
    let mut m = Vec::new();
    if let Some([t, r, b, l]) = node.pointer("/layout/padding").and_then(box4).filter(|p| p.iter().any(|v| *v > 0.0)) {
        m.push(format!(".padding(EdgeInsets(top: {}, leading: {}, bottom: {}, trailing: {}))", number(t), number(l), number(b), number(r)));
    }
    let sizing = |axis: &str| node.pointer(&format!("/sizing/{axis}")).and_then(Value::as_str);
    let explicit_box = LEAF_SHAPES.contains(&node["type"].as_str().unwrap_or("")) || node["position"]["type"] == "absolute";
    let fixed = |axis: &str, size: f64| (sizing(axis) == Some("FIXED") || (sizing(axis).is_none() && explicit_box)) && size > 0.0;
    match (fixed("horizontal", w), fixed("vertical", h)) {
        (true, true) => m.push(format!(".frame(width: {}, height: {})", number(w), number(h))),
        (true, false) => m.push(format!(".frame(width: {})", number(w))),
        (false, true) => m.push(format!(".frame(height: {})", number(h))),
        _ => {}
    }
    if sizing("horizontal") == Some("FILL") { m.push(".frame(maxWidth: .infinity)".into()); }
    if sizing("vertical") == Some("FILL") { m.push(".frame(maxHeight: .infinity)".into()); }
    let radius = node.get("borderRadius").and_then(box4).map(|r| r[0]).filter(|r| *r > 0.0);
    if node["type"] != "TEXT" {
        if let Some(c) = concrete_fill(node) { m.push(format!(".background({})", swift_color(c))); }
    }
    if let Some(r) = radius { m.push(format!(".clipShape(RoundedRectangle(cornerRadius: {}))", number(r))); }
    if let Some(c) = node.pointer("/stroke/color").and_then(Value::as_str).and_then(|c| Rgba::parse(&json!(c)).ok()) {
        let lw = node.pointer("/stroke/weight").and_then(Value::as_f64).unwrap_or(1.0);
        m.push(format!(".overlay(RoundedRectangle(cornerRadius: {}).stroke({}, lineWidth: {}))", number(radius.unwrap_or(0.0)), swift_color(c), number(lw)));
    }
    if let Some(op) = node["opacity"].as_f64().filter(|o| *o < 1.0) { m.push(format!(".opacity({})", number(op))); }
    if let Some(pos) = node.get("position").filter(|p| p["type"] == "absolute") {
        m.push(format!(".offset(x: {}, y: {})", number(pos["x"].as_f64().unwrap_or(0.0)), number(pos["y"].as_f64().unwrap_or(0.0))));
    }
    for line in m { out.push_str(&format!("{indent}    {line}\n")); }
}

fn render_swiftui_node(node: &Value, out: &mut String, indent_level: usize) {
    if node["isMask"] == true { return; }
    let indent = "    ".repeat(indent_level);
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("FRAME");
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");

    if node_type == "TEXT" {
        let content = node["characters"].as_str().unwrap_or(name);
        out.push_str(&format!("{indent}Text({})\n", serde_json::to_string(content).unwrap()));
        if let Some(t) = node.get("text").filter(|t| t.is_object()) {
            if let Some(fs) = px(&t["fontSize"]) {
                let weight = match numeric_weight(t).unwrap_or(400) {
                    0..=150 => "thin", 151..=250 => "ultraLight", 251..=350 => "light", 351..=450 => "regular",
                    451..=550 => "medium", 551..=650 => "semibold", 651..=750 => "bold", 751..=850 => "heavy", _ => "black",
                };
                out.push_str(&format!("{indent}    .font(.system(size: {}, weight: .{weight}))\n", number(fs)));
            }
            if let Some(ls) = px(&t["letterSpacing"]).filter(|v| *v != 0.0) { out.push_str(&format!("{indent}    .tracking({})\n", number(ls))); }
            if let (Some(lh), Some(fs)) = (line_height_px(t), px(&t["fontSize"])) {
                if lh > fs { out.push_str(&format!("{indent}    .lineSpacing({})\n", number(lh - fs))); }
            }
        }
        if let Some(c) = concrete_fill(node) { out.push_str(&format!("{indent}    .foregroundColor({})\n", swift_color(c))); }
        swift_modifiers(node, &indent, out);
        return;
    }

    let children = node["children"].as_array().filter(|c| !c.is_empty());
    if let Some(child_nodes) = children {
        let layout = node.get("layout");
        let row = layout.and_then(|l| l["flexDirection"].as_str()) == Some("row");
        let spacing = layout.and_then(|l| px(&l["gap"])).unwrap_or(0.0);
        let align = layout.and_then(|l| l["alignItems"].as_str()).unwrap_or("flex-start");
        let header = match (layout.is_some(), row) {
            // Frames without auto-layout place children absolutely.
            (false, _) => "ZStack(alignment: .topLeading)".to_string(),
            (true, true) => format!("HStack(alignment: .{}, spacing: {})", match align { "center" => "center", "flex-end" => "bottom", _ => "top" }, number(spacing)),
            (true, false) => format!("VStack(alignment: .{}, spacing: {})", match align { "center" => "center", "flex-end" => "trailing", _ => "leading" }, number(spacing)),
        };
        out.push_str(&format!("{indent}{header} {{\n"));
        for child in child_nodes {
            render_swiftui_node(child, out, indent_level + 1);
        }
        out.push_str(&format!("{indent}}}\n"));
        swift_modifiers(node, &indent, out);
        return;
    }

    let shape = if node_type == "ELLIPSE" { "Ellipse()" } else { "Rectangle()" };
    match concrete_fill(node) {
        Some(c) => out.push_str(&format!("{indent}{shape}.fill({})\n", swift_color(c))),
        None => out.push_str(&format!("{indent}{shape}.fill(Color.clear)\n")),
    }
    let mut leaf = node.clone();
    leaf.as_object_mut().map(|o| o.remove("paintData"));
    leaf.as_object_mut().map(|o| o.remove("fill"));
    swift_modifiers(&leaf, &indent, out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typography_keeps_exact_sizes_and_explicit_weights() {
        for (node, expected) in [
            (json!({"type":"TEXT", "fontSize":15.5, "fontWeight":"Semi Bold"}), vec!["text-[15.5px]", "font-semibold"]),
            (json!({"type":"TEXT", "text":{"fontSize":13, "fontWeight":"Regular"}}), vec!["text-[13px]", "font-normal"]),
            (json!({"typography":{"fontSize":"17.25px", "fontWeight":650}}), vec!["text-[17.25px]", "font-[650]"]),
            (json!({"typography":{"fontSize":22, "fontWeight":"Extra Bold Italic"}}), vec!["text-[22px]", "font-extrabold", "italic"]),
            (json!({"type":"TEXT", "text":{"fontSize":12, "lineHeight":"125%", "letterSpacing":"-0.24px", "align":"center", "textCase":"UPPER", "fontFamily":"Open Sans"}}),
                vec!["leading-[1.25]", "tracking-[-0.24px]", "text-center", "uppercase", "font-['Open_Sans']"]),
        ] {
            let classes = node_to_tailwind_classes(&node);
            for class in expected { assert!(classes.iter().any(|c| c == class), "{classes:?}"); }
        }
    }

    #[test]
    fn exact_shadow_and_alpha_classes_are_preserved() {
        let node = json!({"type":"FRAME", "fill":"rgb(100% 0% 0% / 50%)", "fillOpacity":0.5,
            "effects":[{"type":"DROP_SHADOW", "color":"#00000020", "offset":{"x":1,"y":2}, "radius":7,"spread":-1},
                {"type":"INNER_SHADOW", "color":"rgba(255,0,0,0)", "offset":{"x":0,"y":0}, "radius":0,"spread":0},
                {"type":"DROP_SHADOW", "color":"#000", "offset":{"x":0,"y":0}, "radius":4, "blendMode":"MULTIPLY"},
                {"type":"BACKGROUND_BLUR", "radius":8}]});
        let classes = node_to_tailwind_classes(&node);
        assert!(classes.contains(&"bg-[rgba(255,0,0,0.5)]".into()));
        // The unsupported MULTIPLY shadow is skipped; the others survive.
        let shadow = classes.iter().find(|class| class.starts_with("shadow-[")).unwrap();
        assert!(shadow.contains("1px_2px_7px_-1px"));
        assert!(shadow.contains("inset_0px_0px_0px_0px_rgba(255,_0,_0,_0)"));
        assert!(classes.contains(&"[backdrop-filter:blur(4px)]".into()), "{classes:?}");
    }

    #[test]
    fn sizing_position_radius_and_strokes_follow_figma() {
        let tree = normalize(&json!({"type":"FRAME", "name":"Row", "layout":{"display":"flex","flexDirection":"row","gap":"12.5px","padding":"0px 0px 0px 0px","wrap":"wrap","rowGap":"8px","alignItems":"baseline"},
            "size":{"width":400,"height":300}, "borderRadius":"32px",
            "children":[
                {"type":"FRAME","name":"Fill","sizing":{"horizontal":"FILL","vertical":"FIXED"},"size":{"width":10,"height":44},
                    "stroke":{"color":"#ff0000","weight":{"top":0,"right":0,"bottom":2,"left":0}}, "borderRadius":"8px 8px 0px 0px"},
                {"type":"FRAME","name":"Badge","position":{"type":"absolute","x":380,"y":-4},"size":{"width":16,"height":16},"borderRadius":"100px",
                    "stroke":{"color":"#000","weight":2,"align":"OUTSIDE"}}
            ]}), None);
        let root = node_to_tailwind_classes(&tree);
        for c in ["flex-wrap", "gap-x-[12.5px]", "gap-y-2", "items-baseline", "rounded-[32px]", "relative"] {
            assert!(root.iter().any(|x| x == c), "{c} missing in {root:?}");
        }
        assert!(!root.iter().any(|x| x.contains("md:")));
        let fill = node_to_tailwind_classes(&tree["children"][0]);
        for c in ["flex-1", "h-[44px]", "border-b-[2px]", "border-[#ff0000]", "rounded-[8px_8px_0px_0px]"] {
            assert!(fill.iter().any(|x| x == c), "{c} missing in {fill:?}");
        }
        let badge = node_to_tailwind_classes(&tree["children"][1]);
        for c in ["absolute", "left-[380px]", "top-[-4px]", "w-[16px]", "rounded-full", "outline outline-[2px]", "outline-black"] {
            assert!(badge.iter().any(|x| x == c), "{c} missing in {badge:?}");
        }
    }

    #[test]
    fn token_fills_and_layer_order_are_kept() {
        let node = json!({"type":"FRAME", "fill":"var(--surface-primary)", "paintData":[{"type":"SOLID","color":"#ffffff"}]});
        assert!(node_to_tailwind_classes(&node).contains(&"bg-[color:var(--surface-primary)]".into()));
        let text = json!({"type":"TEXT", "fill":"var(--text-neutral)", "characters":"Hi"});
        assert!(node_to_tailwind_classes(&text).contains(&"text-[color:var(--text-neutral)]".into()));
    }

    #[test]
    fn markup_escapes_text_and_html_closes_tags() {
        let ctx = json!({"name":"Box","type":"FRAME","layout":{"display":"flex","flexDirection":"column"},
            "children":[{"type":"TEXT","name":"Price","text":{"content":"Price {total} < 5"}},{"type":"RECTANGLE","name":"Line","size":{"width":10,"height":1}}]});
        let react = generate_code_from_context(&ctx, "react-tailwind", None).unwrap();
        assert!(react.contains(r#"{"Price {total} < 5"}"#), "{react}");
        let html = generate_code_from_context(&ctx, "html", None).unwrap();
        assert!(html.contains("Price &#123;total} &lt; 5"), "{html}");
        assert!(html.contains("class=\"") && !html.contains("className") && !html.contains("/>"), "{html}");
        let vue = generate_code_from_context(&ctx, "vue-tailwind", None).unwrap();
        assert!(vue.contains("class=\"") && !vue.contains("className"));
        let swift = generate_code_from_context(&json!({"type":"TEXT","name":"Q","text":{"content":"Say \"hi\"","fontSize":14}}), "swiftui", None).unwrap();
        assert!(swift.contains(r#"Text("Say \"hi\"")"#) && swift.contains(".font(.system(size: 14"), "{swift}");
    }

    #[test]
    fn shadcn_matches_whole_words_only() {
        assert!(!name_has(&json!({"name":"Discard Dialog"}), &["card"]));
        assert!(name_has(&json!({"name":"IconButton"}), &["button"]));
        assert!(!name_has(&json!({"name":"Vintage"}), &["tag"]));
        let ctx = json!({"type":"FRAME","name":"Stage","children":[{"type":"FRAME","name":"Primary Button","fill":"#111111","children":[{"type":"TEXT","characters":"Go"}]}]});
        let code = generate_code_from_context(&ctx, "react-shadcn", None).unwrap();
        assert!(code.contains("<Button>Go</Button>") && !code.contains("Badge"), "{code}");
    }

    /// Real `get_design_context` output captured from the plugin.
    #[test]
    fn real_plugin_context_renders_tree_text_and_layout() {
        let raw: Value = serde_json::from_str(include_str!("../../tests/fixtures/design-context-checkout.json")).unwrap();
        let code = generate_code_from_context(&raw, "react-tailwind", None).unwrap();
        assert!(code.contains("export const OptionContainer"));
        assert!(code.matches("<div").count() > 5, "{code}");
        assert!(code.contains(">Home<") && code.contains(">Checkout<"), "{code}");
        assert!(code.contains("flex flex-col") && code.contains("pt-10") && code.contains("pb-20"), "{code}");
        assert!(code.contains("text-[color:var(--text-neutral-secondary)]"), "{code}");
        let rn = generate_code_from_context(&raw, "react-native", None).unwrap();
        assert!(rn.contains("fontSize: 12") && rn.contains("Home"), "{rn}");
    }

    #[test]
    fn react_native_and_swiftui_emit_styles() {
        let ctx = json!({"name":"Card","type":"FRAME","layout":{"display":"flex","flexDirection":"row","gap":"8px","padding":"16px 16px 16px 16px","alignItems":"center"},
            "paintData":[{"type":"SOLID","color":"#ffffff"}], "borderRadius":"12px", "sizing":{"horizontal":"FIXED","vertical":"HUG"}, "size":{"width":320,"height":64},
            "children":[{"type":"TEXT","name":"T","text":{"content":"Hi","fontSize":16,"fontWeight":"Bold","color":"#111111"}}]});
        let rn = generate_code_from_context(&ctx, "react-native", None).unwrap();
        for s in ["flexDirection: 'row'", "gap: 8", "paddingTop: 16", "width: 320", "backgroundColor: '#ffffff'", "borderRadius: 12", "fontWeight: '700'", "color: '#111111'"] {
            assert!(rn.contains(s), "{s} missing in {rn}");
        }
        let swift = generate_code_from_context(&ctx, "swiftui", None).unwrap();
        for s in ["HStack(alignment: .center, spacing: 8)", ".frame(width: 320)", ".clipShape(RoundedRectangle(cornerRadius: 12))", "weight: .bold"] {
            assert!(swift.contains(s), "{s} missing in {swift}");
        }
    }

    #[test]
    fn test_color_opacity_tailwind_generation() {
        let node_rgba = json!({"name": "Overlay", "type": "FRAME", "fill": "rgba(0, 0, 0, 0.3)"});
        assert!(node_to_tailwind_classes(&node_rgba).contains(&"bg-black/30".to_string()));
        let node_hex_with_opacity = json!({"name": "Backdrop", "type": "FRAME", "fill": "#000000", "fillOpacity": 0.3,
            "stroke": { "color": "#ffffff", "opacity": 0.5 }});
        let classes_hex = node_to_tailwind_classes(&node_hex_with_opacity);
        assert!(classes_hex.contains(&"bg-black/30".to_string()));
        assert!(classes_hex.contains(&"border-white/50".to_string()));
    }
}
