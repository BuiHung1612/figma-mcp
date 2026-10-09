use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextElement {
    pub id: String,
    pub name: String,
    pub text: String,
    pub font_size: Option<f64>,
    pub font_weight: Option<String>,
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segments: Option<Value>,
    pub color: String,
    pub line_height: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedIconSpec {
    pub id: String,
    pub name: String,
    pub file_name: String,
    pub local_path: Option<String>,
    pub component_name: String,
    pub import_statement: String,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutSectionSpec {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub bounding_box: Value,
    pub background: Option<String>,
    pub layout_mode: Option<String>,
    pub gap: Option<String>,
    pub padding: Option<String>,
    pub direct_text_elements: Vec<String>,
    pub direct_icons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorTokenSpec {
    pub name: String,
    pub hex: String,
    pub primitive: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultimodalDesignPack {
    pub node_id: String,
    pub node_name: String,
    pub screenshot_path: Option<String>,
    pub all_visible_texts: Vec<TextElement>,
    pub extracted_icons: Vec<ExportedIconSpec>,
    pub layout_sections: Vec<LayoutSectionSpec>,
    pub color_palette: Vec<ColorTokenSpec>,
    pub resolved_tokens: HashMap<String, String>,
    pub matched_codebase_components: HashMap<String, String>,
    pub implementation_checklist: Vec<String>,
}

/// Recursively traverses a Figma node JSON and extracts every single visible text element
pub fn extract_all_text_elements(node: &Value) -> Vec<TextElement> {
    let mut texts = Vec::new();
    traverse_text(node, &mut texts);
    texts
}

fn traverse_text(node: &Value, out: &mut Vec<TextElement>) {
    if !node.is_object() {
        return;
    }

    // get_design_context wraps the optimized tree in `context`. Keep support
    // for both that shape and the raw get_design/get_selection tree.
    if let Some(context) = node.get("context") {
        traverse_text(context, out);
    }

    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let is_visible = node.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
    if !is_visible {
        return;
    }

    if node_type == "TEXT" {
        let text_obj = node.get("text").filter(|v| v.is_object());
        let text_content = text_obj
            .and_then(|v| v.get("content"))
            .or_else(|| node.get("characters"))
            .or_else(|| node.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !text_content.trim().is_empty() {
            let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let font_size = text_obj
                .and_then(|v| v.get("fontSize"))
                .or_else(|| node.get("typography").and_then(|t| t.get("fontSize")))
                .or_else(|| node.get("fontSize"))
                .and_then(|v| v.as_f64()
                    .or_else(|| v.as_str()?.strip_suffix("px")?.parse::<f64>().ok()));

            let font_weight = text_obj
                .and_then(|v| v.get("fontWeight"))
                .or_else(|| node.get("typography").and_then(|t| t.get("fontWeight")))
                .or_else(|| node.get("fontWeight"))
                .filter(|v| !v.is_null())
                .map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()));

            let color = text_obj
                .and_then(|v| v.get("color"))
                .or_else(|| node.get("fill"))
                .and_then(|v| v.as_str())
                .unwrap_or("#000000")
                .to_string();
            let line_height = text_obj
                .and_then(|v| v.get("lineHeight"))
                .or_else(|| node.get("typography").and_then(|t| t.get("lineHeight")))
                .or_else(|| node.get("lineHeight"))
                .and_then(|v| v.as_str())
                .map(String::from);

            out.push(TextElement {
                id,
                name,
                text: text_content.to_string(),
                font_size,
                font_weight,
                font_family: text_obj.and_then(|v| v.get("fontFamily"))
                    .or_else(|| node.get("typography").and_then(|t| t.get("fontFamily")))
                    .or_else(|| node.get("fontFamily")).and_then(Value::as_str).map(str::to_owned),
                segments: text_obj.and_then(|v| v.get("segments")).or_else(|| node.get("segments")).cloned(),
                color,
                line_height,
            });
        }
    }

    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            traverse_text(child, out);
        }
    }
}

/// Transforms raw exported icon list into project-ready React/React Native SVG components
pub fn generate_icon_specs(icons: &[Value], icon_dir: &str) -> Vec<ExportedIconSpec> {
    let mut specs = Vec::new();
    for ic in icons {
        let id = ic.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = ic.get("name").and_then(|v| v.as_str()).unwrap_or("icon").to_string();
        let file_name = ic.get("fileName").and_then(|v| v.as_str()).unwrap_or("icon.svg").to_string();
        let w = ic.get("width").and_then(|v| v.as_f64()).unwrap_or(24.0);
        let h = ic.get("height").and_then(|v| v.as_f64()).unwrap_or(24.0);

        // Sanitize component name e.g. "ic_account_order_history" -> "IcAccountOrderHistory"
        let comp_name = sanitize_svg_component_name(&name);
        let import_statement = format!("import {{ {} }} from '@/{}';", comp_name, icon_dir.trim_start_matches('/'));
        let local_path = format!("{}/{}", icon_dir.trim_end_matches('/'), file_name);

        specs.push(ExportedIconSpec {
            id,
            name,
            file_name,
            local_path: Some(local_path),
            component_name: comp_name,
            import_statement,
            width: w,
            height: h,
        });
    }
    specs
}

fn sanitize_svg_component_name(name: &str) -> String {
    let parts: Vec<&str> = name.split(['-', '_', ' ']).collect();
    let mut out = String::new();
    for p in parts {
        let mut c = p.chars();
        if let Some(f) = c.next() {
            out.push_str(&f.to_uppercase().to_string());
            out.push_str(c.as_str());
        }
    }
    if out.is_empty() {
        "Icon".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_text_keeps_copy_spacing_and_direct_line_height() {
        let texts = extract_all_text_elements(&serde_json::json!({"type":"TEXT","id":"t","content":"  Label\n", "fontSize":15.5,"fontWeight":"Regular","lineHeight":"125%"}));
        assert_eq!(texts[0].text,"  Label\n");
        assert_eq!(texts[0].line_height.as_deref(),Some("125%"));
        assert_eq!(texts[0].font_size,Some(15.5));
    }

    #[test]
    fn text_pack_preserves_flat_weights_and_mixed_runs_without_guessed_defaults() {
        let texts = extract_all_text_elements(&json!({"type":"FRAME", "children":[
            {"type":"TEXT", "content":"Title", "fontSize":15.5,"fontWeight":"Semi Bold"},
            {"type":"TEXT", "text":{"content":"Mixed", "segments":[
                {"text":"Mix", "fontSize":14,"fontWeight":"Regular"},
                {"text":"ed", "fontSize":22,"fontWeight":"Bold"}]}}
        ]}));
        assert_eq!(texts[0].font_size, Some(15.5));
        assert_eq!(texts[0].font_weight.as_deref(), Some("Semi Bold"));
        assert!(texts[1].font_size.is_none());
        assert!(texts[1].font_weight.is_none());
        assert_eq!(texts[1].segments.as_ref().unwrap()[1]["fontSize"], 22);
    }

    #[test]
    fn test_extract_all_text_elements() {
        let tree = json!({
            "name": "Screen",
            "type": "FRAME",
            "children": [
                {
                    "name": "Title",
                    "type": "TEXT",
                    "characters": "Hi, John",
                    "fontSize": 24.0
                },
                {
                    "name": "Badge",
                    "type": "FRAME",
                    "children": [
                        {
                            "name": "TierText",
                            "type": "TEXT",
                            "characters": "Tier: Gold",
                            "fontSize": 14.0
                        }
                    ]
                }
            ]
        });

        let texts = extract_all_text_elements(&tree);
        assert_eq!(texts.len(), 2);
        assert_eq!(texts[0].text, "Hi, John");
        assert_eq!(texts[1].text, "Tier: Gold");
    }

    #[test]
    fn test_generate_icon_specs() {
        let icons = vec![json!({
            "id": "1:2",
            "name": "ic_account_order_history",
            "fileName": "ic_account_order_history.svg",
            "width": 20.0,
            "height": 20.0
        })];

        let specs = generate_icon_specs(&icons, "assets/images");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].component_name, "IcAccountOrderHistory");
        assert_eq!(specs[0].local_path, Some("assets/images/ic_account_order_history.svg".to_string()));
    }

    #[test]
    fn test_extract_text_from_design_context_shape() {
        let context = json!({
            "nodeId": "1:2",
            "context": {
                "id": "1:2",
                "name": "Screen",
                "type": "FRAME",
                "children": [{
                    "id": "1:3",
                    "name": "Title",
                    "type": "TEXT",
                    "text": {
                        "content": "Hello",
                        "fontSize": 24.0,
                        "fontWeight": "Bold",
                        "color": "#123456",
                        "lineHeight": "28px"
                    }
                }]
            }
        });

        let texts = extract_all_text_elements(&context);
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].text, "Hello");
        assert_eq!(texts[0].font_size, Some(24.0));
        assert_eq!(texts[0].color, "#123456");
    }
}
