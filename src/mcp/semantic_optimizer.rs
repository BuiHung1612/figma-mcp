use serde_json::{json, Value};

/// Flattens redundant single-child wrapper frames and merges styles into the parent.
/// Removes purely structural wrappers (e.g. Group 123, AutoLayoutFrame) that have no visual borders/backgrounds.
pub fn optimize_semantic_tree(node: &Value) -> Value {
    if !node.is_object() {
        return node.clone();
    }

    let mut current = node.clone();

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

        if let Some(obj) = current.as_object_mut() {
            if !optimized_children.is_empty() {
                obj.insert("children".to_string(), Value::Array(optimized_children));
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
    let has_effects = node.get("effects").and_then(|v| v.as_array()).map_or(false, |a| !a.is_empty());
    let has_radius = node.get("borderRadius").and_then(|v| v.as_str()).map_or(false, |r| r != "0px");

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
}
