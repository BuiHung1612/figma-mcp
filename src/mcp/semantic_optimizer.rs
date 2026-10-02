use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

// Keep node identity, geometry, text and hierarchy inline. Only exact style
// bundles are shared; no heuristic state aggregation or default pruning.
const STYLE_FIELDS: &[&str] = &[
    "fill",
    "fills",
    "stroke",
    "strokes",
    "effects",
    "typography",
    "layout",
    "fontFamily",
    "fontSize",
    "fontWeight",
    "lineHeight",
    "letterSpacing",
    "textAlignHorizontal",
    "textAlignVertical",
    "borderRadius",
    "opacity",
    "blendMode",
    "padding",
    "itemSpacing",
];

fn style_bundle(node: &Map<String, Value>) -> Map<String, Value> {
    STYLE_FIELDS
        .iter()
        .filter_map(|key| {
            node.get(*key)
                .map(|value| ((*key).to_string(), value.clone()))
        })
        .collect()
}

fn visit_nodes(value: &mut Value, visitor: &mut impl FnMut(&mut Map<String, Value>)) {
    match value {
        Value::Object(obj) => {
            // Reserve our metadata names: collisions cause a raw fallback.
            if obj.get("type").is_some_and(Value::is_string) {
                visitor(obj);
            }
            for child in obj.values_mut() {
                visit_nodes(child, visitor);
            }
        }
        Value::Array(items) => {
            for item in items {
                visit_nodes(item, visitor);
            }
        }
        _ => {}
    }
}

/// Lossless relative to the plugin payload. Full mode bypasses compression.
/// Return raw data if metadata collides or the complete encoded result is larger.
pub fn compress_tree(data: &Value, enabled: bool) -> Value {
    if !enabled || !data.is_object() || data.get("_compression").is_some() {
        return data.clone();
    }
    let mut result = data.clone();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut collision = false;
    visit_nodes(&mut result, &mut |node| {
        collision |= node.contains_key("styleRef");
        let style = style_bundle(node);
        if !style.is_empty() {
            *counts.entry(Value::Object(style).to_string()).or_default() += 1;
        }
    });
    if collision {
        return data.clone();
    }

    let mut references = BTreeMap::new();
    let mut styles = Map::new();
    for (style, count) in counts {
        let id = format!("s{}", styles.len());
        // Include reference and table overhead; tiny bundles stay inline.
        if count > 1 && (count - 1) * style.len() > count * (id.len() + 16) + id.len() + 8 {
            styles.insert(
                id.clone(),
                serde_json::from_str(&style).expect("serialized style"),
            );
            references.insert(style, id);
        }
    }
    if styles.is_empty() {
        return data.clone();
    }
    visit_nodes(&mut result, &mut |node| {
        let style = Value::Object(style_bundle(node)).to_string();
        if let Some(id) = references.get(&style) {
            for key in STYLE_FIELDS {
                node.remove(*key);
            }
            node.insert("styleRef".to_string(), json!(id));
        }
    });
    result["_compression"] = json!({
        "version": 1,
        "styles": styles,
        "decode": "Merge _compression.styles[node.styleRef] into each referenced node, remove styleRef and root _compression. All other data is unchanged."
    });
    if result.to_string().len() < data.to_string().len() {
        result
    } else {
        data.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restore(mut data: Value) -> Value {
        if let Some(metadata) = data.as_object_mut().unwrap().remove("_compression") {
            visit_nodes(&mut data, &mut |node| {
                if let Some(id) = node.remove("styleRef") {
                    node.extend(
                        metadata["styles"][id.as_str().unwrap()]
                            .as_object()
                            .unwrap()
                            .clone(),
                    );
                }
            });
        }
        data
    }

    #[test]
    fn roundtrip_preserves_every_node_and_visual_property() {
        let children: Vec<Value> = (0..80).map(|i| json!({
            "id": format!("1:{i}"), "name": format!("State {i}"), "type": "FRAME",
            "width": 402.25, "height": 800, "isMask": i == 1,
            "transform": [[1, 0, i], [0, 1, 0]], "variant": format!("State={i}"),
            "clipsContent": true, "blendMode": "MULTIPLY", "visible": i != 2,
            "opacity": 1, "itemSpacing": 0, "fill": "#ffffff",
            "layout": {"display": "flex", "padding": "16px", "gap": "12px"},
            "typography": {"fontFamily": "Inter", "fontSize": 16, "fontWeight": 600},
            "children": [{"type": "TEXT", "id": format!("2:{i}"), "content": format!("Unique text {i}")}]
        })).collect();
        for field in ["tree", "context", "selection", "fullNode"] {
            let raw = json!({field: {"type": "SECTION", "children": children}, "meta": {"nodesTruncated": false}});
            let compressed = compress_tree(&raw, true);
            assert!(compressed.get("_compression").is_some());
            assert_eq!(restore(compressed.clone()), raw);
            assert_eq!(compress_tree(&raw, false), raw);
            assert_eq!(compress_tree(&compressed, true), compressed);
            eprintln!(
                "{field}: {} -> {} bytes",
                raw.to_string().len(),
                compressed.to_string().len()
            );
        }
    }

    #[test]
    fn small_payload_and_reserved_fields_fall_back_to_raw() {
        for raw in [
            json!({"type": "FRAME", "blendMode": "MULTIPLY", "isMask": true, "children": []}),
            json!({"tree": {"type": "TEXT", "styleRef": "existing", "fill": "#fff"}}),
            json!({"_compression": null}),
            json!([]),
        ] {
            assert_eq!(compress_tree(&raw, true), raw);
        }
    }
}
