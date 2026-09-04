use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ── Index Entry Types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexNode {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characters: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_spacing: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fills: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strokes: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_style: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_data: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexComponent {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStyle {
    pub id: String,
    pub name: String,
    pub style_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexVariable {
    pub id: String,
    pub name: String,
    pub resolved_type: String,
    pub collection_name: String,
    pub values: HashMap<String, Value>,
}

// ── Main Index ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexStats {
    pub total_nodes: usize,
    pub total_components: usize,
    pub total_styles: usize,
    pub total_variables: usize,
    pub indexed_at_ms: u64,
    pub duration_ms: u64,
    pub file_name: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct FigmaIndex {
    pub nodes: HashMap<String, IndexNode>,
    pub components: Vec<IndexComponent>,
    pub styles: Vec<IndexStyle>,
    pub variables: Vec<IndexVariable>,
    pub top_level_frames: Vec<String>,
    pub stats: IndexStats,
    pub dirty: bool,
}

impl FigmaIndex {
    pub fn is_ready(&self) -> bool {
        self.stats.indexed_at_ms > 0 && !self.dirty
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn from_raw(
        session_id: &str,
        file_name: &str,
        page_nodes: &Value,
        styles_data: Option<&Value>,
        vars_data: Option<&Value>,
        comps_data: Option<&Value>,
        start_ms: u64,
    ) -> Self {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut idx = FigmaIndex {
            stats: IndexStats {
                indexed_at_ms: now_ms,
                duration_ms: now_ms.saturating_sub(start_ms),
                file_name: file_name.to_string(),
                session_id: session_id.to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        // Index page-level nodes
        let nodes_arr = page_nodes
            .as_array()
            .map(|a| a.as_slice())
            .or_else(|| page_nodes.get("nodes").and_then(|v| v.as_array()).map(|a| a.as_slice()))
            .unwrap_or(&[]);

        for node in nodes_arr {
            let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if id.is_empty() { continue; }
            idx.top_level_frames.push(id.clone());
            idx.ingest_node(node, None);
        }

        if let Some(styles) = styles_data { idx.ingest_styles(styles); }
        if let Some(vars) = vars_data { idx.ingest_variables(vars); }
        if let Some(comps) = comps_data { idx.ingest_components(comps); }

        idx.stats.total_nodes = idx.nodes.len();
        idx.stats.total_components = idx.components.len();
        idx.stats.total_styles = idx.styles.len();
        idx.stats.total_variables = idx.variables.len();
        idx
    }

    pub fn merge_chunk(&mut self, nodes: &[Value]) {
        for node in nodes {
            let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !id.is_empty() && !self.top_level_frames.contains(&id) {
                self.top_level_frames.push(id.clone());
            }
            self.ingest_node(node, None);
        }
        self.stats.total_nodes = self.nodes.len();
        self.dirty = false;
    }

    fn ingest_node(&mut self, node: &Value, parent_id: Option<&str>) {
        let id = match node.get("id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return,
        };

        let children_ids: Vec<String> = node
            .get("children")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|c| c.get("id").and_then(|v| v.as_str())).map(|s| s.to_string()).collect())
            .unwrap_or_default();

        let entry = IndexNode {
            id: id.clone(),
            name: node.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            node_type: node.get("type").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            width: node.get("width").and_then(|v| v.as_f64())
                .or_else(|| node.get("absoluteBoundingBox").and_then(|b| b.get("width")).and_then(|v| v.as_f64())),
            height: node.get("height").and_then(|v| v.as_f64())
                .or_else(|| node.get("absoluteBoundingBox").and_then(|b| b.get("height")).and_then(|v| v.as_f64())),
            x: node.get("x").and_then(|v| v.as_f64()),
            y: node.get("y").and_then(|v| v.as_f64()),
            characters: node.get("characters").and_then(|v| v.as_str()).map(|s| s.to_string()),
            visible: node.get("visible").and_then(|v| v.as_bool()),
            layout_mode: node.get("layoutMode").and_then(|v| v.as_str()).map(|s| s.to_string()),
            item_spacing: node.get("itemSpacing").and_then(|v| v.as_f64()),
            padding: node.get("padding").cloned()
                .or_else(|| {
                    let top = node.get("paddingTop").and_then(|v| v.as_f64());
                    let right = node.get("paddingRight").and_then(|v| v.as_f64());
                    let bottom = node.get("paddingBottom").and_then(|v| v.as_f64());
                    let left = node.get("paddingLeft").and_then(|v| v.as_f64());
                    if top.is_some() || right.is_some() || bottom.is_some() || left.is_some() {
                        Some(serde_json::json!({
                            "top": top.unwrap_or(0.0),
                            "right": right.unwrap_or(0.0),
                            "bottom": bottom.unwrap_or(0.0),
                            "left": left.unwrap_or(0.0)
                        }))
                    } else {
                        None
                    }
                }),
            fills: node.get("fills").cloned(),
            strokes: node.get("strokes").cloned(),
            border_radius: node.get("borderRadius").cloned()
                .or_else(|| node.get("cornerRadius").cloned()),
            effects: node.get("effects").cloned(),
            text_style: node.get("textStyle").cloned(),
            full_data: Some(node.clone()),
            children: children_ids,
        };

        self.nodes.insert(id.clone(), entry);
        if self.nodes.len() >= 50_000 { return; }

        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for child in children { self.ingest_node(child, Some(&id)); }
        }
    }

    pub fn upsert_node(&mut self, node: &Value) {
        let id = match node.get("id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return,
        };

        let children_ids: Vec<String> = node
            .get("children")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|c| c.get("id").and_then(|v| v.as_str())).map(|s| s.to_string()).collect())
            .unwrap_or_default();

        let parent_id = node.get("parent").and_then(|p| p.get("id")).and_then(|v| v.as_str()).map(|s| s.to_string())
            .or_else(|| self.nodes.get(&id).and_then(|existing| existing.parent_id.clone()));

        let entry = IndexNode {
            id: id.clone(),
            name: node.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            node_type: node.get("type").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string(),
            parent_id,
            width: node.get("width").and_then(|v| v.as_f64())
                .or_else(|| node.get("absoluteBoundingBox").and_then(|b| b.get("width")).and_then(|v| v.as_f64())),
            height: node.get("height").and_then(|v| v.as_f64())
                .or_else(|| node.get("absoluteBoundingBox").and_then(|b| b.get("height")).and_then(|v| v.as_f64())),
            x: node.get("x").and_then(|v| v.as_f64()),
            y: node.get("y").and_then(|v| v.as_f64()),
            characters: node.get("characters").and_then(|v| v.as_str()).map(|s| s.to_string()),
            visible: node.get("visible").and_then(|v| v.as_bool()),
            layout_mode: node.get("layoutMode").and_then(|v| v.as_str()).map(|s| s.to_string()),
            item_spacing: node.get("itemSpacing").and_then(|v| v.as_f64()),
            padding: node.get("padding").cloned()
                .or_else(|| {
                    let top = node.get("paddingTop").and_then(|v| v.as_f64());
                    let right = node.get("paddingRight").and_then(|v| v.as_f64());
                    let bottom = node.get("paddingBottom").and_then(|v| v.as_f64());
                    let left = node.get("paddingLeft").and_then(|v| v.as_f64());
                    if top.is_some() || right.is_some() || bottom.is_some() || left.is_some() {
                        Some(serde_json::json!({
                            "top": top.unwrap_or(0.0),
                            "right": right.unwrap_or(0.0),
                            "bottom": bottom.unwrap_or(0.0),
                            "left": left.unwrap_or(0.0)
                        }))
                    } else {
                        None
                    }
                }),
            fills: node.get("fills").cloned(),
            strokes: node.get("strokes").cloned(),
            border_radius: node.get("borderRadius").cloned()
                .or_else(|| node.get("cornerRadius").cloned()),
            effects: node.get("effects").cloned(),
            text_style: node.get("textStyle").cloned(),
            full_data: Some(node.clone()),
            children: if children_ids.is_empty() {
                self.nodes.get(&id).map(|e| e.children.clone()).unwrap_or_default()
            } else {
                children_ids
            },
        };

        self.nodes.insert(id, entry);
        self.dirty = false;
    }

    pub fn apply_delta(&mut self, node_id: &str, delta: &Value) {
        if let Some(existing) = self.nodes.get_mut(node_id) {
            if let Some(name) = delta.get("name").and_then(|v| v.as_str()) {
                existing.name = name.to_string();
            }
            if let Some(characters) = delta.get("characters").and_then(|v| v.as_str()) {
                existing.characters = Some(characters.to_string());
            }
            if let Some(visible) = delta.get("visible").and_then(|v| v.as_bool()) {
                existing.visible = Some(visible);
            }
            if let Some(w) = delta.get("width").and_then(|v| v.as_f64()) {
                existing.width = Some(w);
            }
            if let Some(h) = delta.get("height").and_then(|v| v.as_f64()) {
                existing.height = Some(h);
            }
            if let Some(x) = delta.get("x").and_then(|v| v.as_f64()) {
                existing.x = Some(x);
            }
            if let Some(y) = delta.get("y").and_then(|v| v.as_f64()) {
                existing.y = Some(y);
            }
            if let Some(fills) = delta.get("fills") {
                existing.fills = Some(fills.clone());
            }
            if let Some(strokes) = delta.get("strokes") {
                existing.strokes = Some(strokes.clone());
            }
            if let Some(effects) = delta.get("effects") {
                existing.effects = Some(effects.clone());
            }
            if let Some(radius) = delta.get("borderRadius").or_else(|| delta.get("cornerRadius")) {
                existing.border_radius = Some(radius.clone());
            }
            self.dirty = false;
        }
    }

    fn ingest_styles(&mut self, data: &Value) {
        let mut push = |arr: &[Value], style_type: &str| {
            for s in arr {
                let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if id.is_empty() { continue; }
                self.styles.push(IndexStyle {
                    id,
                    name: s.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    style_type: style_type.to_string(),
                    hex: s.get("hex").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    font_family: s.get("fontFamily").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    font_size: s.get("fontSize").and_then(|v| v.as_f64()),
                    font_weight: s.get("fontWeight").and_then(|v| v.as_str()).map(|s| s.to_string()),
                });
            }
        };
        if let Some(a) = data.get("paintStyles").and_then(|v| v.as_array()) { push(a, "PAINT"); }
        if let Some(a) = data.get("textStyles").and_then(|v| v.as_array()) { push(a, "TEXT"); }
        if let Some(a) = data.get("effectStyles").and_then(|v| v.as_array()) { push(a, "EFFECT"); }
    }

    fn ingest_variables(&mut self, data: &Value) {
        if let Some(collections) = data.get("collections").and_then(|v| v.as_array()) {
            for col in collections {
                let col_name = col.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let modes: Vec<String> = col.get("modes")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|m| m.get("name").and_then(|v| v.as_str())).map(|s| s.to_string()).collect())
                    .unwrap_or_default();

                if let Some(vars) = col.get("variables").and_then(|v| v.as_array()) {
                    for var in vars {
                        let id = var.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if id.is_empty() { continue; }
                        let mut values = HashMap::new();
                        if let Some(vals_obj) = var.get("valuesByMode").and_then(|v| v.as_object()) {
                            for (i, (_, val)) in vals_obj.iter().enumerate() {
                                let mode_name = modes.get(i).cloned().unwrap_or_else(|| format!("mode_{}", i));
                                values.insert(mode_name, val.clone());
                            }
                        }
                        self.variables.push(IndexVariable {
                            id,
                            name: var.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            resolved_type: var.get("resolvedType").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            collection_name: col_name.clone(),
                            values,
                        });
                    }
                }
            }
        }
    }

    fn ingest_components(&mut self, data: &Value) {
        let mut push = |c: &Value, set_name: Option<&str>| {
            let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if id.is_empty() { return; }
            self.components.push(IndexComponent {
                id,
                name: c.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                description: c.get("description").and_then(|v| v.as_str()).map(|s| s.to_string()),
                set_name: set_name.map(|s| s.to_string()),
                variant_label: c.get("variantLabel").and_then(|v| v.as_str()).map(|s| s.to_string()),
                width: c.get("width").and_then(|v| v.as_f64()),
                height: c.get("height").and_then(|v| v.as_f64()),
            });
        };
        if let Some(comps) = data.get("components").and_then(|v| v.as_array()) {
            for c in comps { push(c, None); }
        }
        if let Some(sets) = data.get("componentSets").and_then(|v| v.as_array()) {
            for set in sets {
                let sn = set.get("name").and_then(|v| v.as_str());
                if let Some(members) = set.get("variants").and_then(|v| v.as_array()) {
                    for m in members { push(m, sn); }
                }
            }
        }
    }

    // ── Query Methods ──────────────────────────────────────────────────────────

    pub fn search_nodes(&self, query: &str, node_type: Option<&str>, limit: usize) -> Vec<&IndexNode> {
        let q = query.to_lowercase();
        self.nodes.values()
            .filter(|n| {
                if let Some(t) = node_type { if !n.node_type.eq_ignore_ascii_case(t) { return false; } }
                q.is_empty()
                    || n.name.to_lowercase().contains(&q)
                    || n.characters.as_deref().map(|c| c.to_lowercase().contains(&q)).unwrap_or(false)
            })
            .take(limit)
            .collect()
    }

    pub fn get_node(&self, id: &str) -> Option<&IndexNode> { self.nodes.get(id) }

    pub fn get_node_by_name(&self, name: &str) -> Option<&IndexNode> {
        let q = name.to_lowercase();
        self.nodes.values().find(|n| n.name.to_lowercase() == q)
    }

    pub fn search_components(&self, name: &str, limit: usize) -> Vec<&IndexComponent> {
        let q = name.to_lowercase();
        self.components.iter()
            .filter(|c| name.is_empty()
                || c.name.to_lowercase().contains(&q)
                || c.set_name.as_deref().map(|s| s.to_lowercase().contains(&q)).unwrap_or(false))
            .take(limit)
            .collect()
    }

    pub fn search_styles(&self, name: &str, style_type: Option<&str>) -> Vec<&IndexStyle> {
        let q = name.to_lowercase();
        self.styles.iter()
            .filter(|s| {
                if let Some(t) = style_type { if !s.style_type.eq_ignore_ascii_case(t) { return false; } }
                name.is_empty() || s.name.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn search_variables(&self, name: &str, collection: Option<&str>) -> Vec<&IndexVariable> {
        let q = name.to_lowercase();
        self.variables.iter()
            .filter(|v| {
                if let Some(col) = collection {
                    if !v.collection_name.to_lowercase().contains(&col.to_lowercase()) { return false; }
                }
                name.is_empty() || v.name.to_lowercase().contains(&q)
            })
            .collect()
    }
}

impl IndexNode {
    /// Generates a clean, token-efficient, complete CSS & Layout specification
    pub fn to_css_spec(&self) -> serde_json::Value {
        let mut css = HashMap::new();

        // Dimensions
        if let (Some(w), Some(h)) = (self.width, self.height) {
            css.insert("width", format!("{}px", w.round()));
            css.insert("height", format!("{}px", h.round()));
        }

        // Layout / Flexbox
        if let Some(ref lm) = self.layout_mode {
            if lm == "HORIZONTAL" || lm == "VERTICAL" {
                css.insert("display", "flex".to_string());
                css.insert(
                    "flex-direction",
                    if lm == "HORIZONTAL" { "row".to_string() } else { "column".to_string() },
                );
                if let Some(gap) = self.item_spacing {
                    if gap > 0.0 {
                        css.insert("gap", format!("{}px", gap.round()));
                    }
                }
                if let Some(ref p) = self.padding {
                    if let (Some(t), Some(r), Some(b), Some(l)) = (
                        p.get("top").and_then(|v| v.as_f64()),
                        p.get("right").and_then(|v| v.as_f64()),
                        p.get("bottom").and_then(|v| v.as_f64()),
                        p.get("left").and_then(|v| v.as_f64()),
                    ) {
                        if t > 0.0 || r > 0.0 || b > 0.0 || l > 0.0 {
                            css.insert("padding", format!("{}px {}px {}px {}px", t.round(), r.round(), b.round(), l.round()));
                        }
                    }
                }
            }
        }

        // Fills (Background / Color)
        if let Some(ref fills) = self.fills {
            if let Some(arr) = fills.as_array() {
                for f in arr {
                    if f.get("visible").and_then(|v| v.as_bool()).unwrap_or(true) {
                        if let Some(c) = f.get("color").and_then(|v| v.as_str()) {
                            let opacity = f.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0);
                            if opacity < 1.0 {
                                css.insert("background-color", format!("{} (opacity: {:.2})", c, opacity));
                            } else {
                                css.insert("background-color", c.to_string());
                            }
                            break;
                        }
                    }
                }
            }
        }

        // Strokes (Border)
        if let Some(ref strokes) = self.strokes {
            if let Some(arr) = strokes.as_array() {
                for s in arr {
                    if let Some(c) = s.get("color").and_then(|v| v.as_str()) {
                        let w = s.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0);
                        css.insert("border", format!("{}px solid {}", w.round(), c));
                        break;
                    }
                }
            } else if let Some(c) = strokes.get("color").and_then(|v| v.as_str()) {
                let w = strokes.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0);
                css.insert("border", format!("{}px solid {}", w.round(), c));
            }
        }

        // Border Radius
        if let Some(ref br) = self.border_radius {
            if let Some(num) = br.as_f64() {
                if num > 0.0 {
                    css.insert("border-radius", format!("{}px", num.round()));
                }
            } else if let Some(s) = br.as_str() {
                if s != "0px" {
                    css.insert("border-radius", s.to_string());
                }
            }
        }

        // Typography (Text)
        let mut typography = HashMap::new();
        if let Some(ref ts) = self.text_style {
            if let Some(ff) = ts.get("fontFamily").and_then(|v| v.as_str()) {
                typography.insert("fontFamily", ff.to_string());
            }
            if let Some(fs) = ts.get("fontSize").and_then(|v| v.as_f64()) {
                typography.insert("fontSize", format!("{}px", fs.round()));
            }
            if let Some(fw) = ts.get("fontWeight").and_then(|v| v.as_str()) {
                typography.insert("fontWeight", fw.to_string());
            }
            if let Some(lh) = ts.get("lineHeight").and_then(|v| v.as_str()) {
                typography.insert("lineHeight", lh.to_string());
            }
            if let Some(c) = ts.get("color").and_then(|v| v.as_str()) {
                typography.insert("color", c.to_string());
            }
            if let Some(tt) = ts.get("textTransform").and_then(|v| v.as_str()) {
                typography.insert("textTransform", tt.to_string());
            }
            if let Some(tc) = ts.get("textCase").and_then(|v| v.as_str()) {
                typography.insert("textCase", tc.to_string());
            }
        }

        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "type": self.node_type,
            "visible": self.visible.unwrap_or(true),
            "css": css,
            "typography": if typography.is_empty() { None } else { Some(typography) },
            "textContent": self.characters,
            "childCount": self.children.len(),
            "childrenIds": self.children
        })
    }
}
