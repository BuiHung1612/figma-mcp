use super::protocol::ToolDefinition;
use serde_json::json;

pub fn get_tools() -> Vec<ToolDefinition> {
    let mut tools = vec![
        ToolDefinition {
            name: "figma_status".to_string(),
            description: "Check the Figma plugin connection, connected tabs (sessionId) and bridge stats. Call first. With `task`, reserve a frame for one AI task: task='start' + frameId returns a taskId to pass to every later call (writes are confined to that frame); task='end' + taskId releases it; task='list' shows reservations.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task": {"type": "string", "enum": ["start", "end", "list"], "description": "Frame reservation action (optional)."},
                    "frameId": {"type": "string", "description": "task=start: independent FRAME under PAGE/SECTION, without component definitions."},
                    "taskId": {"type": "string", "description": "task=end: the taskId returned by start."},
                    "sessionId": {"type": "string", "description": "Exact tab ID; required for task actions when multiple tabs are connected."}
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "figma_read".to_string(),
            description: "READ anything from Figma. Nodes: get_selection, read_nodes, inspect_node (one node: CSS, layout, tokens, typography; defaults to selection), get_spec, scan_design, get_css. Search (in-memory index): search_nodes, search_components, search_styles, search_variables, subtree, typography, index_status, refresh. Design system: rules (markdown rule sheet of tokens, text styles and components), get_styles, get_variables, get_tokens, get_local_components, get_component_map. Export: screenshot, export_image (png/jpg), export_svg — pass outputPath to save to disk.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": [
                            "get_selection", "read_nodes", "inspect_node", "get_spec", "get_page_nodes", "scan_design", "get_css",
                            "search_nodes", "search_components", "search_styles", "search_variables", "subtree", "typography", "index_status", "refresh",
                            "rules", "get_styles", "get_variables", "get_tokens", "get_local_components", "get_component_map", "get_unmapped_components", "get_viewport",
                            "screenshot", "export_image", "export_svg"
                        ],
                        "description": "read_nodes: canonical flat node API with fields, depth, expandInstances and cursor. Returns schemaVersion, nodes, scope, revision, nextCursor and complete. Default fields are geometry and content; text adds typography including mixed runs. Tokens returns raw bindings/style IDs. Instances remain opaque unless expandInstances=true. Cursors expire after edits; continue with cursor and limit only. Other operations provide selection, styles/tokens, CSS, components, exports and scoped design summaries.",
                    },
                    "mode": {"type":"string","enum":["overview","spec","detail"],"description":"get_spec: overview (default) reads depth 2 to discover sections; spec reads a section deeply; detail reads one node. Exact shared styles/templates preserve overrides."},
                    "maxBytes": {"type":"integer","minimum":1024,"maximum":1000000,"description":"Raw node byte target per page; default 24000 for get_spec. One oversized node and metadata/component resolution can exceed it. Continue nextCursor; no nodes are dropped."},
                    "fields": { "type":"array", "items":{"type":"string","enum":["geometry","content","text","style","layout","tokens","component"]}, "minItems":1, "description":"Requested node groups; default geometry/content. Identity and topology are always returned." },
                    "cursor": { "type":"string", "description":"Opaque continuation from read_nodes. Send cursor and limit only; expired cursors require restarting." },
                    "expandInstances": { "type":"boolean", "description":"Expand instance descendants; default false (instances report opaque=true); scan_design defaults true." },
                    "nodeId": { "type": "string", "description": "Target node ID (optional — omit to use current selection)." },
                    "nodeName": { "type": "string", "description": "Target node name (alternative to nodeId)." },
                    "scale": { "type": "number", "description": "Export scale for screenshot / export_image (default 1 for screenshot, 2 for export_image)." },
                    "depth": { "oneOf":[{"type":"integer","minimum":0,"maximum":256},{"type":"string","enum":["full"]}], "description":"read_nodes depth: page default 0, frame default full (256)." },
                    "limit": { "type":"integer", "minimum":1, "maximum":500, "description":"Nodes per read_nodes response; default 200." },
                    "format": { "type": "string", "description": "Image format for export_image: 'png' (default) or 'jpg'." },
                    "detail": { "type": "string", "description": "Detail level for read_nodes/get_design/get_selection: 'full' (default) bypasses Rust compression; 'compact'/'minimal' use shared styles when smaller. Merge _compression.styles[node.styleRef] into referenced nodes. Plugin detail/depth/node budgets still apply." },
                    "outputPath": { "type": "string", "description": "get_spec: save the complete requested raw scope to JSON and return coverage only. Also saves export_svg/export_image/screenshot." },
                    "includeHidden": { "type": "boolean", "description": "Include invisible nodes (visible:false) in results. Default false — hidden layers are skipped to reduce noise." },
                    "maxNodes": { "type": "number", "description": "Node budget for get_design/get_selection (default 3000) and scan_design (default 50000). Subtrees past it are summarized and meta.nodesTruncated is set — raise it for one big frame, lower it to keep the payload small." },
                    "precision": { "type": "string", "enum": ["exact", "rounded"], "description": "Geometry precision for read operations. Full/exact preserves fractional Figma coordinates; rounded is smaller and uses integer geometry." },
                    "absolute": { "type": "boolean", "description": "Force absoluteBoundingBox on every node in get_design/get_selection. Off by default: it is emitted only where parent-relative x/y is insufficient (inside a GROUP, or under rotation)." },
                    "inlineIcons": { "type": "boolean", "description": "Inline SVG markup for icon nodes in get_design (detail 'full', first 10 icons). Off by default — exporting SVG for many icons is slow." },
                    "withAnnotations": { "type": "boolean", "description": "For screenshot: export image with numbered bounding-box annotation overlays and metadata for multimodal LLM analysis." },
                    "keepViewport": { "type": "boolean", "description": "For screenshot/export_image: default true restores the user's canvas position after the render nudge. Pass false to leave the canvas on the exported node." },
                    "query": { "type": "string", "description": "Search text for search_* operations." },
                    "nodeType": { "type": "string", "description": "search_nodes: filter by node type (FRAME, TEXT, COMPONENT, INSTANCE...)." },
                    "styleType": { "type": "string", "enum": ["PAINT", "TEXT", "EFFECT"], "description": "search_styles: filter by style type." },
                    "collection": { "type": "string", "description": "search_variables: filter by collection name." },
                    "includeComponents": { "type": "boolean", "description": "refresh: also index the document-wide component catalogue." },
                    "sessionId": { "type": "string", "description": "Target a specific Figma file/tab when multiple are connected. Omit to auto-select." }
                },
                "required": ["operation"]
            }),
        },
        ToolDefinition {
            name: "figma_write".to_string(),
            description: "Execute JavaScript code to CREATE or MODIFY designs in Figma. ⚠️ MANDATORY: Call figma_docs BEFORE writing any design code. Skipping figma_docs causes hardcoded colors, wrong sizing, broken layouts, and low-quality UI. Use the `figma` proxy object — all methods return Promises, use async/await. Operations: create, modify, delete, clone, group, ungroup, flatten, resize, set_selection, set_viewport, batch (multiple ops in one call). Design Tokens: createVariableCollection, createVariable, setVariableValue, addVariableMode, renameVariableMode, removeVariableMode, applyVariable, setFrameVariableMode, clearFrameVariableMode, createPaintStyle, createTextStyle, createComponent. Prototyping: setReactions, getReactions, removeReactions (click/hover/press → navigate/overlay/swap with Smart Animate transitions). Scroll: setScrollBehavior (overflowDirection: NONE/HORIZONTAL/VERTICAL/BOTH). Variants: setComponentProperties, swapComponent, getComponentProperties. Component property definitions (master-side, required for instance text overrides to recalc auto-layout): addComponentProperty (TEXT/BOOLEAN/INSTANCE_SWAP), bindComponentPropertyToText, removeComponentProperty. The code runs in a sandboxed VM: no access to require, process, fs, fetch, or network.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "JavaScript using figma.create(), figma.modify(), figma.setPage(), etc."
                    },
                    "sessionId": {
                        "type": "string",
                        "description": "Target a specific Figma file/tab when multiple are connected. Omit to auto-select."
                    }
                },
                "required": ["code"]
            }),
        },
        ToolDefinition {
            name: "figma_docs".to_string(),
            description: "Get the API reference and design rules for figma_write. Call with no args first — returns quick-start guide + critical rules. Then load specific sections as needed: section='rules' (design principles, token rules, layer order, component-first), section='layout' (auto-layout, button/card/badge/progress/mobile rules), section='api' (create/modify/delete/clone/batch/read operations + workflow), section='tokens' (variables, multi-mode, paint styles, text styles), section='icons' (loadImage, loadIcon, loadIconIn, icon libraries, coloring, sizing). Always call figma_docs BEFORE any figma_write code.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "section": {
                        "type": "string",
                        "enum": ["rules", "layout", "api", "tokens", "icons"],
                        "description": "Which section to load. Omit (or null) for quick-start + critical rules. Load layout before any auto-layout work. Load api for full operation reference. Load tokens for variable/multi-mode work. Load icons for image/icon placement."
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "figma_prepare_design".to_string(),
            description: "⭐ PRIMARY & MANDATORY FIRST STEP when user gives a Figma link or asks to build/update frontend code from Figma. Single-call All-In-One Grounding Pack: (1) Captures high-res visual screenshot, (2) Auto-exports all SVG icons directly into your project assets folder with generated import statements, (3) Reads visible text through expanded, paginated instances with explicit coverage, and (4) Maps existing codebase components. Call THIS tool first instead of raw figma_read to eliminate icon guessing and missing UI elements. only='assets' just exports SVG icons (iconDir) and raster images (imageDir) with an index.ts barrel; only='components' just lists codebase components and matches nodeId against them.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "detail": {"type":"string","enum":["compact","full"],"description":"Default compact omits the duplicate colorPalette; full retains it."},
                    "outputPath": {"type":"string","description":"Save the grounding pack and raw design nodes to JSON outside model context; return coverage, screenshot path and warnings only."},
                    "nodeId": {
                        "type": "string",
                        "description": "Target Figma frame / screen node ID (omit to use active selection)."
                    },
                    "iconDir": {
                        "type": "string",
                        "description": "Project directory where SVG icons should be exported (e.g. 'src/assets/icons' or 'assets/images'). Default: 'src/assets/icons'."
                    },
                    "only": {"type": "string", "enum": ["assets", "components"], "description": "Run one part only: assets export or codebase component matching."},
                    "imageDir": {"type": "string", "description": "only=assets: directory for raster images (default 'public/images')."},
                    "createBarrel": {"type": "boolean", "description": "only=assets: write an index.ts barrel in iconDir (default true)."},
                    "projectDir": {
                        "type": "string",
                        "description": "Base directory of your project for component discovery (default: '.')."
                    },
                    "sessionId": {
                        "type": "string",
                        "description": "Target a specific Figma file/tab."
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "figma_to_code".to_string(),
            description: "Compile any Figma node (Frame, Component, Section, or selection) directly into clean, production-ready component code. Formats: 'react-tailwind' (default, React + Tailwind CSS), 'react-shadcn' (React + Shadcn/UI components & Radix), 'react-native' (React Native View/Text/TouchableOpacity), 'vue-tailwind' (Vue 3 SFC template + script setup), 'html' (plain HTML + Tailwind), 'swiftui' (SwiftUI VStack/HStack), 'clean-spec' (token-pruned AST for AI prompt context). Supports 'outputPath' to write directly to your codebase (e.g. 'src/components/UserProfileCard.tsx'). target='tokens' exports variables/styles as css (light/dark), tailwind, typescript, w3c or json (format). target='project' scaffolds a design system: globals.css, tailwind.config.ts and one component per local component (skips existing files unless overwrite).".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "nodeId": {
                        "type": "string",
                        "description": "Target Figma node ID. Omit to use the current canvas selection."
                    },
                    "nodeName": {
                        "type": "string",
                        "description": "Target node name (alternative to nodeId)."
                    },
                    "framework": {
                        "type": "string",
                        "enum": ["react-tailwind", "react-shadcn", "react-native", "vue-tailwind", "html", "swiftui", "clean-spec"],
                        "description": "Target framework: 'react-tailwind' (default), 'react-shadcn' (Shadcn/UI), 'react-native', 'vue-tailwind', 'html', 'swiftui', or 'clean-spec'. target=project supports react-tailwind, react-shadcn, vue-tailwind and html."
                    },
                    "componentName": {
                        "type": "string",
                        "description": "Custom component name (e.g. 'Navbar', 'ProfileCard'). Inferred from node name if omitted."
                    },
                    "outputPath": {
                        "type": "string",
                        "description": "Optional file path to save the generated component file directly into your codebase (e.g. 'src/components/Navbar.tsx')."
                    },
                    "target": {"type": "string", "enum": ["component", "tokens", "project"], "description": "component (default): one node to code. tokens: design tokens file. project: scaffold design system."},
                    "format": {"type": "string", "enum": ["css", "tailwind", "typescript", "w3c", "json"], "description": "target=tokens: output format (default css)."},
                    "collection": {"type": "string", "description": "target=tokens: filter by variable collection name."},
                    "mode": {"type": "string", "description": "target=tokens: one mode (e.g. Light, Dark); omit for all modes."},
                    "prefix": {"type": "string", "description": "target=tokens: CSS variable prefix."},
                    "projectDir": {"type": "string", "description": "target=project: project root; other paths are relative to it (default '.')."},
                    "componentDir": {"type": "string", "description": "target=project: component directory (default 'src/components/ui')."},
                    "stylesPath": {"type": "string", "description": "target=project: global CSS path (default 'src/styles/globals.css')."},
                    "tailwindConfigPath": {"type": "string", "description": "target=project: Tailwind config path (default 'tailwind.config.ts'; skipped for html unless given)."},
                    "overwrite": {"type": "boolean", "description": "target=project: replace existing files and components already in the codebase."},
                    "limit": {"type": "integer", "description": "target=project: max components, 1-100 (default 30)."},
                    "sessionId": {
                        "type": "string",
                        "description": "Target a specific Figma file/tab when multiple are connected."
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "figma_verify_ui".to_string(),
            description: "Validate and compare actual rendered HTML/React UI styles and layout against Figma design specifications. Checks dimensions, padding, flex gap, colors, border-radius, and typography, returning an exact match percentage, specific discrepancies, and actionable CSS/Tailwind fixes.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "nodeId": {
                        "type": "string",
                        "description": "Figma node ID to compare against (omit to use active selection)."
                    },
                    "nodeName": {
                        "type": "string",
                        "description": "Figma node name (alternative to nodeId)."
                    },
                    "computedStyles": {
                        "type": "object",
                        "description": "Computed CSS from the browser: font-size in px, font-weight numeric, font-family, plus layout/paint properties. For mixed text provide segments as an ordered array of CSS objects matching the Figma segments. Typography is checked per node/run without rounding to standard sizes."
                    },
                    "url": {
                        "type": "string",
                        "description": "Target localhost dev URL to inspect (optional context)."
                    },
                    "selector": {
                        "type": "string",
                        "description": "Target DOM selector inspected (optional context)."
                    },
                    "sessionId": {
                        "type": "string",
                        "description": "Target a specific Figma file/tab."
                    }
                },
                "required": []
            }),
        },
    ];
    for tool in &mut tools {
        if !matches!(tool.name.as_str(), "figma_status" | "figma_docs") {
            tool.input_schema["properties"]["taskId"] = json!({"type": "string", "description": "Frame task ID returned by figma_status task=start. Pins the tab and confines writes to its frame."});
        }
    }
    tools
}

/// Maps a public tool call onto the handler that serves it. Legacy tool names
/// (figma_index, figma_get_tokens, ...) are not listed but still dispatch unchanged.
pub fn route(name: &str, args: &mut serde_json::Value) -> String {
    let field = |key: &str| args[key].as_str().map(str::to_string);
    let internal = match (name, field("operation").or_else(|| field("task")).or_else(|| field("only")).or_else(|| field("target")).as_deref()) {
        ("figma_status", Some(action @ ("start" | "end" | "list"))) => { args["action"] = json!(action); "figma_task" }
        ("figma_read", Some("inspect_node")) => "figma_inspect_node",
        ("figma_read", Some("rules")) => "figma_rules",
        ("figma_read", Some("index_status")) => { args["operation"] = json!("status"); "figma_index" }
        ("figma_read", Some("search_components" | "search_styles" | "search_variables" | "subtree" | "typography" | "refresh")) => "figma_index",
        ("figma_prepare_design", Some("assets")) => "figma_export_assets",
        ("figma_prepare_design", Some("components")) => "figma_match_components",
        ("figma_to_code", Some("tokens")) => "figma_get_tokens",
        ("figma_to_code", Some("project")) => "figma_scaffold_project",
        _ => name,
    };
    internal.to_string()
}

/// Usage key shown in the plugin's Usage tab: the public tool plus its mode, e.g. "figma_read.inspect_node".
pub fn usage_key(name: &str, args: &serde_json::Value) -> String {
    match ["operation", "task", "only", "target"].iter().find_map(|key| args[*key].as_str()) {
        Some(mode) => format!("{name}.{mode}"),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{get_tools, route, usage_key};
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn tool_registry_has_unique_names_and_valid_schemas() {
        let tools = get_tools();
        let names: HashSet<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(names.len(), tools.len());
        assert_eq!(tools.len(), 7);
        assert!(tools.iter().all(|tool| {
            tool.name.starts_with("figma_")
                && tool.input_schema.get("type").and_then(|v| v.as_str()) == Some("object")
        }));
    }

    #[test]
    fn consolidated_calls_route_to_their_handlers() {
        let cases = [
            ("figma_status", json!({"task": "start", "frameId": "1:2"}), "figma_task"),
            ("figma_status", json!({}), "figma_status"),
            ("figma_read", json!({"operation": "inspect_node"}), "figma_inspect_node"),
            ("figma_read", json!({"operation": "rules"}), "figma_rules"),
            ("figma_read", json!({"operation": "search_styles"}), "figma_index"),
            ("figma_read", json!({"operation": "read_nodes"}), "figma_read"),
            ("figma_prepare_design", json!({"only": "assets"}), "figma_export_assets"),
            ("figma_prepare_design", json!({"only": "components"}), "figma_match_components"),
            ("figma_to_code", json!({"target": "tokens"}), "figma_get_tokens"),
            ("figma_to_code", json!({"target": "project"}), "figma_scaffold_project"),
            ("figma_to_code", json!({"target": "component"}), "figma_to_code"),
            ("figma_index", json!({"operation": "status"}), "figma_index"),
        ];
        for (name, mut args, expected) in cases {
            assert_eq!(route(name, &mut args), expected, "{name} {args}");
        }
        let mut args = json!({"task": "end", "taskId": "t"});
        route("figma_status", &mut args);
        assert_eq!(args["action"], "end");
        let mut args = json!({"operation": "index_status"});
        route("figma_read", &mut args);
        assert_eq!(args["operation"], "status");
        assert_eq!(usage_key("figma_read", &json!({"operation": "rules"})), "figma_read.rules");
        assert_eq!(usage_key("figma_write", &json!({"code": "x"})), "figma_write");
    }
}
