use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferredState {
    pub state_name: String,
    pub state_type: String, // "boolean" | "string" | "number"
    pub default_value: Value,
    pub setter_name: String,
    pub trigger_event: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferredRepeater {
    pub collection_name: String,
    pub item_type_name: String,
    pub sample_items: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComponentLogicInference {
    pub states: Vec<InferredState>,
    pub repeaters: Vec<InferredRepeater>,
    pub props_interface: String,
}

/// Analyzes a Figma component tree and infers reactive React states, repeaters, and TypeScript types
pub fn infer_component_logic(node: &Value, component_name: &str) -> ComponentLogicInference {
    let mut inference = ComponentLogicInference::default();
    let name_lower = component_name.to_lowercase();

    // 1. Infer common interactive states based on Component name & variants
    if name_lower.contains("dropdown") || name_lower.contains("modal") || name_lower.contains("dialog") || name_lower.contains("menu") || name_lower.contains("popover") {
        inference.states.push(InferredState {
            state_name: "isOpen".to_string(),
            state_type: "boolean".to_string(),
            default_value: json!(false),
            setter_name: "setIsOpen".to_string(),
            trigger_event: "onClick={() => setIsOpen(!isOpen)}".to_string(),
        });
    }

    if name_lower.contains("tab") {
        inference.states.push(InferredState {
            state_name: "activeTab".to_string(),
            state_type: "string".to_string(),
            default_value: json!("tab-1"),
            setter_name: "setActiveTab".to_string(),
            trigger_event: "onClick={() => setActiveTab(tab.id)}".to_string(),
        });
    }

    if name_lower.contains("switch") || name_lower.contains("toggle") || name_lower.contains("checkbox") {
        inference.states.push(InferredState {
            state_name: "isChecked".to_string(),
            state_type: "boolean".to_string(),
            default_value: json!(false),
            setter_name: "setIsChecked".to_string(),
            trigger_event: "onClick={() => setIsChecked(!isChecked)}".to_string(),
        });
    }

    // 2. Repeater Detection (Cards list, Menu items, Table rows)
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        if children.len() >= 2 {
            let first_type = children[0].get("type").and_then(|v| v.as_str()).unwrap_or("");
            let all_same_type = children.iter().all(|c| c.get("type").and_then(|v| v.as_str()).unwrap_or("") == first_type);

            if all_same_type && !first_type.is_empty() {
                let mut samples = Vec::new();
                for (idx, child) in children.iter().enumerate() {
                    let title = child.get("name").and_then(|v| v.as_str()).unwrap_or("Item");
                    let text = child.get("characters").and_then(|v| v.as_str()).unwrap_or(title);
                    samples.push(json!({
                        "id": format!("item-{}", idx + 1),
                        "title": text,
                    }));
                }

                inference.repeaters.push(InferredRepeater {
                    collection_name: "items".to_string(),
                    item_type_name: format!("{}Item", component_name),
                    sample_items: samples,
                });
            }
        }
    }

    // 3. Generate TypeScript Props Interface
    let mut props = Vec::new();
    props.push("  className?: string;".to_string());
    for rep in &inference.repeaters {
        props.push(format!("  {}?: {}[];", rep.collection_name, rep.item_type_name));
    }
    for st in &inference.states {
        props.push(format!("  on{}Change?: (val: {}) => void;", capitalize(&st.state_name), st.state_type));
    }

    inference.props_interface = format!(
        "export interface {}Props {{\n{}\n}}",
        component_name,
        props.join("\n")
    );

    inference
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_dropdown_state() {
        let node = json!({ "name": "UserDropdown", "type": "FRAME" });
        let res = infer_component_logic(&node, "UserDropdown");
        assert_eq!(res.states.len(), 1);
        assert_eq!(res.states[0].state_name, "isOpen");
    }

    #[test]
    fn test_infer_repeater_list() {
        let node = json!({
            "name": "CardList",
            "type": "FRAME",
            "children": [
                { "name": "Card 1", "type": "FRAME", "characters": "Pro Plan" },
                { "name": "Card 2", "type": "FRAME", "characters": "Enterprise Plan" }
            ]
        });
        let res = infer_component_logic(&node, "PricingList");
        assert_eq!(res.repeaters.len(), 1);
        assert_eq!(res.repeaters[0].sample_items.len(), 2);
    }
}
