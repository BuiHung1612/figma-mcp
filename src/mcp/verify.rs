use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

fn css_font_weight(value: &str) -> Option<u16> {
    let value = value.to_lowercase().replace([' ', '-'], "");
    let value = value.strip_suffix("italic").unwrap_or(&value);
    match value {
        "thin" => Some(100), "extralight" | "ultralight" => Some(200), "light" => Some(300),
        "regular" | "normal" | "" => Some(400), "medium" => Some(500), "semibold" => Some(600),
        "bold" => Some(700), "extrabold" | "ultrabold" => Some(800), "black" | "heavy" => Some(900),
        _ => value.parse::<u16>().ok().filter(|w| (1..=1000).contains(w)),
    }
}

pub(crate) fn px(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.trim().trim_end_matches("px").parse().ok())
}

/// CSS box shorthand (1–4 values), arrays, numbers or {top,right,bottom,left} → [t, r, b, l].
pub(crate) fn box4(v: &Value) -> Option<[f64; 4]> {
    if let Some(n) = px(v) { return Some([n; 4]); }
    let parts: Vec<f64> = if let Some(s) = v.as_str() {
        s.split_whitespace().map(|p| px(&Value::from(p))).collect::<Option<_>>()?
    } else if let Some(a) = v.as_array() {
        a.iter().map(px).collect::<Option<_>>()?
    } else {
        let o = v.as_object()?;
        return Some([px(o.get("top")?)?, px(o.get("right")?)?, px(o.get("bottom")?)?, px(o.get("left")?)?]);
    };
    match parts[..] {
        [a] => Some([a; 4]),
        [a, b] => Some([a, b, a, b]),
        [a, b, c] => Some([a, b, c, b]),
        [a, b, c, d] => Some([a, b, c, d]),
        _ => None,
    }
}

fn fmt_box(b: [f64; 4]) -> String {
    b.iter().map(|v| format!("{}px", crate::mcp::color::number(*v))).collect::<Vec<_>>().join(" ")
}

fn actual_box(computed: &HashMap<String, String>, shorthand: &str, longhands: [&str; 4]) -> Option<[f64; 4]> {
    let sides: Option<Vec<f64>> = longhands.iter().map(|k| computed.get(*k).and_then(|v| px(&Value::from(v.as_str())))).collect();
    if let Some(s) = sides { return Some([s[0], s[1], s[2], s[3]]); }
    computed.get(shorthand).and_then(|v| box4(&Value::from(v.as_str())))
}

/// Concrete colour behind a node's fill: token references (`var(--x)`) fall back
/// to the resolved top-most solid paint so they can still be compared.
pub(crate) fn solid_fill(spec: &Value) -> Option<String> {
    let fill = spec.get("fill").and_then(Value::as_str);
    if let Some(f) = fill.filter(|f| !f.starts_with("var(")) { return Some(f.to_string()); }
    if let Some(v) = spec.pointer("/backgroundCss/value").and_then(Value::as_str) {
        if spec.pointer("/backgroundCss/property").and_then(Value::as_str) == Some("background-color") { return Some(v.to_string()); }
    }
    spec.get("paintData").or_else(|| spec.get("fills")).and_then(Value::as_array)?
        .iter().rev().find(|p| p["visible"] != false && p["type"] == "SOLID")
        .and_then(|p| p.get("color").and_then(Value::as_str)).map(str::to_string)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutMetric {
    pub name: String,
    pub figma_value: Option<String>,
    pub actual_value: Option<String>,
    pub is_matched: bool,
    pub difference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub node_id: String,
    pub node_name: String,
    pub target_url: Option<String>,
    pub target_selector: Option<String>,
    pub match_percentage: f64,
    pub layout_discrepancies: Vec<LayoutMetric>,
    pub style_discrepancies: Vec<LayoutMetric>,
    pub actionable_fixes: Vec<String>,
    pub visual_summary: String,
}

impl VerificationReport {
    pub fn new(node_id: &str, node_name: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            node_name: node_name.to_string(),
            target_url: None,
            target_selector: None,
            match_percentage: 100.0,
            layout_discrepancies: Vec::new(),
            style_discrepancies: Vec::new(),
            actionable_fixes: Vec::new(),
            visual_summary: String::new(),
        }
    }
}

/// Helper function to compare computed CSS values with Figma properties
pub fn compare_design_metrics(
    figma_spec: &Value,
    computed_styles: &HashMap<String, String>,
) -> (Vec<LayoutMetric>, Vec<LayoutMetric>, Vec<String>, f64) {
    let figma_spec = figma_spec.get("context").unwrap_or(figma_spec);
    // Accept the plugin's get_design_context shape (size/layout/borderRadius),
    // the index's to_css_spec shape (css map) and flat Figma fields.
    let css = figma_spec.get("css").cloned().unwrap_or(Value::Null);
    let spec_width = figma_spec.get("width").and_then(px).or_else(|| figma_spec.pointer("/size/width").and_then(px));
    let spec_height = figma_spec.get("height").and_then(px).or_else(|| figma_spec.pointer("/size/height").and_then(px));
    let spec_padding = figma_spec.get("padding").or_else(|| figma_spec.pointer("/layout/padding")).or_else(|| css.get("padding")).and_then(box4);
    let spec_gap = figma_spec.get("itemSpacing").and_then(px)
        .or_else(|| figma_spec.pointer("/layout/gap").and_then(px)).or_else(|| css.get("gap").and_then(px));
    let spec_radius = figma_spec.get("cornerRadius").or_else(|| figma_spec.get("borderRadius")).or_else(|| css.get("border-radius")).and_then(box4);
    let is_text = figma_spec.get("type").and_then(Value::as_str) == Some("TEXT");
    let typography = figma_spec.get("text").filter(|v| v.is_object())
        .or_else(|| figma_spec.get("typography")).unwrap_or(figma_spec);
    let mut layout_metrics = Vec::new();
    let mut style_metrics = Vec::new();
    let mut fixes = Vec::new();

    let mut total_checks = 0;
    let mut matched_checks = 0;

    // 1. Width & Height comparison
    if let Some(w) = spec_width {
        total_checks += 1;
        let actual_w = computed_styles.get("width").cloned();
        let is_match = actual_w.as_ref().is_some_and(|val| {
            let px = val.trim_end_matches("px").parse::<f64>().unwrap_or(0.0);
            (px - w).abs() <= 2.0 // Tolerant 2px
        });

        if is_match { matched_checks += 1; } else {
            let diff = format!("Expected {}px, got {}", w, actual_w.as_deref().unwrap_or("unknown"));
            fixes.push(format!("Adjust width: expected `w-[{}px]` or appropriate responsive width constraint", w));
            layout_metrics.push(LayoutMetric {
                name: "width".to_string(),
                figma_value: Some(format!("{}px", w)),
                actual_value: actual_w,
                is_matched: false,
                difference: Some(diff),
            });
        }
    }

    if let Some(h) = spec_height {
        total_checks += 1;
        let actual_h = computed_styles.get("height").cloned();
        let is_match = actual_h.as_ref().is_some_and(|val| {
            let px = val.trim_end_matches("px").parse::<f64>().unwrap_or(0.0);
            (px - h).abs() <= 2.0
        });

        if is_match { matched_checks += 1; } else {
            let diff = format!("Expected {}px, got {}", h, actual_h.as_deref().unwrap_or("unknown"));
            fixes.push(format!("Adjust height / min-height: expected `h-[{}px]` or `min-h-[{}px]`", h, h));
            layout_metrics.push(LayoutMetric {
                name: "height".to_string(),
                figma_value: Some(format!("{}px", h)),
                actual_value: actual_h,
                is_matched: false,
                difference: Some(diff),
            });
        }
    }

    // 2. Padding comparison (per side, 1px tolerance)
    if let Some(expected) = spec_padding {
        total_checks += 1;
        let actual = actual_box(computed_styles, "padding", ["padding-top", "padding-right", "padding-bottom", "padding-left"]);
        if actual.is_some_and(|a| a.iter().zip(expected).all(|(a, e)| (a - e).abs() <= 1.0)) { matched_checks += 1; } else {
            fixes.push(format!("Adjust padding to `{}`", fmt_box(expected)));
            layout_metrics.push(LayoutMetric {
                name: "padding".to_string(),
                figma_value: Some(fmt_box(expected)),
                actual_value: actual.map(fmt_box),
                is_matched: false,
                difference: Some("Padding mismatch".to_string()),
            });
        }
    }

    // 3. Item Spacing / Gap
    if let Some(gap) = spec_gap {
        total_checks += 1;
        let actual_gap = computed_styles.get("gap").or_else(|| computed_styles.get("column-gap")).cloned();
        // CSS reports an unset gap as "normal" (0 for flex).
        let is_match = (px(&Value::from(actual_gap.as_deref().unwrap_or("normal"))).unwrap_or(0.0) - gap).abs() <= 1.0;

        if is_match { matched_checks += 1; } else {
            fixes.push(format!("Set flex gap: `gap-[{}px]`", gap));
            layout_metrics.push(LayoutMetric {
                name: "gap".to_string(),
                figma_value: Some(format!("{}px", gap)),
                actual_value: actual_gap,
                is_matched: false,
                difference: Some(format!("Expected {}px gap", gap)),
            });
        }
    }

    // 4. Border Radius (per corner: top-left, top-right, bottom-right, bottom-left)
    if let Some(expected) = spec_radius {
        total_checks += 1;
        let actual = actual_box(computed_styles, "border-radius", ["border-top-left-radius", "border-top-right-radius", "border-bottom-right-radius", "border-bottom-left-radius"])
            .or_else(|| computed_styles.get("borderRadius").and_then(|v| box4(&Value::from(v.as_str()))));
        if actual.is_some_and(|a| a.iter().zip(expected).all(|(a, e)| (a - e).abs() <= 1.0)) { matched_checks += 1; } else {
            fixes.push(format!("Set border radius: `{}`", fmt_box(expected)));
            style_metrics.push(LayoutMetric {
                name: "borderRadius".to_string(),
                figma_value: Some(fmt_box(expected)),
                actual_value: actual.map(fmt_box),
                is_matched: false,
                difference: Some("Border radius mismatch".to_string()),
            });
        }
    }

    // 5. Typography (fontSize, fontWeight)
    if let Some(font_size) = typography.get("fontSize").and_then(|v| v.as_f64()
        .or_else(|| v.as_str()?.strip_suffix("px")?.parse::<f64>().ok())) {
        total_checks += 1;
        let actual_fs = computed_styles.get("font-size").or_else(|| computed_styles.get("fontSize")).cloned();
        let is_match = actual_fs.as_ref().is_some_and(|val| {
            val.strip_suffix("px").and_then(|v| v.trim().parse::<f64>().ok())
                .is_some_and(|px| (px - font_size).abs() <= 0.01)
        });

        if is_match { matched_checks += 1; } else {
            fixes.push(format!("Adjust typography font-size: `text-[{}px]`", font_size));
            style_metrics.push(LayoutMetric {
                name: "fontSize".to_string(),
                figma_value: Some(format!("{}px", font_size)),
                actual_value: actual_fs,
                is_matched: false,
                difference: Some(format!("Expected {}px font-size", font_size)),
            });
        }
    }

    for (field, css) in [("fontWeight", "font-weight"), ("fontFamily", "font-family")] {
        if let Some(expected) = typography.get(field).filter(|v| !v.is_null()) {
            let expected = expected.as_str().map(str::to_owned).unwrap_or_else(|| expected.to_string());
            total_checks += 1;
            let actual = computed_styles.get(css).or_else(|| computed_styles.get(field)).cloned();
            let matched = actual.as_ref().is_some_and(|actual| {
                if field == "fontWeight" {
                    css_font_weight(&expected).zip(css_font_weight(actual)).is_some_and(|(a,b)| a == b)
                } else {
                    actual.split(',').next().unwrap_or("").trim().trim_matches(['\'', '"']).eq_ignore_ascii_case(&expected)
                }
            });
            if matched { matched_checks += 1; } else {
                fixes.push(format!("Set {css}: {expected}"));
                style_metrics.push(LayoutMetric { name:field.into(), figma_value:Some(expected), actual_value:actual,
                    is_matched:false, difference:Some(format!("Typography {field} differs")) });
            }
        }
    }

    // 6. Color / Fill
    // A TEXT node's fill is its glyph colour, compared against CSS `color`.
    if let Some(hex) = solid_fill(figma_spec).or_else(|| typography.get("color").and_then(Value::as_str).filter(|c| !c.starts_with("var(")).map(str::to_string)) {
        let hex = hex.as_str();
        let (css_key, camel) = if is_text { ("color", "color") } else { ("background-color", "backgroundColor") };
        total_checks += 1;
        let actual_bg = computed_styles.get(css_key).or_else(|| computed_styles.get(camel)).cloned();
        let is_match = actual_bg.as_ref().is_some_and(|actual| {
            match (crate::mcp::color::Rgba::parse(&serde_json::json!(hex)), crate::mcp::color::Rgba::parse(&serde_json::json!(actual))) {
                (Ok(expected), Ok(actual)) => expected.matches(actual),
                _ => false,
            }
        });

        if is_match { matched_checks += 1; } else {
            fixes.push(format!("Fix {css_key} to `{}`", hex));
            style_metrics.push(LayoutMetric {
                name: camel.to_string(),
                figma_value: Some(hex.to_string()),
                actual_value: actual_bg,
                is_matched: false,
                difference: Some(format!("Expected color {}", hex)),
            });
        }
    }

    // 7. Typography Text Transform (uppercase, lowercase, capitalize)
    let figma_text_transform = figma_spec.get("textTransform")
        .and_then(|v| v.as_str())
        .or_else(|| figma_spec.get("text_transform").and_then(|v| v.as_str()))
        .or_else(|| figma_spec.get("typography").and_then(|v| v.get("textTransform")).and_then(|v| v.as_str()))
        .or_else(|| figma_spec.get("context").and_then(|v| v.get("text")).and_then(|v| v.get("textTransform")).and_then(|v| v.as_str()))
        .or_else(|| {
            // Check in child nodes if container (e.g. Button has a child label)
            if let Some(children) = figma_spec.get("context").and_then(|v| v.get("children")).and_then(|v| v.as_array()) {
                for c in children {
                    if let Some(tt) = c.get("text").and_then(|t| t.get("textTransform")).and_then(|v| v.as_str()) {
                        return Some(tt);
                    }
                }
            }
            None
        });

    if let Some(expected_tt) = figma_text_transform {
        total_checks += 1;
        let actual_tt = computed_styles.get("text-transform")
            .or_else(|| computed_styles.get("textTransform"))
            .cloned();

        let is_match = actual_tt.as_deref().map(|s| s.to_lowercase()) == Some(expected_tt.to_lowercase());
        if is_match {
            matched_checks += 1;
        } else {
            fixes.push(format!("Set text-transform: `{}` (e.g., class `{}`)", expected_tt, expected_tt));
            style_metrics.push(LayoutMetric {
                name: "textTransform".to_string(),
                figma_value: Some(expected_tt.to_string()),
                actual_value: actual_tt.clone(),
                is_matched: false,
                difference: Some(format!("Expected text-transform '{}', got '{}'", expected_tt, actual_tt.as_deref().unwrap_or("none"))),
            });
        }
    }

    if let Some(segments) = typography.get("segments").and_then(Value::as_array) {
        let actual: Value = computed_styles.get("segments").and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
        for (i, segment) in segments.iter().enumerate() {
            let styles: HashMap<String, String> = actual.get(i).and_then(Value::as_object).map(|obj| obj.iter()
                .map(|(key, value)| (key.clone(), value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string()))).collect()).unwrap_or_default();
            // Per-run typography only: do not compare geometry/paint on a text run.
            let expected = serde_json::json!({"fontSize":segment.get("fontSize"), "fontWeight":segment.get("fontWeight"), "fontFamily":segment.get("fontFamily")});
            let (_, differences, run_fixes, score) = compare_design_metrics(&expected, &styles);
            total_checks += 1;
            if score == 100.0 { matched_checks += 1; }
            for mut difference in differences { difference.name = format!("segments[{i}].{}", difference.name); style_metrics.push(difference); }
            fixes.extend(run_fixes.into_iter().map(|fix| format!("Segment {i}: {fix}")));
        }
    }

    // An empty spec is not a successful verification. Returning 100 here made
    // missing/incomplete Figma payloads look like perfect matches.
    let percentage = if total_checks > 0 {
        ((matched_checks as f64) / (total_checks as f64)) * 100.0
    } else {
        0.0
    };

    (layout_metrics, style_metrics, fixes, percentage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn verifies_exact_nested_typography_and_mixed_runs() {
        let spec = json!({"context":{"text":{"fontSize":15.5,"fontWeight":"Semi Bold","fontFamily":"Inter"}}});
        let mut styles = HashMap::from([("font-size".into(),"15.5px".into()),("font-weight".into(),"600".into()),("font-family".into(),"\"Inter\", sans-serif".into())]);
        assert_eq!(compare_design_metrics(&spec, &styles).3, 100.0);
        styles.insert("font-size".into(),"16px".into());
        assert!(compare_design_metrics(&spec, &styles).1.iter().any(|d| d.name == "fontSize"));
        styles.insert("font-weight".into(),"400".into());
        assert!(compare_design_metrics(&spec, &styles).1.iter().any(|d| d.name == "fontWeight"));
        let mixed = json!({"segments":[{"fontSize":14,"fontWeight":"Regular"},{"fontSize":22,"fontWeight":"Bold"}]});
        let styles = HashMap::from([("segments".into(),json!([{"font-size":"14px","font-weight":"400"},{"font-size":"22px","font-weight":"700"}]).to_string())]);
        assert_eq!(compare_design_metrics(&mixed, &styles).3,100.0);
        assert!(compare_design_metrics(&mixed,&HashMap::new()).1.iter().any(|d| d.name == "segments[1].fontWeight"));
    }

    #[test]
    fn alpha_and_modern_color_syntax_are_compared_numerically() {
        let spec = json!({"fill":"#ff000080"});
        let mut styles = HashMap::new();
        styles.insert("background-color".into(), "rgb(100% 0% 0% / 50%)".into());
        let (_, differences, _, percentage) = compare_design_metrics(&spec, &styles);
        assert!(differences.is_empty());
        assert_eq!(percentage, 100.0);
        styles.insert("background-color".into(), "rgb(255, 0, 0)".into());
        let (_, differences, _, percentage) = compare_design_metrics(&spec, &styles);
        assert!(!differences.is_empty());
        assert_eq!(percentage, 0.0);
    }

    #[test]
    fn test_compare_design_metrics() {
        let figma_spec = json!({
            "width": 320.0,
            "height": 48.0,
            "padding": 16.0,
            "cornerRadius": 8.0,
            "fontSize": 14.0,
            "fill": "#7C3AED",
            "textTransform": "uppercase"
        });

        let mut computed = HashMap::new();
        computed.insert("width".to_string(), "320px".to_string());
        computed.insert("height".to_string(), "48px".to_string());
        computed.insert("padding".to_string(), "16px".to_string());
        computed.insert("borderRadius".to_string(), "8px".to_string());
        computed.insert("fontSize".to_string(), "14px".to_string());
        computed.insert("backgroundColor".to_string(), "rgb(124, 58, 237)".to_string()); // Matches #7c3aed
        computed.insert("textTransform".to_string(), "uppercase".to_string());

        let (layout_diffs, style_diffs, fixes, percentage) = compare_design_metrics(&figma_spec, &computed);
        assert!(layout_diffs.is_empty());
        assert!(style_diffs.is_empty());
        assert!(fixes.is_empty());
        assert_eq!(percentage, 100.0);
    }

    /// Real `get_design_context` output: size, padding, gap and radius live
    /// under size/layout/borderRadius and must all be checked.
    #[test]
    fn plugin_context_layout_is_verified() {
        let spec: Value = serde_json::from_str(include_str!("../../tests/fixtures/design-context-checkout.json")).unwrap();
        let good = HashMap::from([("width".into(), "1512px".into()), ("height".into(), "1116px".into()),
            ("padding".into(), "40px 16px 80px".into()), ("gap".into(), "40px".into()), ("border-radius".into(), "6px".into())]);
        let (layout, style, _, pct) = compare_design_metrics(&spec, &good);
        assert!(layout.is_empty() && style.is_empty(), "{layout:?} {style:?}");
        assert_eq!(pct, 100.0);
        let bad = HashMap::from([("width".into(), "1512px".into()), ("height".into(), "1116px".into()),
            ("padding".into(), "16px".into()), ("gap".into(), "24px".into()), ("border-radius".into(), "6px".into())]);
        let (layout, _, _, pct) = compare_design_metrics(&spec, &bad);
        assert!(layout.iter().any(|m| m.name == "padding") && layout.iter().any(|m| m.name == "gap"), "{layout:?}");
        assert!(pct < 100.0);
        // A TEXT node's colour is checked against CSS `color`, through its token's resolved paint.
        let text = &spec["context"]["children"][1];
        assert_eq!(text["type"], "TEXT");
        let expected = solid_fill(text).unwrap();
        let (_, style, _, _) = compare_design_metrics(text, &HashMap::from([("color".into(), expected)]));
        assert!(!style.iter().any(|m| m.name == "color"), "{style:?}");
    }

    #[test]
    fn empty_spec_is_not_reported_as_a_match() {
        let (layout, style, fixes, percentage) = compare_design_metrics(&json!({}), &HashMap::new());
        assert!(layout.is_empty());
        assert!(style.is_empty());
        assert!(fixes.is_empty());
        assert_eq!(percentage, 0.0);
    }
}
