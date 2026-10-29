//! `get_spec mode:"outline"`: one line per layer with the values an implementer
//! maps to code (type, spacing, colors, token names), instead of raw records.
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;

const SHAPES: [&str; 6] = ["VECTOR", "BOOLEAN_OPERATION", "STAR", "POLYGON", "LINE", "ELLIPSE"];
// ponytail: matched by layer name; OS mockups have no stable component key across files.
const DEVICE_CHROME: [&str; 4] = ["status bar", "keyboard", "home indicator", "dynamic island"];

struct Tree<'a> {
    nodes: HashMap<&'a str, &'a Value>,
    children: HashMap<&'a str, Vec<&'a str>>,
    names: &'a Value,
    // Block signature -> copy of its first occurrence.
    seen: RefCell<HashMap<String, Vec<String>>>,
    memo: RefCell<HashMap<String, String>>,
}

pub fn render(data: &Value) -> String {
    let empty = Vec::new();
    let list = data["nodes"].as_array().unwrap_or(&empty);
    let mut tree = Tree { nodes: HashMap::new(), children: HashMap::new(), names: &data["names"], seen: RefCell::default(), memo: RefCell::default() };
    let mut roots = Vec::new();
    for node in list {
        let Some(id) = node["id"].as_str() else { continue };
        tree.nodes.insert(id, node);
    }
    // Records arrive in pre-order, so pushing in arrival order keeps sibling order.
    for node in list {
        let Some(id) = node["id"].as_str() else { continue };
        match node["parentId"].as_str().filter(|p| tree.nodes.contains_key(p)) {
            Some(parent) => tree.children.entry(parent).or_default().push(id),
            None => roots.push(id),
        }
    }
    let mut out = String::new();
    for root in roots { tree.write(root, 0, &mut out); }
    out
}

impl Tree<'_> {
    fn kids(&self, id: &str) -> &[&str] { self.children.get(id).map(Vec::as_slice).unwrap_or_default() }

    fn only_shapes(&self, id: &str) -> bool {
        let kids = self.kids(id);
        !kids.is_empty() && kids.iter().all(|k| {
            let ty = self.nodes[k]["type"].as_str().unwrap_or("");
            SHAPES.contains(&ty) || (ty != "TEXT" && self.only_shapes(k))
        })
    }

    fn texts(&self, id: &str, acc: &mut Vec<String>) {
        let node = self.nodes[id];
        if node["type"] == "TEXT" { acc.push(quote(node["content"].as_str().unwrap_or(""))); }
        for k in self.kids(id) { self.texts(k, acc); }
    }

    /// The layer's line, and whether its children are listed under it.
    fn head(&self, id: &str, depth: usize) -> Option<(String, bool)> {
        let node = self.nodes[id];
        let ty = node["type"].as_str().unwrap_or("");
        let name = node["name"].as_str().unwrap_or("");
        if DEVICE_CHROME.iter().any(|c| name.to_lowercase().starts_with(c)) {
            return Some((format!("{ty} '{name}' {} (device chrome)", size(node)), false));
        }
        if SHAPES.contains(&ty) && depth > 0 { return None; }
        if ty != "TEXT" && self.only_shapes(id) {
            let color = self.kids(id).iter().find_map(|k| self.shape_color(k)).unwrap_or_default();
            return Some((format!("ICON '{name}' {}{color}", size(node)), false));
        }
        Some((self.line(node), true))
    }

    /// The uncollapsed block, indented from zero: a repeat has to match it in full.
    fn full(&self, id: &str) -> String {
        if let Some(block) = self.memo.borrow().get(id) { return block.clone(); }
        let mut block = String::new();
        if let Some((line, descend)) = self.head(id, 1) {
            block.push_str(&line);
            block.push('\n');
            for k in self.kids(id).iter().filter(|_| descend) {
                for l in self.full(k).lines() { block.push_str("  "); block.push_str(l); block.push('\n'); }
            }
        }
        self.memo.borrow_mut().insert(id.to_string(), block.clone());
        block
    }

    fn write(&self, id: &str, depth: usize, out: &mut String) {
        let Some((line, descend)) = self.head(id, depth) else { return };
        let indent = "  ".repeat(depth);
        out.push_str(&format!("{indent}{line}\n"));
        if !descend { return; }
        for k in self.kids(id) {
            // Blocks repeat across the tree (list items, the same card in two sections);
            // after the first, only the head line and the differing copy are kept.
            let full = self.full(k);
            let mut texts = Vec::new();
            self.texts(k, &mut texts);
            let first = if full.lines().count() < 2 { None } else {
                let mut seen = self.seen.borrow_mut();
                let key = signature(&full);
                let first = seen.get(&key).cloned();
                if first.is_none() { seen.insert(key, texts.clone()); }
                first
            };
            let Some(first) = first else {
                self.write(k, depth + 1, out);
                continue;
            };
            // Styling already printed at the first occurrence; name and size locate the layer.
            let node = self.nodes[k];
            let copy = if texts.is_empty() || texts == first { String::new() } else { format!(": {}", texts.join(" ")) };
            out.push_str(&format!("{indent}  {} '{}' {} = same layout as earlier{copy}\n",
                node["type"].as_str().unwrap_or(""), node["name"].as_str().unwrap_or(""), size(node)));
        }
    }

    fn shape_color(&self, id: &str) -> Option<String> {
        let node = self.nodes[id];
        self.paints(&node["paintData"]).or_else(|| self.paints(&node["strokes"]))
            .map(|c| format!(" {c}"))
            .or_else(|| self.kids(id).iter().find_map(|k| self.shape_color(k)))
    }

    fn token(&self, binding: &Value) -> Option<&str> {
        binding["id"].as_str().and_then(|id| self.names[id].as_str())
    }

    fn paints(&self, paints: &Value) -> Option<String> {
        let parts: Vec<String> = paints.as_array()?.iter().filter(|p| p["visible"] != false).map(|p| {
            match p["type"].as_str().unwrap_or("") {
                "SOLID" => {
                    let color = p["color"].as_str().unwrap_or("?").to_string();
                    match self.token(&p["boundVariables"]["color"]) { Some(t) => format!("{color}({t})"), None => color }
                }
                "IMAGE" => "image".into(),
                other => other.to_lowercase(),
            }
        }).collect();
        (!parts.is_empty()).then(|| parts.join("+"))
    }

    fn bound(&self, node: &Value, key: &str, value: String) -> String {
        match self.token(&node["boundVariables"][key]) { Some(t) => format!("{value}({t})"), None => value }
    }

    fn line(&self, node: &Value) -> String {
        let ty = node["type"].as_str().unwrap_or("");
        let name = node["name"].as_str().unwrap_or("");
        let mut parts = Vec::new();
        if ty == "TEXT" {
            parts.push(format!("TEXT {}", quote(node["content"].as_str().unwrap_or(""))));
            if node["mixed"] == true {
                for s in node["segments"].as_array().into_iter().flatten() {
                    let fill = s["fill"].as_str().map(|f| format!(" {f}")).unwrap_or_default();
                    parts.push(format!("[{} {}{fill}]", quote(s["text"].as_str().unwrap_or("")), font(s)));
                }
            } else {
                parts.push(font(node));
                if let Some(fill) = self.paints(&node["paintData"]).or_else(|| node["fill"].as_str().map(str::to_string)) { parts.push(fill); }
            }
            if let Some(style) = node["textStyleId"].as_str().and_then(|id| self.names[id].as_str()) { parts.push(format!("style={style}")); }
            if let Some(align) = node["textAlignHorizontal"].as_str().filter(|a| *a != "LEFT") { parts.push(format!("align={}", align.to_lowercase())); }
            if let Some(lines) = node["maxLines"].as_u64() { parts.push(format!("maxLines={lines}")); }
            if let Some(d) = node["textDecoration"].as_str() { parts.push(d.to_lowercase()); }
            if let Some(c) = node["textCase"].as_str() { parts.push(format!("case={}", c.to_lowercase())); }
        } else {
            let mut head = format!("{ty} '{name}'");
            if let Some(component) = node["componentSetName"].as_str().or_else(|| node["componentName"].as_str()) {
                if component != name { head.push_str(&format!(" <{component}>")); }
            }
            parts.push(head);
            parts.push(size(node));
            if let Some(variant) = node["variant"].as_object() {
                let v: Vec<String> = variant.iter().map(|(k, v)| format!("{k}={}", plain(v))).collect();
                if !v.is_empty() { parts.push(format!("{{{}}}", v.join(", "))); }
            }
            if let Some(props) = node["props"].as_object() {
                // Instance-swap props carry node ids, which mean nothing outside Figma.
                let p: Vec<String> = props.iter().filter(|(_, v)| !v.as_str().is_some_and(is_node_id))
                    .map(|(k, v)| format!("{k}={}", plain(v))).collect();
                if !p.is_empty() { parts.push(format!("props{{{}}}", p.join(", "))); }
            }
            parts.extend(self.layout(node));
            if let Some(fill) = self.paints(&node["paintData"]) { parts.push(format!("bg {fill}")); }
        }
        if let Some(stroke) = self.paints(&node["strokes"]) {
            let weight = num(&node["strokeWeight"]).unwrap_or_else(|| "1".into());
            let align = node["strokeAlign"].as_str().filter(|a| *a != "INSIDE").map(|a| format!(" {}", a.to_lowercase())).unwrap_or_default();
            parts.push(format!("border {weight} {stroke}{align}"));
        }
        parts.extend(self.radius(node));
        for e in node["effects"].as_array().into_iter().flatten().filter(|e| e["visible"] != false) {
            let kind = e["type"].as_str().unwrap_or("").to_lowercase();
            let offset = (num(&e["offset"]["x"]), num(&e["offset"]["y"]));
            let color = e["color"].as_object().map(|c| format!(" rgba({},{},{},{})",
                channel(&c["r"]), channel(&c["g"]), channel(&c["b"]), num(&c["a"]).unwrap_or_default())).unwrap_or_default();
            match offset {
                (Some(x), Some(y)) => parts.push(format!("{kind} {x},{y} blur{} spread{}{color}",
                    num(&e["radius"]).unwrap_or_default(), num(&e["spread"]).unwrap_or_else(|| "0".into()))),
                _ => parts.push(format!("{kind} {}", num(&e["radius"]).unwrap_or_default())),
            }
        }
        if let Some(o) = node["opacity"].as_f64().filter(|o| *o < 1.0) { parts.push(format!("opacity {}", fmt(o))); }
        if node["layoutPositioning"] == "ABSOLUTE" {
            parts.push(format!("absolute @{},{}", num(&node["x"]).unwrap_or_default(), num(&node["y"]).unwrap_or_default()));
        }
        parts.join(" ")
    }

    fn layout(&self, node: &Value) -> Vec<String> {
        let mut parts = Vec::new();
        let mode = node["layoutMode"].as_str().unwrap_or("NONE");
        if mode != "NONE" {
            parts.push(if mode == "VERTICAL" { "col".to_string() } else { "row".to_string() });
            if node["layoutWrap"] == "WRAP" { parts.push("wrap".into()); }
            if let Some(gap) = num(&node["itemSpacing"]).filter(|g| g != "0") { parts.push(self.bound(node, "itemSpacing", format!("gap{gap}"))); }
            if let Some(gap) = num(&node["counterAxisSpacing"]).filter(|g| g != "0" && node["layoutWrap"] == "WRAP") { parts.push(format!("rowGap{gap}")); }
            let pad: Vec<String> = ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"].iter()
                .map(|k| self.bound(node, k, num(&node[*k]).unwrap_or_else(|| "0".into()))).collect();
            if pad.iter().any(|p| p != "0") {
                let joined = if pad.iter().all(|p| *p == pad[0]) { pad[0].clone() }
                    else if pad[0] == pad[2] && pad[1] == pad[3] { format!("{},{}", pad[0], pad[1]) }
                    else { pad.join(",") };
                parts.push(format!("pad{joined}"));
            }
            for (key, label) in [("primaryAxisAlignItems", "main"), ("counterAxisAlignItems", "cross")] {
                if let Some(a) = node[key].as_str().filter(|a| *a != "MIN") { parts.push(format!("{label}={}", a.to_lowercase())); }
            }
        }
        for (key, axis) in [("layoutSizingHorizontal", "w"), ("layoutSizingVertical", "h")] {
            if let Some(s) = node[key].as_str().filter(|s| *s != "FIXED") { parts.push(format!("{axis}:{}", s.to_lowercase())); }
        }
        parts
    }

    fn radius(&self, node: &Value) -> Option<String> {
        if let Some(r) = num(&node["cornerRadius"]) {
            return (r != "0").then(|| self.bound(node, "topLeftRadius", format!("r{r}")));
        }
        let corners: Vec<String> = ["topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"]
            .iter().map(|k| num(&node[*k]).unwrap_or_else(|| "0".into())).collect();
        corners.iter().any(|c| c != "0").then(|| format!("r{}", corners.join(",")))
    }
}

fn font(node: &Value) -> String {
    let mut s = format!("{} {}/{}", node["fontFamily"].as_str().unwrap_or("?"),
        num(&node["fontSize"]).unwrap_or_else(|| "?".into()), node["fontWeight"].as_str().unwrap_or("?"));
    if let Some(lh) = num(&node["lineHeight"]).or_else(|| node["lineHeight"].as_str().map(str::to_string)) { s.push_str(&format!(" lh{lh}")); }
    if let Some(ls) = num(&node["letterSpacing"]).filter(|l| l != "0") { s.push_str(&format!(" ls{ls}")); }
    s
}

fn size(node: &Value) -> String {
    format!("{}x{}", num(&node["width"]).unwrap_or_else(|| "?".into()), num(&node["height"]).unwrap_or_else(|| "?".into()))
}

fn signature(block: &str) -> String {
    // Copy differs between list items; structure and styling must not.
    let mut out = String::with_capacity(block.len());
    let mut in_quote = false;
    for c in block.chars() {
        if c == '"' { in_quote = !in_quote; out.push(c); } else if !in_quote { out.push(c); }
    }
    out
}

fn quote(text: &str) -> String {
    let one_line = text.replace('\n', "⏎");
    let short: String = one_line.chars().take(60).collect();
    format!("\"{}{}\"", short.replace('"', "'"), if one_line.chars().count() > 60 { "…" } else { "" })
}

fn plain(v: &Value) -> String { v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()) }

fn is_node_id(s: &str) -> bool {
    let mut parts = s.split(':');
    parts.clone().count() == 2 && parts.all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn fmt(v: f64) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn num(v: &Value) -> Option<String> { v.as_f64().map(fmt) }

fn channel(v: &Value) -> String { format!("{}", (v.as_f64().unwrap_or(0.0) * 255.0).round() as u8) }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn outline_prunes_shapes_collapses_repeats_and_names_tokens() {
        let card = |n: u32, title: &str| vec![
            json!({"id":format!("c{n}"),"parentId":"list","type":"FRAME","name":"Card","width":370,"height":83,"layoutMode":"HORIZONTAL",
                "itemSpacing":8,"paddingTop":8,"paddingRight":8,"paddingBottom":8,"paddingLeft":8,"cornerRadius":10,
                "strokeWeight":1.5,"strokes":[{"type":"SOLID","color":"#f6f6f6","visible":true}]}),
            json!({"id":format!("t{n}"),"parentId":format!("c{n}"),"type":"TEXT","name":"Title","content":title,
                "fontFamily":"Inter","fontSize":14,"fontWeight":"Medium","lineHeight":22,"fill":"#122936",
                "paintData":[{"type":"SOLID","color":"#122936","visible":true,"boundVariables":{"color":{"type":"VARIABLE_ALIAS","id":"VariableID:lib/1:2"}}}]}),
        ];
        let mut nodes = vec![
            json!({"id":"root","type":"FRAME","name":"Screen","width":402,"height":874,"layoutMode":"VERTICAL","itemSpacing":12,
                "paddingTop":12,"paddingRight":16,"paddingBottom":16,"paddingLeft":16,
                "boundVariables":{"itemSpacing":{"type":"VARIABLE_ALIAS","id":"VariableID:4:305"}},
                "paintData":[{"type":"SOLID","color":"#ffffff","visible":true}]}),
            json!({"id":"sb","parentId":"root","type":"INSTANCE","name":"Status bar - iPhone","width":402,"height":62}),
            json!({"id":"sbt","parentId":"sb","type":"TEXT","name":"Time","content":"9:41"}),
            json!({"id":"icon","parentId":"root","type":"INSTANCE","name":"Search","width":24,"height":24}),
            json!({"id":"vec","parentId":"icon","type":"VECTOR","name":"Vector","strokes":[{"type":"SOLID","color":"#122936","visible":true}]}),
            json!({"id":"list","parentId":"root","type":"FRAME","name":"List","width":370,"height":200,"layoutMode":"VERTICAL","itemSpacing":8}),
        ];
        nodes.extend(card(1, "Retatrutide 30mg"));
        nodes.extend(card(2, "GHK-Cu"));
        nodes.extend(card(3, "Retatrutide 30mg"));
        let out = render(&json!({"nodes":nodes,"names":{"VariableID:lib/1:2":"text/deep-navy/primary","VariableID:4:305":"spacing/16"}}));
        assert_eq!(out, "\
FRAME 'Screen' 402x874 col gap12(spacing/16) pad12,16,16,16 bg #ffffff
  INSTANCE 'Status bar - iPhone' 402x62 (device chrome)
  ICON 'Search' 24x24 #122936
  FRAME 'List' 370x200 col gap8
    FRAME 'Card' 370x83 row gap8 pad8 border 1.5 #f6f6f6 r10
      TEXT \"Retatrutide 30mg\" Inter 14/Medium lh22 #122936(text/deep-navy/primary)
    FRAME 'Card' 370x83 = same layout as earlier: \"GHK-Cu\"
    FRAME 'Card' 370x83 = same layout as earlier
");
    }

    #[test]
    fn outline_fixture_size() {
        let Ok(path) = std::env::var("FIGMA_OUTLINE_FIXTURE") else { return };
        let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let out = render(&fixture);
        eprintln!("{out}\noutline: {} -> {} bytes", fixture.to_string().len(), out.len());
    }
}
