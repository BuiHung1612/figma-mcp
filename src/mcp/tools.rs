use super::protocol::ToolDefinition;
use serde_json::json;

pub fn get_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "figma_status".to_string(),
            description: "Check whether the Figma plugin bridge is connected. Always call this first to confirm the plugin is running before any other tool.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
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
            name: "figma_get_selection".to_string(),
            description: "Get the design tree of currently selected element(s) in Figma with smart token compression. Fast single-call inspection for UI building.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "depth": {
                        "type": "string",
                        "description": "Tree depth: number (default 10) or 'full'."
                    },
                    "detail": {
                        "type": "string",
                        "enum": ["minimal", "compact", "full"],
                        "description": "Detail level: 'compact' (recommended default), 'full', 'minimal'."
                    },
                    "maxNodes": {
                        "type": "number",
                        "description": "Node budget (default 3000). Subtrees past it are summarized and meta.nodesTruncated is set."
                    },
                    "absolute": {
                        "type": "boolean",
                        "description": "Force absoluteBoundingBox on every node (emitted automatically inside groups / under rotation)."
                    },
                    "includeHidden": {
                        "type": "boolean",
                        "description": "Include invisible nodes (visible:false). Default false."
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
            name: "figma_inspect_node".to_string(),
            description: "Inspect a specific Figma node by ID (or name) — returns CSS styles, flex layout, tokens, typography, and fills in a clean format.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "nodeId": {
                        "type": "string",
                        "description": "The Figma node ID (e.g. '2413:27687')."
                    },
                    "nodeName": {
                        "type": "string",
                        "description": "The node name (alternative to nodeId)."
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
            name: "figma_export_asset".to_string(),
            description: "Export any Figma node as PNG, JPG, or SVG. Specify 'outputPath' to automatically save directly to a project folder (e.g. 'src/assets/logo.png') without bloating chat tokens with base64 data.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "nodeId": {
                        "type": "string",
                        "description": "Target node ID (omit to use current selection)."
                    },
                    "nodeName": {
                        "type": "string",
                        "description": "Target node name (alternative to nodeId)."
                    },
                    "format": {
                        "type": "string",
                        "enum": ["png", "jpg", "svg"],
                        "description": "Export format: 'png' (default), 'jpg', or 'svg'."
                    },
                    "scale": {
                        "type": "number",
                        "description": "Export scale factor (default 2 for high-res PNG/JPG, ignored for SVG)."
                    },
                    "outputPath": {
                        "type": "string",
                        "description": "Optional file path to save the exported image/SVG directly to disk (e.g. 'assets/images/splash.png'). Parent directories are created automatically."
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
            name: "figma_read".to_string(),
            description: "READ design data from Figma — extract node trees, colors, typography, spacing, and screenshots. Use to understand an existing design before generating code, or to inspect what's on the canvas.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": [
                            "get_selection", "get_design", "get_page_nodes", "screenshot", "export_svg",
                            "get_styles", "get_local_components", "get_viewport", "get_variables",
                            "get_node_detail", "get_css", "get_design_context", "get_component_map",
                            "get_unmapped_components", "export_image", "search_nodes", "scan_design"
                        ],
                        "description": "── Design-to-code (use these for code generation) ──\nget_design_context: AI-optimized payload for a node — flex layout, token-resolved colors, typography with style names, component instances with variant properties. Best single call for design→React/Vue/Swift code.\nget_css: ready-to-use CSS string for a single node — background, flex, border, radius, shadow, typography, opacity, transform.\nget_component_map: list every component instance in a frame with componentSetName, variantLabel, properties, and suggestedImport path. Use to scaffold import statements.\nget_unmapped_components: find component instances that have no description in Figma (likely no code mapping yet). Prompts AI to ask user for correct import paths.\n── Inspect ──\nget_node_detail: structured properties for a single node — fills, bound variables (resolved to name+value), style refs (resolved to name+hex), instance overrides (full field list), componentSetName/variantLabel.\nget_selection: full design tree of selected node(s) + design tokens summary.\nget_design: full node tree for a frame/page (depth param: number or 'full'). Multi-style TEXT reports real per-run values plus `segments`; nodes inside groups or under rotation also carry `absoluteBoundingBox`; capped subtrees carry `childrenTruncated` and meta.nodesTruncated.\nget_page_nodes: top-level frames on the current page.\n── Styles & tokens ──\nget_styles: all local paint, text, effect, grid styles.\nget_variables: all local Design Token variables — collections, modes, resolved values.\nget_local_components: component listing with descriptions + variant property definitions.\n── Export ──\nscreenshot: PNG of a node — displays inline in Claude Code.\nexport_svg: SVG markup string.\nexport_image: base64 PNG/JPG for saving to disk (scale param for resolution).\n── Search ──\nsearch_nodes: filter by type, namePattern (wildcard *), fill color, fontFamily, fontSize, hasImage, hasIcon.\nscan_design: structured summary of large frames — all text, colors, fonts, images, icons, sections. Every capped list is paired with real counts in `totals` and flags in `truncated`.\n── Viewport ──\nget_viewport: current viewport center, zoom, bounds."
                    },
                    "nodeId": { "type": "string", "description": "Target node ID (optional — omit to use current selection)." },
                    "nodeName": { "type": "string", "description": "Target node name (alternative to nodeId)." },
                    "scale": { "type": "number", "description": "Export scale for screenshot / export_image (default 1 for screenshot, 2 for export_image)." },
                    "depth": { "type": "string", "description": "Tree depth for get_design/get_selection. Number (default 10) or 'full' for unlimited. Higher = more detail but larger output." },
                    "format": { "type": "string", "description": "Image format for export_image: 'png' (default) or 'jpg'." },
                    "detail": { "type": "string", "description": "Detail level for get_design/get_selection: 'minimal' (~5% tokens), 'compact' (~30%), 'full' (default, 100%). Use minimal for large files." },
                    "outputPath": { "type": "string", "description": "Optional file path to save exported SVG/image directly to disk (for export_svg, export_image, or screenshot)." },
                    "includeHidden": { "type": "boolean", "description": "Include invisible nodes (visible:false) in results. Default false — hidden layers are skipped to reduce noise." },
                    "maxNodes": { "type": "number", "description": "Node budget for get_design/get_selection (default 3000) and scan_design (default 50000). Subtrees past it are summarized and meta.nodesTruncated is set — raise it for one big frame, lower it to keep the payload small." },
                    "absolute": { "type": "boolean", "description": "Force absoluteBoundingBox on every node in get_design/get_selection. Off by default: it is emitted only where parent-relative x/y is insufficient (inside a GROUP, or under rotation)." },
                    "inlineIcons": { "type": "boolean", "description": "Inline SVG markup for icon nodes in get_design (detail 'full', first 10 icons). Off by default — exporting SVG for many icons is slow." },
                    "keepViewport": { "type": "boolean", "description": "For screenshot/export_image: default true restores the user's canvas position after the render nudge. Pass false to leave the canvas on the exported node." },
                    "sessionId": { "type": "string", "description": "Target a specific Figma file/tab when multiple are connected. Omit to auto-select." }
                },
                "required": ["operation"]
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
            name: "figma_rules".to_string(),
            description: "Generate a design system rule sheet from the current Figma file — aggregates color tokens, typography styles, variables (all modes), and component catalog into a single markdown block. Equivalent to official Figma MCP's create_design_system_rules. Call once at the start of a design-to-code session to give the AI full context: what tokens to use, what text styles exist, which components are available. Re-run when the design system changes.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Target a specific Figma file/tab. Omit to auto-select."
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "figma_index".to_string(),
            description: "Instant <1ms in-memory queries against the pre-indexed Figma file (nodes, components, design tokens, styles). Avoids slow roundtrips to the Figma canvas. Operations: 'status' (view index health and node counts), 'search_nodes' (instant text/type search across all nodes), 'get_node' (instant node lookup by id), 'search_components' (find component sets & variants), 'search_styles' (find paint & text styles), 'search_variables' (find design tokens), 'refresh' (trigger background re-index of canvas).".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["status", "search_nodes", "get_node", "search_components", "search_styles", "search_variables", "refresh"],
                        "description": "Index operation to perform."
                    },
                    "query": {
                        "type": "string",
                        "description": "Search query text (for search_nodes, search_components, search_styles, search_variables)."
                    },
                    "nodeId": {
                        "type": "string",
                        "description": "Node ID to look up (for get_node)."
                    },
                    "nodeType": {
                        "type": "string",
                        "description": "Filter by node type (e.g. 'FRAME', 'TEXT', 'COMPONENT', 'INSTANCE') for search_nodes."
                    },
                    "styleType": {
                        "type": "string",
                        "enum": ["PAINT", "TEXT", "EFFECT"],
                        "description": "Filter by style type for search_styles."
                    },
                    "collection": {
                        "type": "string",
                        "description": "Filter by variable collection name for search_variables."
                    },
                    "limit": {
                        "type": "number",
                        "description": "Max results to return (default 30)."
                    },
                    "sessionId": {
                        "type": "string",
                        "description": "Target a specific Figma file/tab. Omit to auto-select."
                    }
                },
                "required": ["operation"]
            }),
        },
    ]
}
