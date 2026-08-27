use serde_json::Value;

pub fn generate_code_from_context(
    context: &Value,
    framework: &str,
    component_name: Option<&str>,
) -> Result<String, String> {
    let name = component_name
        .map(sanitize_component_name)
        .unwrap_or_else(|| {
            let n = context.get("name").and_then(|v| v.as_str()).unwrap_or("Component");
            sanitize_component_name(n)
        });

    let fmt = framework.to_lowercase();
    match fmt.as_str() {
        "react-tailwind" | "react" | "next" | "tailwind" => Ok(generate_react_tailwind(context, &name)),
        "react-shadcn" | "shadcn" | "shadcn-ui" => Ok(generate_shadcn_react(context, &name)),
        "react-native" | "rn" => Ok(generate_react_native(context, &name)),
        "vue" | "vue-tailwind" => Ok(generate_vue_tailwind(context, &name)),
        "html" | "html-tailwind" => Ok(generate_html_tailwind(context)),
        "swiftui" => Ok(generate_swiftui(context, &name)),
        "clean-spec" | "clean" | "spec" | "yaml-spec" => Ok(generate_clean_spec(context)),
        _ => Err(format!(
            "Unsupported framework: '{}'. Available: 'react-tailwind', 'react-shadcn', 'react-native', 'vue-tailwind', 'html', 'swiftui', 'clean-spec'",
            framework
        )),
    }
}

fn sanitize_component_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "Component".to_string();
    }
    if trimmed.contains(' ') || trimmed.contains('-') || trimmed.contains('_') || trimmed.contains('/') {
        let clean: String = trimmed
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect();
        let words: Vec<&str> = clean.split_whitespace().collect();
        return words
            .into_iter()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect();
    }
    let mut chars = trimmed.chars().filter(|c| c.is_alphanumeric());
    match chars.next() {
        None => "Component".to_string(),
        Some(first) => first.to_uppercase().collect::<String>() + &chars.collect::<String>(),
    }
}

// ── Tailwind Helpers ─────────────────────────────────────────────────────────

fn px_to_tailwind_spacing(px: f64) -> String {
    if (px - 0.0).abs() < 0.1 { return "0".to_string(); }
    if (px - 4.0).abs() < 0.1 { return "1".to_string(); }
    if (px - 8.0).abs() < 0.1 { return "2".to_string(); }
    if (px - 12.0).abs() < 0.1 { return "3".to_string(); }
    if (px - 16.0).abs() < 0.1 { return "4".to_string(); }
    if (px - 20.0).abs() < 0.1 { return "5".to_string(); }
    if (px - 24.0).abs() < 0.1 { return "6".to_string(); }
    if (px - 32.0).abs() < 0.1 { return "8".to_string(); }
    if (px - 40.0).abs() < 0.1 { return "10".to_string(); }
    if (px - 48.0).abs() < 0.1 { return "12".to_string(); }
    if (px - 64.0).abs() < 0.1 { return "16".to_string(); }
    format!("[{}px]", px.round() as i64)
}

fn px_to_tailwind_radius(px: f64) -> &'static str {
    if px <= 0.0 { return "rounded-none"; }
    if px <= 3.0 { return "rounded-sm"; }
    if px <= 6.0 { return "rounded"; }
    if px <= 10.0 { return "rounded-lg"; }
    if px <= 14.0 { return "rounded-xl"; }
    if px <= 20.0 { return "rounded-2xl"; }
    if px <= 30.0 { return "rounded-3xl"; }
    "rounded-full"
}

fn hex_to_tailwind_color(hex_or_var: &str, prefix: &str) -> String {
    let s = hex_or_var.trim();
    if s.starts_with("var(") {
        return format!("{}-[{}]", prefix, s);
    }
    match s.to_lowercase().as_str() {
        "#ffffff" | "#fff" | "rgb(255,255,255)" | "rgba(255,255,255,1)" => format!("{}-white", prefix),
        "#000000" | "#000" | "rgb(0,0,0)" | "rgba(0,0,0,1)" => format!("{}-black", prefix),
        "#f8fafc" => format!("{}-slate-50", prefix),
        "#f1f5f9" => format!("{}-slate-100", prefix),
        "#e2e8f0" => format!("{}-slate-200", prefix),
        "#cbd5e1" => format!("{}-slate-300", prefix),
        "#94a3b8" => format!("{}-slate-400", prefix),
        "#64748b" => format!("{}-slate-500", prefix),
        "#475569" => format!("{}-slate-600", prefix),
        "#334155" => format!("{}-slate-700", prefix),
        "#1e293b" => format!("{}-slate-800", prefix),
        "#0f172a" => format!("{}-slate-900", prefix),
        "#020617" => format!("{}-slate-950", prefix),
        _ => format!("{}-[{}]", prefix, s),
    }
}

fn node_to_tailwind_classes(node: &Value) -> Vec<String> {
    let mut classes = Vec::new();

    // 1. Layout
    if let Some(layout) = node.get("layout") {
        classes.push("flex".to_string());
        
        let width = node.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let is_column = layout.get("flexDirection").and_then(|v| v.as_str()) == Some("column");
        
        if is_column {
            classes.push("flex-col".to_string());
        } else {
            // Responsive breakpoint inference: large horizontal frames collapse to column on mobile
            if width >= 768.0 {
                classes.push("flex-col md:flex-row".to_string());
            } else {
                classes.push("flex-row".to_string());
            }
        }

        // Wrap inference
        if layout.get("layoutWrap").and_then(|v| v.as_str()) == Some("WRAP")
            || layout.get("wrap").and_then(|v| v.as_bool()) == Some(true)
        {
            classes.push("flex-wrap".to_string());
        }

        if let Some(gap_str) = layout.get("gap").and_then(|v| v.as_str()) {
            if let Ok(num) = gap_str.trim_end_matches("px").parse::<f64>() {
                if num > 0.0 {
                    classes.push(format!("gap-{}", px_to_tailwind_spacing(num)));
                }
            }
        }

        if let Some(items) = layout.get("alignItems").and_then(|v| v.as_str()) {
            match items {
                "center" => classes.push("items-center".to_string()),
                "flex-start" => classes.push("items-start".to_string()),
                "flex-end" => classes.push("items-end".to_string()),
                "stretch" => classes.push("items-stretch".to_string()),
                _ => {}
            }
        }

        if let Some(justify) = layout.get("justifyContent").and_then(|v| v.as_str()) {
            match justify {
                "center" => classes.push("justify-center".to_string()),
                "flex-start" => classes.push("justify-start".to_string()),
                "flex-end" => classes.push("justify-end".to_string()),
                "space-between" => classes.push("justify-between".to_string()),
                _ => {}
            }
        }

        if let Some(pad) = layout.get("padding").and_then(|v| v.as_str()) {
            let parts: Vec<f64> = pad
                .split_whitespace()
                .filter_map(|p| p.trim_end_matches("px").parse::<f64>().ok())
                .collect();
            if parts.len() == 4 {
                let (top, right, bottom, left) = (parts[0], parts[1], parts[2], parts[3]);
                if (top - bottom).abs() < 0.1 && (left - right).abs() < 0.1 {
                    if (top - left).abs() < 0.1 && top > 0.0 {
                        classes.push(format!("p-{}", px_to_tailwind_spacing(top)));
                    } else {
                        if top > 0.0 { classes.push(format!("py-{}", px_to_tailwind_spacing(top))); }
                        if left > 0.0 { classes.push(format!("px-{}", px_to_tailwind_spacing(left))); }
                    }
                } else {
                    if top > 0.0 { classes.push(format!("pt-{}", px_to_tailwind_spacing(top))); }
                    if right > 0.0 { classes.push(format!("pr-{}", px_to_tailwind_spacing(right))); }
                    if bottom > 0.0 { classes.push(format!("pb-{}", px_to_tailwind_spacing(bottom))); }
                    if left > 0.0 { classes.push(format!("pl-{}", px_to_tailwind_spacing(left))); }
                }
            }
        }
    }

    // 2. Sizing / Constraints
    if let Some(grow) = node.get("layoutGrow").and_then(|v| v.as_f64()) {
        if grow > 0.0 { classes.push("flex-1".to_string()); }
    }
    if let Some(align) = node.get("layoutAlign").and_then(|v| v.as_str()) {
        if align == "STRETCH" { classes.push("w-full".to_string()); }
    }
    if let Some(min_w) = node.get("minWidth").and_then(|v| v.as_f64()) {
        if min_w > 0.0 { classes.push(format!("min-w-[{}px]", min_w as i64)); }
    }
    if let Some(max_w) = node.get("maxWidth").and_then(|v| v.as_f64()) {
        if max_w > 0.0 {
            if (max_w - 1280.0).abs() < 10.0 {
                classes.push("max-w-screen-xl".to_string());
            } else if (max_w - 1024.0).abs() < 10.0 {
                classes.push("max-w-screen-lg".to_string());
            } else {
                classes.push(format!("max-w-[{}px]", max_w as i64));
            }
        }
    }

    // 3. Fill (Background / Text color)
    if let Some(fill) = node.get("fill").and_then(|v| v.as_str()) {
        let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if node_type == "TEXT" {
            classes.push(hex_to_tailwind_color(fill, "text"));
        } else {
            classes.push(hex_to_tailwind_color(fill, "bg"));
        }
    }

    // 4. Border / Stroke
    if let Some(stroke) = node.get("stroke") {
        if let Some(color) = stroke.get("color").and_then(|v| v.as_str()) {
            classes.push("border".to_string());
            classes.push(hex_to_tailwind_color(color, "border"));
        }
    }

    // 5. Border Radius
    if let Some(radius_str) = node.get("borderRadius").and_then(|v| v.as_str()) {
        if let Ok(num) = radius_str.trim_end_matches("px").parse::<f64>() {
            classes.push(px_to_tailwind_radius(num).to_string());
        }
    }

    // 6. Effects (Shadows)
    if let Some(effects) = node.get("effects").and_then(|v| v.as_array()) {
        for eff in effects {
            if eff.get("type").and_then(|v| v.as_str()) == Some("DROP_SHADOW") {
                let radius = eff.get("radius").and_then(|v| v.as_f64()).unwrap_or(4.0);
                if radius <= 3.0 { classes.push("shadow-sm".to_string()); }
                else if radius <= 8.0 { classes.push("shadow".to_string()); }
                else if radius <= 16.0 { classes.push("shadow-md".to_string()); }
                else if radius <= 24.0 { classes.push("shadow-lg".to_string()); }
                else { classes.push("shadow-xl".to_string()); }
                break;
            }
        }
    }

    // 7. Typography (For TEXT nodes)
    if let Some(typo) = node.get("typography") {
        if let Some(size) = typo.get("fontSize").and_then(|v| v.as_f64()) {
            if size <= 12.0 { classes.push("text-xs".to_string()); }
            else if size <= 14.0 { classes.push("text-sm".to_string()); }
            else if size <= 16.0 { classes.push("text-base".to_string()); }
            else if size <= 18.0 { classes.push("text-lg".to_string()); }
            else if size <= 20.0 { classes.push("text-xl".to_string()); }
            else if size <= 24.0 { classes.push("text-2xl".to_string()); }
            else if size <= 30.0 { classes.push("text-3xl".to_string()); }
            else if size <= 36.0 { classes.push("text-4xl".to_string()); }
            else { classes.push(format!("text-[{}px]", size as i64)); }
        }

        if let Some(weight) = typo.get("fontWeight").and_then(|v| v.as_str()) {
            match weight.to_lowercase().as_str() {
                "bold" | "700" => classes.push("font-bold".to_string()),
                "semibold" | "600" => classes.push("font-semibold".to_string()),
                "medium" | "500" => classes.push("font-medium".to_string()),
                "light" | "300" => classes.push("font-light".to_string()),
                _ => {}
            }
        }
    }

    classes
}

// ── React + Tailwind Generator ───────────────────────────────────────────────

fn generate_react_tailwind(context: &Value, component_name: &str) -> String {
    // 1. Optimize tree by flattening redundant single-child wrapper frames
    let optimized_context = crate::mcp::semantic_optimizer::optimize_semantic_tree(context);

    // 2. Infer dynamic interactive states, repeaters, and typescript props interface
    let logic = crate::mcp::state_engine::infer_component_logic(&optimized_context, component_name);

    let mut jsx_buffer = String::new();
    render_jsx_node(&optimized_context, &mut jsx_buffer, 2);

    let mut state_decls = String::new();
    for st in &logic.states {
        state_decls.push_str(&format!(
            "  const [{}, {}] = React.useState<{}>(Boolean({}));\n",
            st.state_name, st.setter_name, st.state_type, st.default_value
        ));
    }

    let state_block = if state_decls.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", state_decls)
    };

    format!(
        "import React from 'react';\n\n{interface}\n\nexport const {name}: React.FC<{name}Props> = ({{\n  className = '',\n}}) => {{{state_block}  return (\n{jsx}\n  );\n}};\n\nexport default {name};\n",
        interface = logic.props_interface,
        name = component_name,
        state_block = state_block,
        jsx = jsx_buffer
    )
}

fn render_jsx_node(node: &Value, out: &mut String, indent_level: usize) {
    let indent = "  ".repeat(indent_level);
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("FRAME");
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let text = node.get("characters").and_then(|v| v.as_str());

    let is_button = name.to_lowercase().contains("button") || name.to_lowercase().contains("btn");
    let tag = if is_button {
        "button"
    } else if node_type == "TEXT" {
        if name.to_lowercase().contains("h1") || name.to_lowercase().contains("title") {
            "h2"
        } else {
            "span"
        }
    } else {
        "div"
    };

    let classes = node_to_tailwind_classes(node);
    let class_attr = if classes.is_empty() {
        String::new()
    } else {
        format!(" className=\"{}\"", classes.join(" "))
    };

    if node_type == "TEXT" {
        let content = text.unwrap_or(name);
        out.push_str(&format!("{}<{}{}>{}</{}>\n", indent, tag, class_attr, content, tag));
        return;
    }

    let children = node.get("children").and_then(|v| v.as_array());
    if let Some(child_nodes) = children {
        if !child_nodes.is_empty() {
            out.push_str(&format!("{}<{}{}>\n", indent, tag, class_attr));
            for child in child_nodes {
                render_jsx_node(child, out, indent_level + 1);
            }
            out.push_str(&format!("{}</{}>\n", indent, tag));
            return;
        }
    }

    out.push_str(&format!("{}<{}{} />\n", indent, tag, class_attr));
}

// ── React + Shadcn/UI Component Generator ─────────────────────────────────────

#[derive(Default)]
struct ShadcnImports {
    button: bool,
    badge: bool,
    card: bool,
    input: bool,
    avatar: bool,
    switch: bool,
}

fn generate_shadcn_react(context: &Value, component_name: &str) -> String {
    let mut imports = ShadcnImports::default();
    let mut jsx_buffer = String::new();
    render_shadcn_node(context, &mut jsx_buffer, 2, &mut imports);

    let mut import_lines = vec!["import React from 'react';".to_string()];
    if imports.button {
        import_lines.push("import { Button } from '@/components/ui/button';".to_string());
    }
    if imports.badge {
        import_lines.push("import { Badge } from '@/components/ui/badge';".to_string());
    }
    if imports.card {
        import_lines.push("import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from '@/components/ui/card';".to_string());
    }
    if imports.input {
        import_lines.push("import { Input } from '@/components/ui/input';".to_string());
    }
    if imports.avatar {
        import_lines.push("import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar';".to_string());
    }
    if imports.switch {
        import_lines.push("import { Switch } from '@/components/ui/switch';".to_string());
    }

    format!(
        "{imports}\n\ninterface {name}Props {{\n  className?: string;\n}}\n\nexport const {name}: React.FC<{name}Props> = ({{\n  className = '',\n}}) => {{\n  return (\n{jsx}\n  );\n}};\n\nexport default {name};\n",
        imports = import_lines.join("\n"),
        name = component_name,
        jsx = jsx_buffer
    )
}

fn render_shadcn_node(node: &Value, out: &mut String, indent_level: usize, imports: &mut ShadcnImports) {
    let indent = "  ".repeat(indent_level);
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("FRAME");
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
    let text = node.get("characters").and_then(|v| v.as_str());

    // 1. Detect Button
    if name.contains("button") || name.contains("btn") {
        imports.button = true;
        let mut variant = "default";
        let fill = node.get("fill").and_then(|v| v.as_str()).unwrap_or("");
        if fill.contains("red") || fill.contains("#ef4444") || fill.contains("#dc2626") {
            variant = "destructive";
        } else if node.get("stroke").is_some() {
            variant = "outline";
        } else if fill.contains("secondary") || fill.contains("#f1f5f9") {
            variant = "secondary";
        } else if fill.is_empty() || fill == "#ffffff" || fill == "transparent" {
            variant = "ghost";
        }

        let label = find_child_text(node).unwrap_or_else(|| "Button".to_string());
        if variant == "default" {
            out.push_str(&format!("{}<Button>{}</Button>\n", indent, label));
        } else {
            out.push_str(&format!("{}<Button variant=\"{}\">{}</Button>\n", indent, variant, label));
        }
        return;
    }

    // 2. Detect Badge
    if name.contains("badge") || name.contains("tag") || name.contains("pill") {
        imports.badge = true;
        let label = find_child_text(node).unwrap_or_else(|| "Badge".to_string());
        out.push_str(&format!("{}<Badge>{}</Badge>\n", indent, label));
        return;
    }

    // 3. Detect Avatar
    if name.contains("avatar") || name.contains("userpic") || name.contains("profile-pic") {
        imports.avatar = true;
        out.push_str(&format!("{}<Avatar>\n{}  <AvatarImage src=\"https://github.com/shadcn.png\" alt=\"User\" />\n{}  <AvatarFallback>CN</AvatarFallback>\n{}</Avatar>\n", indent, indent, indent, indent));
        return;
    }

    // 4. Detect Input
    if name.contains("input") || name.contains("textfield") || name.contains("searchbar") {
        imports.input = true;
        let placeholder = find_child_text(node).unwrap_or_else(|| "Type here...".to_string());
        out.push_str(&format!("{}<Input placeholder=\"{}\" />\n", indent, placeholder));
        return;
    }

    // 5. Detect Card
    if name.contains("card") && node_type == "FRAME" {
        imports.card = true;
        let classes = node_to_tailwind_classes(node);
        let class_attr = if classes.is_empty() { String::new() } else { format!(" className=\"{}\"", classes.join(" ")) };
        out.push_str(&format!("{}<Card{}>\n", indent, class_attr));
        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for child in children {
                render_shadcn_node(child, out, indent_level + 1, imports);
            }
        }
        out.push_str(&format!("{}</Card>\n", indent));
        return;
    }

    // Default: Fallback to standard JSX
    let tag = if node_type == "TEXT" { "span" } else { "div" };
    let classes = node_to_tailwind_classes(node);
    let class_attr = if classes.is_empty() { String::new() } else { format!(" className=\"{}\"", classes.join(" ")) };

    if node_type == "TEXT" {
        let content = text.unwrap_or(&name);
        out.push_str(&format!("{}<{}{}>{}</{}>\n", indent, tag, class_attr, content, tag));
        return;
    }

    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        if !children.is_empty() {
            out.push_str(&format!("{}<{}{}>\n", indent, tag, class_attr));
            for child in children {
                render_shadcn_node(child, out, indent_level + 1, imports);
            }
            out.push_str(&format!("{}</{}>\n", indent, tag));
            return;
        }
    }

    out.push_str(&format!("{}<{}{} />\n", indent, tag, class_attr));
}

fn find_child_text(node: &Value) -> Option<String> {
    if let Some(c) = node.get("characters").and_then(|v| v.as_str()) {
        return Some(c.to_string());
    }
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            if let Some(txt) = find_child_text(child) {
                return Some(txt);
            }
        }
    }
    None
}

// ── Clean Spec / Token-Pruned AST Generator ──────────────────────────────────

pub fn prune_ast_node(node: &Value) -> Value {
    if let Some(obj) = node.as_object() {
        let mut pruned = serde_json::Map::new();

        for (k, v) in obj {
            match k.as_str() {
                "visible" => { if v.as_bool() == Some(false) { pruned.insert(k.clone(), v.clone()); } }
                "opacity" => { if let Some(op) = v.as_f64() { if (op - 1.0).abs() > 0.01 { pruned.insert(k.clone(), v.clone()); } } }
                "blendMode" => { if v.as_str() != Some("PASS_THROUGH") { pruned.insert(k.clone(), v.clone()); } }
                "padding" => {
                    if let Some(pad_str) = v.as_str() {
                        if pad_str != "0px 0px 0px 0px" && pad_str != "0 0 0 0" {
                            pruned.insert(k.clone(), v.clone());
                        }
                    } else if !v.is_null() {
                        pruned.insert(k.clone(), v.clone());
                    }
                }
                "borderRadius" => {
                    if let Some(r_str) = v.as_str() {
                        if r_str != "0px" && r_str != "0" {
                            pruned.insert(k.clone(), v.clone());
                        }
                    }
                }
                "children" => {
                    if let Some(arr) = v.as_array() {
                        if !arr.is_empty() {
                            let pruned_children: Vec<Value> = arr.iter().map(prune_ast_node).collect();
                            pruned.insert(k.clone(), Value::Array(pruned_children));
                        }
                    }
                }
                _ => {
                    if !v.is_null() {
                        pruned.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        return Value::Object(pruned);
    }
    node.clone()
}

fn generate_clean_spec(context: &Value) -> String {
    let mut out = String::new();
    render_clean_spec_node(context, &mut out, 0);
    out
}

fn render_clean_spec_node(node: &Value, out: &mut String, indent_level: usize) {
    let indent = "  ".repeat(indent_level);
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("Layer");
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("FRAME");
    let text = node.get("characters").and_then(|v| v.as_str());

    let mut attrs = Vec::new();
    if let Some(fill) = node.get("fill").and_then(|v| v.as_str()) {
        attrs.push(format!("fill=\"{}\"", fill));
    }
    if let Some(layout) = node.get("layout") {
        if let Some(dir) = layout.get("flexDirection").and_then(|v| v.as_str()) {
            attrs.push(format!("flex=\"{}\"", dir));
        }
        if let Some(gap) = layout.get("gap").and_then(|v| v.as_str()) {
            if gap != "0px" { attrs.push(format!("gap=\"{}\"", gap)); }
        }
    }
    if let Some(radius) = node.get("borderRadius").and_then(|v| v.as_str()) {
        if radius != "0px" { attrs.push(format!("radius=\"{}\"", radius)); }
    }

    let attr_str = if attrs.is_empty() { String::new() } else { format!(" {}", attrs.join(" ")) };

    if node_type == "TEXT" {
        let content = text.unwrap_or(name);
        out.push_str(&format!("{}<Text name=\"{}\"{}>{}</Text>\n", indent, name, attr_str, content));
        return;
    }

    let children = node.get("children").and_then(|v| v.as_array());
    if let Some(child_nodes) = children {
        if !child_nodes.is_empty() {
            out.push_str(&format!("{}<{} name=\"{}\"{}>\n", indent, node_type, name, attr_str));
            for child in child_nodes {
                render_clean_spec_node(child, out, indent_level + 1);
            }
            out.push_str(&format!("{}</{}>\n", indent, node_type));
            return;
        }
    }

    out.push_str(&format!("{}<{} name=\"{}\"{} />\n", indent, node_type, name, attr_str));
}

// ── React Native Generator ───────────────────────────────────────────────────

fn generate_react_native(context: &Value, component_name: &str) -> String {
    let mut buffer = String::new();
    render_rn_node(context, &mut buffer, 2);

    format!(
        "import React from 'react';\nimport {{ View, Text, StyleSheet, TouchableOpacity }} from 'react-native';\n\ninterface {name}Props {{\n  style?: any;\n}}\n\nexport const {name}: React.FC<{name}Props> = ({{\n  style,\n}}) => {{\n  return (\n{content}\n  );\n}};\n\nexport default {name};\n",
        name = component_name,
        content = buffer
    )
}

fn render_rn_node(node: &Value, out: &mut String, indent_level: usize) {
    let indent = "  ".repeat(indent_level);
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("FRAME");
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let text = node.get("characters").and_then(|v| v.as_str());

    let is_btn = name.to_lowercase().contains("button");
    let tag = if is_btn {
        "TouchableOpacity"
    } else if node_type == "TEXT" {
        "Text"
    } else {
        "View"
    };

    if node_type == "TEXT" {
        let content = text.unwrap_or(name);
        out.push_str(&format!("{}<{}>{}</{}>\n", indent, tag, content, tag));
        return;
    }

    let children = node.get("children").and_then(|v| v.as_array());
    if let Some(child_nodes) = children {
        if !child_nodes.is_empty() {
            out.push_str(&format!("{}<{}>\n", indent, tag));
            for child in child_nodes {
                render_rn_node(child, out, indent_level + 1);
            }
            out.push_str(&format!("{}</{}>\n", indent, tag));
            return;
        }
    }

    out.push_str(&format!("{}<{} />\n", indent, tag));
}

// ── Vue 3 SFC Generator ──────────────────────────────────────────────────────

fn generate_vue_tailwind(context: &Value, _component_name: &str) -> String {
    let mut template_buffer = String::new();
    render_jsx_node(context, &mut template_buffer, 1);

    format!(
        "<script setup lang=\"ts\">\n// Generated by Figma MCP (https://github.com/BuiHung1612/figma-mcp)\n</script>\n\n<template>\n{template}</template>\n",
        template = template_buffer
    )
}

// ── Plain HTML Generator ─────────────────────────────────────────────────────

fn generate_html_tailwind(context: &Value) -> String {
    let mut buffer = String::new();
    render_jsx_node(context, &mut buffer, 0);
    buffer
}

// ── SwiftUI Generator ────────────────────────────────────────────────────────

fn generate_swiftui(context: &Value, component_name: &str) -> String {
    let mut body_buffer = String::new();
    render_swiftui_node(context, &mut body_buffer, 2);

    format!(
        "import SwiftUI\n\n// Generated by Figma MCP (https://github.com/BuiHung1612/figma-mcp)\nstruct {name}: View {{\n    var body: some View {{\n{body}    }}\n}}\n\n#Preview {{\n    {name}()\n}}\n",
        name = component_name,
        body = body_buffer
    )
}

fn render_swiftui_node(node: &Value, out: &mut String, indent_level: usize) {
    let indent = "    ".repeat(indent_level);
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("FRAME");
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let text = node.get("characters").and_then(|v| v.as_str());

    if node_type == "TEXT" {
        let content = text.unwrap_or(name);
        out.push_str(&format!("{}Text(\"{}\")\n", indent, content));
        return;
    }

    let is_horizontal = node.get("layout").and_then(|l| l.get("flexDirection")).and_then(|v| v.as_str()) == Some("row");
    let stack_type = if is_horizontal { "HStack" } else { "VStack" };

    let children = node.get("children").and_then(|v| v.as_array());
    if let Some(child_nodes) = children {
        if !child_nodes.is_empty() {
            out.push_str(&format!("{}{} {{\n", indent, stack_type));
            for child in child_nodes {
                render_swiftui_node(child, out, indent_level + 1);
            }
            out.push_str(&format!("{}}}\n", indent));
            return;
        }
    }

    out.push_str(&format!("{}Rectangle()\n", indent));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_codegen_react_tailwind() {
        let context = json!({
            "name": "UserProfileCard",
            "type": "FRAME",
            "width": 800.0,
            "layout": {
                "display": "flex",
                "flexDirection": "row",
                "gap": "16px",
                "alignItems": "center",
                "justifyContent": "space-between",
                "padding": "24px 24px 24px 24px"
            },
            "fill": "#ffffff",
            "borderRadius": "16px",
            "children": [
                {
                    "name": "Header Title",
                    "type": "TEXT",
                    "characters": "Welcome Back, User!",
                    "typography": {
                        "fontSize": 24.0,
                        "fontWeight": "Bold"
                    },
                    "fill": "#0f172a"
                },
                {
                    "name": "Submit Button",
                    "type": "FRAME",
                    "layout": {
                        "display": "flex",
                        "flexDirection": "row",
                        "gap": "8px",
                        "alignItems": "center",
                        "justifyContent": "center",
                        "padding": "8px 16px 8px 16px"
                    },
                    "fill": "#6366f1",
                    "borderRadius": "8px",
                    "children": [
                        {
                            "name": "Button Text",
                            "type": "TEXT",
                            "characters": "Get Started",
                            "fill": "#ffffff"
                        }
                    ]
                }
            ]
        });

        let code = generate_code_from_context(&context, "react-tailwind", Some("UserProfileCard")).unwrap();
        assert!(code.contains("export const UserProfileCard: React.FC<UserProfileCardProps>"));
        assert!(code.contains("flex flex-col md:flex-row"));
        assert!(code.contains("gap-4"));
        assert!(code.contains("p-6"));
        assert!(code.contains("bg-white"));
        assert!(code.contains("rounded-2xl"));
        assert!(code.contains("<button"));
        assert!(code.contains("bg-[#6366f1]"));
        assert!(code.contains("Welcome Back, User!"));

        let shadcn_code = generate_code_from_context(&context, "react-shadcn", Some("UserProfileCard")).unwrap();
        assert!(shadcn_code.contains("import { Button } from '@/components/ui/button';"));
        assert!(shadcn_code.contains("<Button>Get Started</Button>"));

        let clean_spec = generate_code_from_context(&context, "clean-spec", None).unwrap();
        assert!(clean_spec.contains("<FRAME name=\"UserProfileCard\" fill=\"#ffffff\" flex=\"row\" gap=\"16px\" radius=\"16px\">"));

        let rn_code = generate_code_from_context(&context, "react-native", Some("UserProfileCard")).unwrap();
        assert!(rn_code.contains("import { View, Text, StyleSheet, TouchableOpacity } from 'react-native'"));
        assert!(rn_code.contains("<TouchableOpacity>"));

        let vue_code = generate_code_from_context(&context, "vue-tailwind", Some("UserProfileCard")).unwrap();
        assert!(vue_code.contains("<script setup lang=\"ts\">"));
        assert!(vue_code.contains("<template>"));

        let swift_code = generate_code_from_context(&context, "swiftui", Some("UserProfileCard")).unwrap();
        assert!(swift_code.contains("struct UserProfileCard: View"));
        assert!(swift_code.contains("HStack {"));
    }
}
