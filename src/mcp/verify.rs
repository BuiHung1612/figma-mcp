use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

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
    let mut layout_metrics = Vec::new();
    let mut style_metrics = Vec::new();
    let mut fixes = Vec::new();

    let mut total_checks = 0;
    let mut matched_checks = 0;

    // 1. Width & Height comparison
    if let Some(w) = figma_spec.get("width").and_then(|v| v.as_f64()) {
        total_checks += 1;
        let actual_w = computed_styles.get("width").cloned();
        let is_match = actual_w.as_ref().map_or(false, |val| {
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

    if let Some(h) = figma_spec.get("height").and_then(|v| v.as_f64()) {
        total_checks += 1;
        let actual_h = computed_styles.get("height").cloned();
        let is_match = actual_h.as_ref().map_or(false, |val| {
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

    // 2. Padding comparison
    if let Some(padding) = figma_spec.get("padding") {
        total_checks += 1;
        let actual_pad = computed_styles.get("padding").cloned();
        let expected_pad = if let Some(p) = padding.as_f64() {
            format!("{}px", p)
        } else if let Some(arr) = padding.as_array() {
            arr.iter().map(|v| format!("{}px", v.as_f64().unwrap_or(0.0))).collect::<Vec<_>>().join(" ")
        } else {
            padding.to_string()
        };

        let is_match = actual_pad.as_ref().map_or(false, |act| act == &expected_pad);
        if is_match { matched_checks += 1; } else {
            fixes.push(format!("Adjust padding to `{}`", expected_pad));
            layout_metrics.push(LayoutMetric {
                name: "padding".to_string(),
                figma_value: Some(expected_pad),
                actual_value: actual_pad,
                is_matched: false,
                difference: Some("Padding mismatch".to_string()),
            });
        }
    }

    // 3. Item Spacing / Gap
    if let Some(gap) = figma_spec.get("itemSpacing").and_then(|v| v.as_f64()) {
        total_checks += 1;
        let actual_gap = computed_styles.get("gap").cloned();
        let is_match = actual_gap.as_ref().map_or(false, |val| {
            let px = val.trim_end_matches("px").parse::<f64>().unwrap_or(0.0);
            (px - gap).abs() <= 1.0
        });

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

    // 4. Border Radius
    if let Some(radius) = figma_spec.get("cornerRadius").and_then(|v| v.as_f64()) {
        total_checks += 1;
        let actual_r = computed_styles.get("border-radius").or_else(|| computed_styles.get("borderRadius")).cloned();
        let is_match = actual_r.as_ref().map_or(false, |val| {
            let px = val.trim_end_matches("px").parse::<f64>().unwrap_or(0.0);
            (px - radius).abs() <= 1.0
        });

        if is_match { matched_checks += 1; } else {
            fixes.push(format!("Set border radius: `rounded-[{}px]`", radius));
            style_metrics.push(LayoutMetric {
                name: "borderRadius".to_string(),
                figma_value: Some(format!("{}px", radius)),
                actual_value: actual_r,
                is_matched: false,
                difference: Some(format!("Expected {}px radius", radius)),
            });
        }
    }

    // 5. Typography (fontSize, fontWeight)
    if let Some(font_size) = figma_spec.get("fontSize").and_then(|v| v.as_f64()) {
        total_checks += 1;
        let actual_fs = computed_styles.get("font-size").or_else(|| computed_styles.get("fontSize")).cloned();
        let is_match = actual_fs.as_ref().map_or(false, |val| {
            let px = val.trim_end_matches("px").parse::<f64>().unwrap_or(0.0);
            (px - font_size).abs() <= 1.0
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

    // 6. Color / Fill
    if let Some(hex) = figma_spec.get("fill").and_then(|v| v.as_str()) {
        total_checks += 1;
        let actual_bg = computed_styles.get("background-color").or_else(|| computed_styles.get("backgroundColor")).cloned();
        let target_hex = hex.trim_start_matches('#').to_lowercase();
        let is_match = actual_bg.as_ref().map_or(false, |act| {
            let act_lower = act.to_lowercase();
            if act_lower.contains(&target_hex) {
                return true;
            }
            // Parse rgb(r, g, b)
            if let Some(caps) = act_lower.strip_prefix("rgb(")
                .and_then(|s| s.strip_suffix(")"))
                .or_else(|| act_lower.strip_prefix("rgba(").and_then(|s| s.strip_suffix(")")))
            {
                let parts: Vec<u8> = caps
                    .split(',')
                    .filter_map(|p| p.trim().parse::<u8>().ok())
                    .collect();
                if parts.len() >= 3 {
                    let act_hex = format!("{:02x}{:02x}{:02x}", parts[0], parts[1], parts[2]);
                    return act_hex == target_hex;
                }
            }
            false
        });

        if is_match { matched_checks += 1; } else {
            fixes.push(format!("Fix background / fill color to `{}`", hex));
            style_metrics.push(LayoutMetric {
                name: "backgroundColor".to_string(),
                figma_value: Some(hex.to_string()),
                actual_value: actual_bg,
                is_matched: false,
                difference: Some(format!("Expected color {}", hex)),
            });
        }
    }

    let percentage = if total_checks > 0 {
        ((matched_checks as f64) / (total_checks as f64)) * 100.0
    } else {
        100.0
    };

    (layout_metrics, style_metrics, fixes, percentage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_compare_design_metrics() {
        let figma_spec = json!({
            "width": 320.0,
            "height": 48.0,
            "padding": 16.0,
            "cornerRadius": 8.0,
            "fontSize": 14.0,
            "fill": "#7C3AED"
        });

        let mut computed = HashMap::new();
        computed.insert("width".to_string(), "320px".to_string());
        computed.insert("height".to_string(), "48px".to_string());
        computed.insert("padding".to_string(), "16px".to_string());
        computed.insert("borderRadius".to_string(), "8px".to_string());
        computed.insert("fontSize".to_string(), "14px".to_string());
        computed.insert("backgroundColor".to_string(), "rgb(124, 58, 237)".to_string()); // Matches #7c3aed

        let (layout_diffs, style_diffs, fixes, percentage) = compare_design_metrics(&figma_spec, &computed);
        assert!(layout_diffs.is_empty());
        assert!(style_diffs.is_empty());
        assert!(fixes.is_empty());
        assert_eq!(percentage, 100.0);
    }
}

