use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

fn node_snapshot(node: &Value) -> Value {
    node.as_object().map(|obj| Value::Object(obj.iter().filter(|(key, _)| key.as_str() != "children")
        .map(|(key, value)| (key.clone(), value.clone())).collect())).unwrap_or(Value::Null)
}

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
    pub source: Value,
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
    pub source: Value,
    pub modes: Value,
    pub default_mode_id: Option<String>,
    pub values: HashMap<String, Value>,
}

// ── Main Index ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexStats {
    pub total_nodes: usize,
    #[serde(default)]
    pub nodes_truncated: bool,
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub scopes: Vec<Value>,
    pub total_components: usize,
    #[serde(default)]
    pub components_indexed: bool,
    pub total_styles: usize,
    pub total_variables: usize,
    pub indexed_at_ms: u64,
    pub duration_ms: u64,
    pub file_name: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct FigmaIndex {
    pub revision: u64,
    pub pending_nodes: std::collections::HashSet<String>,
    pub nodes: HashMap<String, IndexNode>,
    pub components: Vec<IndexComponent>,
    pub styles: Vec<IndexStyle>,
    pub variables: Vec<IndexVariable>,
    pub top_level_frames: Vec<String>,
    pub stats: IndexStats,
    pub page_id: Option<String>,
    pub dirty: bool,
    pub raw_components: Option<Value>,
    pub raw_styles: Option<Value>,
    pub raw_variables: Option<Value>,
    pub tokens_dirty: bool,
}

impl FigmaIndex {
    pub fn cache_subtree(&mut self, tree: &Value) {
        let parent = tree["id"].as_str().and_then(|id| self.nodes.get(id)).and_then(|n| n.parent_id.clone());
        self.ingest_node(tree, parent.as_deref());
        self.stats.total_nodes = self.nodes.len();
    }
    pub fn related(&self, a: &str, b: &str) -> bool {
        fn ancestor<'a>(idx: &'a FigmaIndex, root: &str, mut id: &'a str) -> bool {
            for _ in 0..idx.nodes.len() + 1 {
                if id == root { return true; }
                let Some(parent) = idx.nodes.get(id).and_then(|n| n.parent_id.as_deref()) else { return false };
                id = parent;
            }
            true // malformed parent cycle: invalidate conservatively
        }
        ancestor(self, a, b) || ancestor(self, b, a) || !self.nodes.contains_key(b)
    }

    pub fn remove_node(&mut self, id: &str) {
        if let Some(parent) = self.nodes.get(id).and_then(|n| n.parent_id.clone()) {
            if let Some(parent) = self.nodes.get_mut(&parent) { parent.children.retain(|child| child != id); }
        }
        let mut stack = vec![id.to_string()];
        let mut removed = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            self.pending_nodes.remove(&id);
            if let Some(node) = self.nodes.remove(&id) { stack.extend(node.children); }
            removed.insert(id);
        }
        self.top_level_frames.retain(|n| !removed.contains(n));
        self.stats.total_nodes = self.nodes.len();
    }

    /// Before merging a scoped rescan, drop old children that a rescanned
    /// parent no longer lists (deleted in Figma), with their subtrees.
    pub fn drop_missing_children(&mut self, scanned: &HashMap<String, IndexNode>) {
        let stale: Vec<String> = scanned.values()
            .filter(|new| new.full_data.as_ref().is_some_and(|data| data["childIds"].is_array()))
            .filter_map(|new| self.nodes.get(&new.id).map(|old| (new, old)))
            .flat_map(|(new, old)| old.children.iter()
                .filter(|child| !new.children.contains(child) && !scanned.contains_key(*child)).cloned())
            .collect();
        for id in stale { self.remove_node(&id); }
    }

    pub fn cache_components(&mut self, data: &Value) {
        self.components.clear();
        self.ingest_components(data);
        self.raw_components = Some(data.clone());
        self.stats.components_indexed = true;
        self.stats.total_components = self.components.len();
    }

    pub fn token_snapshot(&self) -> Option<(Value, Value)> {
        let (styles, variables) = (self.raw_styles.as_ref()?, self.raw_variables.as_ref()?);
        if self.stats.indexed_at_ms == 0 || self.dirty || self.tokens_dirty || styles["schemaVersion"] != 2 || variables["schemaVersion"] != 2 {
            return None;
        }
        Some((styles.clone(), variables.clone()))
    }

    pub fn is_ready(&self) -> bool {
        self.stats.indexed_at_ms > 0 && !self.dirty
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.tokens_dirty = true;
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
                components_indexed: comps_data.is_some_and(Value::is_object),
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
            if idx.nodes.len() >= 50_000 { idx.stats.nodes_truncated = true; break; }
            let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if id.is_empty() { continue; }
            let parent = node.get("parentId").and_then(Value::as_str);
            if parent.is_none() { idx.top_level_frames.push(id.clone()); }
            idx.ingest_node(node, parent);
        }

        if let Some(styles) = styles_data { idx.ingest_styles(styles); }
        if let Some(vars) = vars_data { idx.ingest_variables(vars); }
        if let Some(comps) = comps_data.filter(|v| v.is_object()) { idx.cache_components(comps); }

        idx.stats.total_nodes = idx.nodes.len();
        idx.stats.total_components = idx.components.len();
        idx.stats.total_styles = idx.styles.len();
        idx.stats.total_variables = idx.variables.len();
        idx
    }

    pub fn merge_chunk(&mut self, nodes: &[Value]) {
        for node in nodes {
            if self.nodes.len() >= 50_000 { self.stats.nodes_truncated = true; break; }
            let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let parent_id = node.get("parentId").and_then(Value::as_str)
                .or_else(|| node.get("parent").and_then(|parent| parent.get("id")).and_then(Value::as_str));
            if !id.is_empty() && parent_id.is_none() && !self.top_level_frames.contains(&id) {
                self.top_level_frames.push(id.clone());
            }
            self.ingest_node(node, None);
        }
        self.stats.total_nodes = self.nodes.len();
    }

    pub fn merge_projected_nodes(&mut self, nodes: &[Value]) {
        for node in nodes {
            let Some(id) = node["id"].as_str() else { continue };
            if !self.nodes.contains_key(id) && self.nodes.len() >= 50_000 {
                self.stats.nodes_truncated = true;
                break;
            }
            let mut merged = self.nodes.get(id).and_then(|n| n.full_data.clone()).unwrap_or_else(|| serde_json::json!({}));
            if let (Some(base), Some(fields)) = (merged.as_object_mut(), node.as_object()) { base.extend(fields.clone()); }
            self.upsert_node(&merged);
        }
    }

    pub fn record_scope(&mut self, scope: &Value, complete: bool) {
        let mut scope = scope.clone();
        scope["complete"] = serde_json::json!(complete);
        self.stats.scopes.retain(|old| old["id"] != scope["id"] || old["depth"] != scope["depth"]
            || old["includeHidden"] != scope["includeHidden"] || old["expandInstances"] != scope["expandInstances"]);
        self.stats.complete |= complete && scope["id"].as_str() == self.page_id.as_deref()
            && scope["depth"] == 256 && scope["expandInstances"] == true && !self.stats.nodes_truncated;
        self.stats.scopes.push(scope);
    }

    pub fn apply_patch_batch(&mut self, base_revision: u64, revision: u64, patches: &[Value]) -> bool {
        if revision <= self.revision { return true; }
        if base_revision != self.revision || revision <= base_revision {
            self.mark_dirty();
            return false;
        }
        for patch in patches {
            match patch["kind"].as_str() {
                Some("delete") => { if let Some(id) = patch["id"].as_str() { self.remove_node(id); } }
                Some("create") => {
                    let node = &patch["node"];
                    let parent = node["parentId"].as_str();
                    if parent.is_none() || parent.is_some_and(|id| self.nodes.contains_key(id)) {
                        self.merge_projected_nodes(std::slice::from_ref(node));
                    }
                    self.stats.complete = false;
                    for scope in &mut self.stats.scopes { scope["complete"] = serde_json::json!(false); }
                }
                Some("update") => {
                    if patch["values"].get("parentId").is_some() {
                        self.stats.complete = false;
                        for scope in &mut self.stats.scopes { scope["complete"] = serde_json::json!(false); }
                    }
                    let Some(id) = patch["id"].as_str() else { continue };
                    if self.nodes.contains_key(id) || patch["values"].get("parentId").is_some() {
                        let mut node = self.nodes.get(id).and_then(|node| node.full_data.clone()).unwrap_or_else(|| serde_json::json!({}));
                        if let Some(fields) = node.as_object_mut() {
                            fields.retain(|key, _| ["id","name","type","parentId","childIds","childCount","childrenLoaded",
                                "opaque","x","y","width","height","visible","rotation","opacity","content","indexDetail"].contains(&key.as_str()));
                            if let Some(values) = patch["values"].as_object() { fields.extend(values.clone()); }
                        }
                        node["id"] = serde_json::json!(id);
                        self.upsert_node(&node);
                    }
                }
                Some("invalidate") => { if let Some(id) = patch["id"].as_str() { self.pending_nodes.insert(id.to_string()); } }
                _ => { self.mark_dirty(); return false; }
            }
        }
        self.revision = revision;
        true
    }

    fn ingest_node(&mut self, node: &Value, parent_id: Option<&str>) {
        let id = match node.get("id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return,
        };

        let children_ids: Vec<String> = node.get("childIds").or_else(|| node.get("children"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|c| c.as_str().or_else(|| c.get("id").and_then(Value::as_str))).map(str::to_owned).collect())
            .unwrap_or_default();

        let actual_parent = parent_id.map(str::to_owned).or_else(|| {
            node.get("parentId").and_then(Value::as_str).map(str::to_owned)
                .or_else(|| node.get("parent").and_then(|parent| parent.get("id")).and_then(Value::as_str).map(str::to_owned))
        });
        let entry = IndexNode {
            id: id.clone(),
            name: node.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            node_type: node.get("type").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string(),
            parent_id: actual_parent,
            width: node.get("width").and_then(|v| v.as_f64())
                .or_else(|| node.get("absoluteBoundingBox").and_then(|b| b.get("width")).and_then(|v| v.as_f64())),
            height: node.get("height").and_then(|v| v.as_f64())
                .or_else(|| node.get("absoluteBoundingBox").and_then(|b| b.get("height")).and_then(|v| v.as_f64())),
            x: node.get("x").and_then(|v| v.as_f64()),
            y: node.get("y").and_then(|v| v.as_f64()),
            characters: node.get("characters").or_else(|| node.get("content")).and_then(|v| v.as_str()).map(|s| s.to_string()),
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
            fills: node.get("paintData").or_else(|| node.get("fills")).cloned(),
            strokes: node.get("strokes").cloned(),
            border_radius: node.get("borderRadius").cloned()
                .or_else(|| node.get("cornerRadius").cloned()),
            effects: node.get("effectData").or_else(|| node.get("effects")).cloned(),
            text_style: text_style_from_node(node),
            full_data: Some(node_snapshot(node)),
            children: children_ids,
        };

        self.nodes.insert(id.clone(), entry);
        if self.nodes.len() >= 50_000 {
            if node.get("children").and_then(Value::as_array).is_some_and(|children| !children.is_empty()) {
                self.stats.nodes_truncated = true;
            }
            return;
        }

        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for child in children { if child.get("type").is_some() { self.ingest_node(child, Some(&id)); } }
        }
    }

    pub fn upsert_node(&mut self, node: &Value) {
        let id = match node.get("id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return,
        };

        let has_child_list = node.get("childIds").or_else(|| node.get("children")).and_then(Value::as_array).is_some();
        let children_ids: Vec<String> = node.get("childIds").or_else(|| node.get("children"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|c| c.as_str().or_else(|| c.get("id").and_then(Value::as_str))).map(str::to_owned).collect())
            .unwrap_or_default();

        let parent_id = match node.get("parentId") {
            Some(Value::Null) => None,
            Some(value) => value.as_str().map(str::to_owned),
            None => node.get("parent").and_then(|p| p.get("id")).and_then(Value::as_str).map(str::to_owned)
                .or_else(|| self.nodes.get(&id).and_then(|existing| existing.parent_id.clone())),
        };

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
            characters: node.get("characters").or_else(|| node.get("content")).and_then(|v| v.as_str()).map(|s| s.to_string()),
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
            fills: node.get("paintData").or_else(|| node.get("fills")).cloned(),
            strokes: node.get("strokes").cloned(),
            border_radius: node.get("borderRadius").cloned()
                .or_else(|| node.get("cornerRadius").cloned()),
            effects: node.get("effectData").or_else(|| node.get("effects")).cloned(),
            text_style: node.get("textStyle").cloned(),
            full_data: Some(node_snapshot(node)),
            children: if !has_child_list {
                self.nodes.get(&id).map(|e| e.children.clone()).unwrap_or_default()
            } else {
                children_ids
            },
        };

        self.pending_nodes.remove(&id);
        let old_parent = self.nodes.get(&id).and_then(|n| n.parent_id.clone());
        let new_parent = entry.parent_id.clone();
        let top_level = new_parent.as_ref().is_none_or(|parent| !self.nodes.contains_key(parent));
        self.top_level_frames.retain(|child| child != &id);
        if top_level { self.top_level_frames.push(id.clone()); }
        let mut entry = entry;
        if let Some(ids) = node.get("childIds").and_then(Value::as_array) {
            entry.children = ids.iter().filter_map(Value::as_str).map(str::to_owned).collect();
        }
        self.nodes.insert(id.clone(), entry);
        if old_parent != new_parent {
            if let Some(parent) = old_parent.and_then(|id| self.nodes.get_mut(&id)) { parent.children.retain(|child| child != &id); }
        }
        if let Some(parent) = new_parent.and_then(|id| self.nodes.get_mut(&id)) {
            if !parent.children.contains(&id) { parent.children.push(id); }
        }
        self.stats.total_nodes = self.nodes.len();
    }

    pub fn apply_delta(&mut self, node_id: &str, delta: &Value) {
        // A node delta cannot establish that catalog styles/variables are fresh.
        self.tokens_dirty = true;
        if let Some(existing) = self.nodes.get_mut(node_id) {
            if let (Some(raw), Some(delta)) = (existing.full_data.as_mut().and_then(Value::as_object_mut), delta.as_object()) {
                raw.extend(delta.clone());
            }
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
        }
    }

    fn ingest_styles(&mut self, data: &Value) {
        self.raw_styles = Some(data.clone());
        let mut push = |arr: &[Value], style_type: &str| {
            for s in arr {
                let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if id.is_empty() { continue; }
                self.styles.push(IndexStyle {
                    id,
                    name: s.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    style_type: style_type.to_string(),
                    source: s.clone(),
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
        self.raw_variables = Some(data.clone());
        if let Some(collections) = data.get("collections").and_then(|v| v.as_array()) {
            for col in collections {
                let col_name = col.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let modes = col.get("modes").and_then(Value::as_array);

                if let Some(vars) = col.get("variables").and_then(|v| v.as_array()) {
                    for var in vars {
                        let id = var.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if id.is_empty() { continue; }
                        let mut values = HashMap::new();
                        if let Some(vals_obj) = var.get("values").or_else(|| var.get("valuesByMode")).and_then(|v| v.as_object()) {
                            for (mode_id, val) in vals_obj {
                                let mode_name = modes.and_then(|ms| ms.iter().find(|m| m["id"].as_str() == Some(mode_id)))
                                    .and_then(|m| m["name"].as_str()).unwrap_or(mode_id).to_string();
                                values.insert(mode_name, val.clone());
                            }
                        }
                        self.variables.push(IndexVariable {
                            id,
                            name: var.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            resolved_type: var.get("resolvedType").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            collection_name: col_name.clone(),
                            source: var.clone(),
                            modes: col["modes"].clone(),
                            default_mode_id: col["defaultModeId"].as_str().map(str::to_owned),
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
        let mut matches: Vec<&IndexNode> = self.nodes.values()
            .filter(|n| {
                if self.pending_nodes.contains(&n.id) { return false; }
                if let Some(t) = node_type { if !n.node_type.eq_ignore_ascii_case(t) { return false; } }
                q.is_empty()
                    || n.name.to_lowercase().contains(&q)
                    || n.characters.as_deref().map(|c| c.to_lowercase().contains(&q)).unwrap_or(false)
            })
            .collect();
        // HashMap order is random; sort so repeated searches return the same page.
        matches.sort_unstable_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        matches.truncate(limit);
        matches
    }

    pub fn get_node(&self, id: &str) -> Option<&IndexNode> { self.nodes.get(id).filter(|_| !self.pending_nodes.contains(id)) }

    pub fn get_node_by_name(&self, name: &str) -> Option<&IndexNode> {
        let q = name.to_lowercase();
        self.nodes.values().find(|n| !self.pending_nodes.contains(&n.id) && n.name.to_lowercase() == q)
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

fn text_style_from_node(node: &Value) -> Option<Value> {
    let mut style = node.get("textStyle").and_then(Value::as_object).cloned().unwrap_or_default();
    for key in ["fontFamily", "fontSize", "fontWeight", "lineHeight", "letterSpacing", "textTransform", "textCase"] {
        if !style.contains_key(key) {
            if let Some(value) = node.get(key) { style.insert(key.to_string(), value.clone()); }
        }
    }
    if !style.contains_key("color") {
        if let Some(color) = node.get("color").or_else(|| node.get("fill")) {
            style.insert("color".to_string(), color.clone());
        }
    }
    (!style.is_empty()).then_some(Value::Object(style))
}

impl IndexNode {
    /// Small search-result projection: no full_data or paint/effect payloads.
    pub fn search_summary(&self) -> Value {
        let mut out = serde_json::json!({"id": self.id, "name": self.name, "type": self.node_type,
            "parentId": self.parent_id, "width": self.width, "height": self.height});
        if let Some(text) = &self.characters {
            out["characters"] = match text.char_indices().nth(200) {
                Some((cut, _)) => Value::String(format!("{}…", &text[..cut])),
                None => Value::String(text.clone()),
            };
        }
        if let Some(name) = self.full_data.as_ref().and_then(|d| d.get("componentName").or_else(|| d.get("mainComponentName"))) {
            out["componentName"] = name.clone();
        }
        out
    }

    pub fn has_details(&self) -> bool {
        self.full_data.as_ref().is_none_or(|data| data["indexDetail"] != "minimal")
    }

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

        let mut diagnostics = Vec::new();
        if let Some(paints) = self.fills.as_ref().and_then(Value::as_array) {
            match crate::mcp::tokens::background_css(paints, self.width.unwrap_or(0.0), self.height.unwrap_or(0.0)) {
                Ok(Some((property, value))) => { css.insert(property, value); }
                Ok(None) => {},
                Err(error) => diagnostics.push(error),
            }
        }
        if let Some(effects) = self.effects.as_ref().and_then(Value::as_array) {
            match crate::mcp::tokens::effect_css(effects) {
                Ok(properties) => { for (key, value) in properties { css.insert(match key.as_str() { "box-shadow" => "box-shadow", "filter" => "filter", _ => "backdrop-filter" }, value); } }
                Err(error) => diagnostics.push(error),
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
                typography.insert("fontSize", format!("{fs}px"));
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
            "fills": self.fills,
            "effects": self.effects,
            "diagnostics": diagnostics,
            "width": self.width,
            "height": self.height,
            "fill": self.full_data.as_ref().and_then(|v| v.get("fill")),
            "typography": if typography.is_empty() { None } else { Some(typography) },
            "fontSize": self.full_data.as_ref().and_then(|v| v.get("fontSize")),
            "fontWeight": self.full_data.as_ref().and_then(|v| v.get("fontWeight")),
            "fontFamily": self.full_data.as_ref().and_then(|v| v.get("fontFamily")),
            "segments": self.full_data.as_ref().and_then(|v| v.get("segments")),
            "textContent": self.characters,
            "childCount": self.children.len(),
            "childrenIds": self.children
    })
    }
}

#[cfg(test)]
mod tests {
    use super::FigmaIndex;
    use serde_json::json;

    #[test]
    fn incremental_move_delete_and_typography_keep_tree_consistent() {
        let mut idx = FigmaIndex::from_raw("s","f",&json!([
            {"id":"a","type":"FRAME","children":[{"id":"t","type":"TEXT","content":"Old","fontSize":14,"fontWeight":"Regular"}]},
            {"id":"b","type":"FRAME","children":[]}
        ]),None,None,None,0);
        idx.pending_nodes.insert("t".into());
        assert!(idx.is_ready());
        assert!(idx.get_node("t").is_none());
        assert!(idx.get_node("b").is_some());
        idx.upsert_node(&json!({"id":"t","parentId":"b","type":"TEXT","content":"New","fontSize":15.5,"fontWeight":"Semi Bold","childIds":[]}));
        assert!(idx.is_ready());
        assert!(idx.nodes["a"].children.is_empty());
        assert_eq!(idx.nodes["b"].children,vec!["t"]);
        assert_eq!(idx.nodes["t"].characters.as_deref(),Some("New"));
        assert_eq!(idx.nodes["t"].to_css_spec()["fontSize"],15.5);
        assert!(!idx.nodes["a"].full_data.as_ref().unwrap().as_object().unwrap().contains_key("children"));
        idx.remove_node("b");
        assert!(!idx.nodes.contains_key("t"));
        assert_eq!(idx.stats.total_nodes,1);
        idx.mark_dirty();
        idx.upsert_node(&json!({"id":"a","type":"FRAME"}));
        assert!(!idx.is_ready());
    }

    #[test]
    fn deferred_components_are_not_a_complete_empty_catalogue() {
        let nodes = json!([]);
        let deferred = FigmaIndex::from_raw("s", "f", &nodes, None, None, Some(&json!(null)), 0);
        assert!(!deferred.stats.components_indexed);
        let complete = FigmaIndex::from_raw("s", "f", &nodes, None, None,
            Some(&json!({"components": [], "componentSets": []})), 0);
        assert!(complete.stats.components_indexed);
        let mut cached = deferred;
        let catalogue = json!({"components": [{"id": "1:1", "name": "Button", "properties": {"size": "small"}}], "componentSets": [], "total": 1});
        cached.cache_components(&catalogue);
        cached.cache_components(&catalogue);
        assert!(cached.stats.components_indexed);
        assert_eq!(cached.raw_components, Some(catalogue));
        assert_eq!(cached.stats.total_components, 1);

    }

    #[test]
    fn indexes_nested_nodes_and_searches_text() {
        let page_nodes = json!([{
            "id": "1:1",
            "name": "Card",
            "type": "FRAME",
            "children": [{
                "id": "1:2",
                "name": "Title",
                "type": "TEXT",
                "characters": "Welcome home",
                "width": 120.0,
                "height": 24.0
            }]
        }]);

        let index = FigmaIndex::from_raw("session", "file", &page_nodes, None, None, None, 0);
        assert_eq!(index.stats.total_nodes, 2);
        assert_eq!(index.top_level_frames, vec!["1:1"]);
        assert_eq!(index.search_nodes("welcome", Some("TEXT"), 10).len(), 1);
        assert_eq!(index.get_node("1:2").and_then(|n| n.parent_id.as_deref()), Some("1:1"));
    }

    #[test]
    fn streamed_flat_nodes_index_descendants_and_direct_typography() {
        let mut index = FigmaIndex { page_id: Some("page-1".into()), dirty: true, ..Default::default() };
        index.merge_chunk(&[
            json!({"id":"root","name":"Frame","type":"FRAME","parentId":null,"childIds":["child"]}),
            json!({"id":"child","name":"Label","type":"TEXT","parentId":"root","childIds":[],"content":"Searchable","textStyle":{"fontFamily":"Inter"},"fontSize":15.5,"fontWeight":"Demi"}),
        ]);

        assert_eq!(index.stats.total_nodes, 2);
        assert_eq!(index.top_level_frames, vec!["root"]);
        assert_eq!(index.get_node("child").and_then(|node| node.parent_id.as_deref()), Some("root"));
        assert_eq!(index.search_nodes("searchable", Some("TEXT"), 10).len(), 1);
        let spec = index.get_node("child").unwrap().to_css_spec();
        let typography = &spec["typography"];
        assert_eq!(typography["fontFamily"], "Inter");
        assert_eq!(typography["fontSize"], "15.5px");
        assert_eq!(typography["fontWeight"], "Demi");
        assert!(index.dirty);
    }

    #[test]
    fn lightweight_nodes_stay_searchable_but_need_live_details() {
        let mut index = FigmaIndex::default();
        index.merge_chunk(&[json!({"id":"label", "type":"TEXT", "content":"Searchable",
            "indexDetail":"minimal", "width":15.5, "childIds":[]})]);
        assert_eq!(index.search_nodes("searchable", Some("TEXT"), 10).len(), 1);
        assert!(!index.get_node("label").unwrap().has_details());
        index.apply_delta("label", &json!({"width":20}));
        assert!(!index.get_node("label").unwrap().has_details());
        index.upsert_node(&json!({"id":"label", "type":"TEXT", "fontSize":15.5}));
        assert!(index.get_node("label").unwrap().has_details());
    }

    #[test]
    fn revisioned_patches_move_delete_and_drop_stale_styles() {
        let mut index = FigmaIndex::from_raw("s", "f", &json!([
            {"id":"a","type":"FRAME","children":[{"id":"t","type":"TEXT","content":"old","fontSize":14,"paintData":[]}]},
            {"id":"b","type":"FRAME"}]), None, None, None, 0);
        assert!(index.apply_patch_batch(0, 1, &[json!({"kind":"update","id":"t",
            "values":{"parentId":"b","content":"new","width":15.5,"indexDetail":"minimal"}})]));
        assert_eq!(index.nodes["t"].parent_id.as_deref(), Some("b"));
        assert_eq!(index.nodes["t"].characters.as_deref(), Some("new"));
        assert!(index.nodes["a"].children.is_empty());
        assert_eq!(index.nodes["b"].children, vec!["t"]);
        assert!(index.nodes["t"].full_data.as_ref().unwrap().get("fontSize").is_none());
        assert!(index.apply_patch_batch(0, 1, &[]));
        assert!(index.apply_patch_batch(1, 2, &[json!({"kind":"delete","id":"b"})]));
        assert!(!index.nodes.contains_key("t"));
        assert!(!index.apply_patch_batch(3, 4, &[]));
        assert!(!index.is_ready());
    }

    #[test]
    fn scoped_rescan_drops_deleted_descendants_and_search_is_sorted() {
        let mut idx = FigmaIndex::from_raw("s","f",&json!([
            {"id":"a","type":"FRAME","children":[
                {"id":"keep","name":"B","type":"TEXT","content":"x".repeat(300)},
                {"id":"gone","name":"A","type":"FRAME","children":[{"id":"deep","type":"TEXT"}]}]}]),None,None,None,0);
        let mut scan = FigmaIndex::default();
        scan.merge_chunk(&[json!({"id":"a","type":"FRAME","parentId":null,"childIds":["keep"]}),
            json!({"id":"keep","name":"B","type":"TEXT","parentId":"a"})]);
        idx.drop_missing_children(&scan.nodes);
        assert!(!idx.nodes.contains_key("gone") && !idx.nodes.contains_key("deep"));
        assert_eq!(idx.nodes["a"].children, vec!["keep"]);
        for n in 0..5 { idx.upsert_node(&json!({"id":format!("n{n}"),"name":format!("Z{}", 4 - n),"type":"FRAME","parentId":"a"})); }
        let names: Vec<_> = idx.search_nodes("z", Some("FRAME"), 3).iter().map(|n| n.name.clone()).collect();
        assert_eq!(names, ["Z0","Z1","Z2"]);
        let summary = idx.nodes["keep"].search_summary();
        assert_eq!(summary["characters"].as_str().unwrap().chars().count(), 201);
        assert!(summary.get("fills").is_none() && summary.get("full_data").is_none());
    }

    #[test]
    fn upsert_and_merge_chunk_keep_index_consistent() {
        let mut index = FigmaIndex::default();
        index.merge_chunk(&[json!({
            "id": "2:1",
            "name": "Old",
            "type": "FRAME"
        })]);
        assert!(!index.dirty);

        index.upsert_node(&json!({
            "id": "2:1",
            "name": "Renamed",
            "type": "FRAME"
        }));
        assert_eq!(index.get_node("2:1").map(|n| n.name.as_str()), Some("Renamed"));
        assert_eq!(index.stats.total_nodes, 1);
    }
}

#[cfg(test)]
mod token_cache_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn live_and_cached_tokens_are_identical_and_mode_ids_are_not_positional() {
        let fixture: Value = serde_json::from_str(include_str!("../../tests/fixtures/tokens-v2.json")).unwrap();
        let styles = &fixture["styles"]; let variables = &fixture["variables"];
        let mut idx = FigmaIndex::from_raw("session", "file", &json!([]), Some(styles), Some(variables), None, 0);
        let (cached_styles, cached_variables) = idx.token_snapshot().unwrap();
        assert_eq!(cached_styles, *styles);
        assert_eq!(cached_variables, *variables);
        assert_eq!(idx.variables[0].values["Light"], "rgb(100% 100% 100% / 50%)");
        assert_eq!(idx.variables[0].values["Dark"], "#0008");
        for format in ["css", "tailwind", "typescript", "json", "w3c"] {
            assert_eq!(crate::mcp::tokens::generate_tokens(styles, variables, format, None, None, None).unwrap(),
                crate::mcp::tokens::generate_tokens(&cached_styles, &cached_variables, format, None, None, None).unwrap());
        }
        idx.mark_dirty();
        idx.apply_delta("missing", &json!({"name":"changed"}));
        assert!(idx.token_snapshot().is_none());
        let legacy = FigmaIndex::from_raw("session", "file", &json!([]), Some(&json!({"paintStyles":[]})), Some(variables), None, 0);
        assert!(legacy.token_snapshot().is_none());
    }
}
