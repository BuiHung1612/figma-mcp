use serde_json::{json, Value};

/// Flattens redundant single-child wrapper frames and merges styles into the parent.
/// Removes purely structural wrappers (e.g. Group 123, AutoLayoutFrame) that have no visual borders/backgrounds.
pub fn optimize_semantic_tree(node: &Value) -> Value {
    if !node.is_object() {
        return node.clone();
    }

    let mut current = node.clone();

    // If payload has a nested "tree", optimize that tree directly
    if let Some(tree_val) = current.get("tree") {
        let opt_tree = optimize_semantic_tree(tree_val);
        if let Some(obj) = current.as_object_mut() {
            obj.insert("tree".to_string(), opt_tree);
        }
        return current;
    }

    // 1. Recursively optimize children first
    if let Some(children) = current.get("children").and_then(|v| v.as_array()) {
        let mut optimized_children = Vec::new();
        for child in children {
            let opt_child = optimize_semantic_tree(child);
            
            // Check if child is an unnecessary single-child wrapper frame without fills or strokes
            if is_redundant_wrapper(&opt_child) {
                if let Some(inner_children) = opt_child.get("children").and_then(|v| v.as_array()) {
                    optimized_children.extend(inner_children.clone());
                    continue;
                }
            }
            optimized_children.push(opt_child);
        }

        // Aggregate screen state frames and repeated list items
        let aggregated_children = aggregate_children_states(optimized_children);

        if let Some(obj) = current.as_object_mut() {
            if !aggregated_children.is_empty() {
                obj.insert("children".to_string(), Value::Array(aggregated_children));
            } else {
                obj.remove("children");
            }
        }
    }

    // 2. Strip default & zero-impact attributes to cut LLM prompt tokens
    prune_zero_impact_attributes(&mut current);

    current
}

/// Checks if a frame is an invisible grouping wrapper with no fills, strokes, or effects
fn is_redundant_wrapper(node: &Value) -> bool {
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if node_type != "FRAME" && node_type != "GROUP" {
        return false;
    }

    let has_fill = node.get("fill").is_some();
    let has_stroke = node.get("stroke").is_some();
    let has_effects = node.get("effects").and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty());
    let has_radius = node.get("borderRadius").and_then(|v| v.as_str()).is_some_and(|r| r != "0px");

    if has_fill || has_stroke || has_effects || has_radius {
        return false;
    }

    // Must have exactly 1 child to be safely flatted without breaking sibling flex rules
    let children_count = node.get("children").and_then(|v| v.as_array()).map_or(0, |c| c.len());
    children_count == 1
}

/// Prunes redundant attributes that match CSS/Figma defaults
fn prune_zero_impact_attributes(node: &mut Value) {
    if let Some(obj) = node.as_object_mut() {
        obj.remove("blendMode");
        obj.remove("isMask");
        obj.remove("layoutVersion");
        obj.remove("strokes");
        
        if obj.get("opacity") == Some(&json!(1.0)) || obj.get("opacity") == Some(&json!(1)) {
            obj.remove("opacity");
        }
        if obj.get("visible") == Some(&json!(true)) {
            obj.remove("visible");
        }
        if obj.get("borderRadius") == Some(&json!("0px")) {
            obj.remove("borderRadius");
        }
        if obj.get("itemSpacing") == Some(&json!(0.0)) || obj.get("itemSpacing") == Some(&json!(0)) {
            obj.remove("itemSpacing");
        }
    }
}

/// Aggregates screen state frames and repeated list items across sibling children
pub fn aggregate_children_states(children: Vec<Value>) -> Vec<Value> {
    if children.len() <= 1 {
        return children;
    }

    // 1. Detect if all or majority of children are screen state frames (e.g. Default, Typing, Success, Error)
    let is_screen_variant_group = children.iter().all(|c| {
        let node_type = c.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let width = c.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let height = c.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
        (node_type == "FRAME" || node_type == "COMPONENT") && width >= 300.0 && height >= 300.0
    });

    if is_screen_variant_group && children.len() >= 2 {
        let first_w = children[0].get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let first_h = children[0].get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);

        let all_same_dim = children.iter().all(|c| {
            let w = c.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let h = c.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
            (w - first_w).abs() <= 10.0 && (h - first_h).abs() <= 10.0
        });

        if all_same_dim {
            let mut base_frame = children[0].clone();
            let mut state_summaries = Vec::new();

            for (idx, child) in children.iter().enumerate() {
                let name = child.get("name").and_then(|v| v.as_str()).unwrap_or("State");
                let id = child.get("id").and_then(|v| v.as_str()).unwrap_or("");

                let diff = if idx == 0 {
                    json!({ "status": "BASE_TEMPLATE" })
                } else {
                    extract_value_diff(&base_frame, child)
                };

                state_summaries.push(json!({
                    "id": id,
                    "stateName": name,
                    "diff": diff
                }));
            }

            if let Some(obj) = base_frame.as_object_mut() {
                obj.insert(
                    "_aggregatedStates".to_string(),
                    json!({
                        "totalStates": children.len(),
                        "baseStateName": children[0].get("name").and_then(|v| v.as_str()).unwrap_or("Default"),
                        "states": state_summaries,
                        "hint": format!("Aggregated {} screen states into base layout + diffs to prevent LLM context overflow.", children.len())
                    }),
                );
            }

            return vec![base_frame];
        }
    }

    children
}

fn extract_value_diff(base: &Value, current: &Value) -> Value {
    let mut diff = json!({});

    // 1. Text diff
    let base_texts = collect_texts(base);
    let cur_texts = collect_texts(current);
    let unique_texts: Vec<String> = cur_texts
        .into_iter()
        .filter(|t| !base_texts.contains(t))
        .take(10)
        .collect();

    if !unique_texts.is_empty() {
        diff["uniqueTexts"] = json!(unique_texts);
    }

    // 2. Component variants and styling changes (e.g. Button State=Disabled -> State=Default, error borders)
    let cur_components = collect_component_variants(current);
    let base_components = collect_component_variants(base);

    let mut component_diffs = Vec::new();
    for (name, var_info) in &cur_components {
        if let Some(base_var) = base_components.get(name) {
            if base_var != var_info {
                component_diffs.push(json!({
                    "component": name,
                    "from": base_var,
                    "to": var_info
                }));
            }
        }
    }
    if !component_diffs.is_empty() {
        diff["componentVariants"] = json!(component_diffs);
    }

    // 3. Fills / Strokes changes
    if let (Some(b_fill), Some(c_fill)) = (base.get("fill"), current.get("fill")) {
        if b_fill != c_fill {
            diff["fill"] = c_fill.clone();
        }
    }
    if let (Some(b_stroke), Some(c_stroke)) = (base.get("stroke"), current.get("stroke")) {
        if b_stroke != c_stroke {
            diff["stroke"] = c_stroke.clone();
        }
    }

    diff
}

fn collect_component_variants(node: &Value) -> std::collections::HashMap<String, Value> {
    let mut map = std::collections::HashMap::new();
    if let Some(c_name) = node.get("componentName").or_else(|| node.get("name")).and_then(|v| v.as_str()) {
        if let Some(variant) = node.get("variant").or_else(|| node.get("variantLabel")) {
            map.insert(c_name.to_string(), variant.clone());
        }
    }
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            map.extend(collect_component_variants(child));
        }
    }
    map
}

fn collect_texts(node: &Value) -> Vec<String> {
    let mut texts = Vec::new();
    if let Some(content) = node.get("content").and_then(|v| v.as_str()) {
        texts.push(content.to_string());
    }
    if let Some(arr) = node.get("textContent").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                texts.push(s.to_string());
            }
        }
    }
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            texts.extend(collect_texts(child));
        }
    }
    texts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimize_semantic_tree_flattening() {
        let raw = json!({
            "name": "Card",
            "type": "FRAME",
            "fill": "#FFFFFF",
            "children": [
                {
                    "name": "Wrapper_Frame_1",
                    "type": "FRAME",
                    "children": [
                        {
                            "name": "Button",
                            "type": "FRAME",
                            "fill": "#7C3AED"
                        }
                    ]
                }
            ]
        });

        let opt = optimize_semantic_tree(&raw);
        let children = opt.get("children").and_then(|v| v.as_array()).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].get("name").and_then(|v| v.as_str()), Some("Button"));
    }

    #[test]
    fn test_aggregate_screen_states() {
        let raw = json!({
            "name": "Sign In Flow",
            "type": "SECTION",
            "children": [
                {
                    "name": "Default",
                    "type": "FRAME",
                    "width": 1920.0,
                    "height": 1080.0,
                    "fill": "#FFFFFF",
                    "content": "Sign in with Email"
                },
                {
                    "name": "Typing",
                    "type": "FRAME",
                    "width": 1920.0,
                    "height": 1080.0,
                    "fill": "#FFFFFF",
                    "content": "user@example.com"
                },
                {
                    "name": "Error",
                    "type": "FRAME",
                    "width": 1920.0,
                    "height": 1080.0,
                    "fill": "#FFFFFF",
                    "content": "Invalid password"
                }
            ]
        });

        let opt = optimize_semantic_tree(&raw);
        let children = opt.get("children").and_then(|v| v.as_array()).unwrap();
        assert_eq!(children.len(), 1);
        let base = &children[0];
        assert_eq!(base.get("name").and_then(|v| v.as_str()), Some("Default"));
        let agg = base.get("_aggregatedStates").unwrap();
        assert_eq!(agg.get("totalStates"), Some(&json!(3)));
        let states = agg.get("states").and_then(|v| v.as_array()).unwrap();
        assert_eq!(states.len(), 3);
    }
}
