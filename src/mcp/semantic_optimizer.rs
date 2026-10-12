use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

// Keep node identity, geometry, text and hierarchy inline. Exact style bundles
// are shared, and a value every layer carries is stored once as a default;
// no heuristic state aggregation.
const STYLE_FIELDS: &[&str] = &[
    "paintData",
    "cornerRadius", "topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius",
    "strokeAlign", "strokeWeight", "constraints",
    "layoutMode", "layoutWrap", "counterAxisSpacing",
    "paddingTop", "paddingRight", "paddingBottom", "paddingLeft",
    "primaryAxisAlignItems", "counterAxisAlignItems", "primaryAxisSizingMode", "counterAxisSizingMode",
    "layoutAlign", "layoutGrow", "layoutPositioning", "layoutSizingHorizontal", "layoutSizingVertical",
    "fill",
    "fills",
    "stroke",
    "strokes",
    "effects",
    "layout",
    "textAlignHorizontal",
    "textAlignVertical",
    "borderRadius",
    "opacity",
    "blendMode",
    "padding",
    "itemSpacing",
];

/// Never moved to `defaults`: identity, geometry, text and hierarchy stay on every layer.
const INLINE_FIELDS: &[&str] = &["id", "name", "type", "parentId", "x", "y", "width", "height", "content", "childIds", "childCount", "children", "styleRef"];

/// Layers are objects with string id, name and type; paints and variable aliases are not.
fn is_layer(node: &Map<String, Value>) -> bool {
    ["id", "name", "type"].iter().all(|key| node.get(*key).is_some_and(Value::is_string))
}

/// Picks, per key present on every layer, its most common value when storing it once saves bytes.
fn layer_defaults(data: &mut Value) -> Map<String, Value> {
    let mut layers = 0;
    let mut seen = BTreeMap::<String, (usize, BTreeMap<String, usize>)>::new();
    visit_nodes(data, &mut |node| {
        if !is_layer(node) { return; }
        layers += 1;
        for (key, value) in node.iter().filter(|(key, _)| !INLINE_FIELDS.contains(&key.as_str())) {
            let entry = seen.entry(key.clone()).or_default();
            entry.0 += 1;
            *entry.1.entry(value.to_string()).or_default() += 1;
        }
    });
    seen.into_iter()
        .filter(|(_, (present, _))| *present == layers)
        .filter_map(|(key, (_, values))| {
            let (value, count) = values.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
            let entry = key.len() + value.len() + 4;
            ((count - 1) * entry > entry).then(|| (key, serde_json::from_str(&value).expect("serialized value")))
        })
        .collect()
}

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
    let mut collision = false;
    visit_nodes(&mut result, &mut |node| collision |= node.contains_key("styleRef"));
    if collision {
        return data.clone();
    }
    let defaults = layer_defaults(&mut result);
    visit_nodes(&mut result, &mut |node| {
        if is_layer(node) {
            node.retain(|key, value| defaults.get(key) != Some(value));
        }
    });
    let mut counts = BTreeMap::<String, usize>::new();
    visit_nodes(&mut result, &mut |node| {
        let style = style_bundle(node);
        if !style.is_empty() {
            *counts.entry(Value::Object(style).to_string()).or_default() += 1;
        }
    });

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
    if styles.is_empty() && defaults.is_empty() {
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
        "version": 2,
        "styles": styles,
        "defaults": defaults,
        "decode": "For each layer (object with id, name and type): merge _compression.styles[styleRef] and remove styleRef, then add every _compression.defaults key the layer lacks. Remove root _compression. All other data is unchanged."
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
                if is_layer(node) {
                    for (key, value) in metadata["defaults"].as_object().unwrap() {
                        node.entry(key.clone()).or_insert_with(|| value.clone());
                    }
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
    fn flat_read_nodes_prune_shared_defaults_losslessly() {
        let nodes: Vec<Value> = (0..40).map(|i| json!({
            "id": format!("1:{i}"), "name": if i % 2 == 0 { "Headline" } else { "Body" }, "type": "TEXT", "parentId": "1:0",
            "x": 0, "y": i * 20, "width": 135, "height": 18, "content": format!("Line {i}"),
            "blendMode": "PASS_THROUGH", "strokes": [], "effectStyleId": "", "visible": true, "opacity": if i == 3 { 0.5 } else { 1.0 },
            "constraints": {"horizontal": "MIN", "vertical": "MIN"},
            "paintData": [{"type": "SOLID", "color": "#111111", "opacity": 1, "visible": true, "boundVariables": {"color": {"id": "VariableID:4:2626", "type": "VARIABLE_ALIAS"}}}],
            "fontSize": if i == 5 { 20 } else { 14 }
        })).collect();
        let raw = json!({"schemaVersion": 4, "nodes": nodes, "complete": true});
        let compact = compress_tree(&raw, true);
        let meta = &compact["_compression"];
        assert_eq!(meta["defaults"]["blendMode"], "PASS_THROUGH");
        assert_eq!(meta["defaults"]["opacity"], 1.0);
        assert!(meta["defaults"].get("name").is_none() && meta["defaults"].get("width").is_none());
        let node3 = &compact["nodes"][3];
        assert_eq!(node3["opacity"], 0.5);
        assert_eq!(node3["name"], "Body");
        assert!(compact.to_string().len() * 2 < raw.to_string().len());
        assert_eq!(restore(compact), raw);
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

    #[test]
    fn typography_remains_inline_when_other_styles_are_shared() {
        let raw = json!({"tree": {"type":"FRAME", "children": (0..50).map(|i| json!({
            "type":"TEXT", "id":format!("1:{i}"), "content":"Label",
            "fontSize":15.5, "fontWeight":"Semi Bold", "fontFamily":"Inter",
            "typography":{"fontSize":15.5,"fontWeight":600},
            "fill":"#ffffff", "layout":{"padding":"16px 16px 16px 16px", "display":"flex"}
        })).collect::<Vec<_>>()}});
        let compact = compress_tree(&raw, true);
        assert!(compact.get("_compression").is_some());
        for node in compact["tree"]["children"].as_array().unwrap() {
            assert_eq!(node["fontSize"], 15.5);
            assert_eq!(node["fontWeight"], "Semi Bold");
            assert_eq!(node["typography"]["fontWeight"], 600);
        }
        assert_eq!(restore(compact), raw);
    }
}
