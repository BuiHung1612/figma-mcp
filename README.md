# figma-mcp (figma-rust-mcp)

High-performance, bidirectional Model Context Protocol (MCP) server written in **Rust** connecting AI coding assistants directly to **Figma Desktop**.

Enables AI agents (Google Antigravity, Claude Code, Cursor, Windsurf, VS Code, Zed) to **compile Figma designs into production code**, **extract design tokens**, **draw UI directly on canvas**, and **inspect hierarchy, styles, and assets** — with zero external API keys needed.

---

## ⚡ Highlights

- **Pure Rust Native Performance**: Starts in `< 1ms`, uses `~3MB RAM`, zero GC pauses.
- **In-Memory Deep Indexing (`figma_index`)**: Queries layers, components, styles, and tokens in `< 1ms` without slow canvas roundtrips.
- **Binary IPC & Chunk Streaming**: Powered by **MessagePack** (`rmp-serde`) and progressive subtree chunking for instant transfers of massive design files.
- **Incremental Diff Updates**: Sub-millisecond live document sync (`upsert_node`) keeps the server index fresh as you edit in Figma.
- **Instant Design-to-Code (`figma_to_code`)**: Compiles Figma frames into clean, semantic components (**React + Tailwind**, **React Native**, **Vue 3 + Tailwind**, **HTML**, **SwiftUI**).
- **Design Token Exporter (`figma_get_tokens`)**: Exports variables and styles to **Tailwind config**, **CSS custom properties** (`:root` & dark mode), **TypeScript consts**, or **W3C DTCG Token Studio JSON**.
- **Batch Asset Extractor (`figma_export_assets` / `figma_export_asset`)**: Extracts SVG icons and raster images directly to project folders (`outputPath`) with auto-generated TypeScript barrel exports (`index.ts`) — zero chat token bloating.
- **Sandboxed JS Drawing Engine (`figma_write`)**: Powered by **Boa** (pure Rust ECMAScript engine) with built-in asset resolvers and 7 icon packs (*Ionicons, Lucide, Tabler, Bootstrap, Fluent, Phosphor*).
- **Multi-Tab / Multi-Session Support**: Seamlessly connects to multiple open Figma files simultaneously.
- **100% Localhost Privacy**: All communication stays strictly on `127.0.0.1:38451`.

---

## 🏛️ Architecture

```text
┌────────────────────────────────────────────────────────────────────────┐
│                      Terminal / Standalone Service                     │
│                       figma-mcp (Pure Rust Engine)                     │
│  • MCP SSE Transport (/sse, /message) & Direct HTTP (/mcp)             │
│  • In-Memory Fast Index (<1ms Lookups & Incremental Diffs)             │
│  • Design-to-Code Compiler (React/Tailwind, Vue, RN, SwiftUI)          │
│  • Design Token Transformer (CSS, Tailwind, TypeScript, W3C DTCG)      │
│  • Sandboxed JS Runtime (Boa ECMAScript Engine)                        │
│  • Binary MessagePack & Progressive Subtree Chunk Receiver             │
└───────────────▲────────────────────────────────────────▲───────────────┘
                │ (ws://127.0.0.1:38451)                 │ (http://127.0.0.1:38451/sse)
                │ (MessagePack / JSON IPC)               │
      ┌─────────┴─────────┐                    ┌─────────┴─────────┐
      │   Figma Desktop   │                    │ AI Assistant(s)   │
      │ (Plugin Bridge)   │                    │ Google Antigravity│
      │                   │                    │ Cursor / Windsurf │
      │ • Canvas Tracker  │                    │ Claude Code / Zed │
      │ • SVG/PNG Render  │                    └───────────────────┘
      └───────────────────┘
```

---

## 🚀 Quick Start

### 1. One-Line Launch with NPX (No Rust toolchain needed)

Launch `figma-rust-mcp` on macOS, Linux, or Windows with zero manual setup:

```bash
npx figma-rust-mcp
```

This automatically downloads the prebuilt native binary for your OS and architecture and starts the server.

---

### 2. Auto-Start as Background Service (Recommended for SSE)

To make `figma-rust-mcp` start automatically upon OS login and run in the background across all your AI IDEs:

```bash
# Install and start auto-start background service (macOS LaunchAgent, Linux systemd, Windows Task Scheduler)
npx figma-rust-mcp --install-service

# Update/Upgrade to the latest version and hot-reload background service
npx -y figma-rust-mcp@latest --install-service

# Check service status
npx figma-rust-mcp --service-status

# Uninstall background service
npx figma-rust-mcp --uninstall-service
```

---

### 3. Build from Source (Rust)

Prerequisites: [Rust toolchain](https://rustup.rs/) (`cargo >= 1.80`).

```bash
git clone https://github.com/BuiHung1612/figma-mcp.git
cd figma-mcp
cargo build --release
./target/release/figma-mcp
```

---

### 4. Install the Figma Plugin (Permanent One-Time Setup)

`figma-rust-mcp` uses a **Dynamic Thin Loader** architecture. You only import it once — all future UI updates, bugfixes, and features stream dynamically from the local Rust server:

```bash
# Setup permanent plugin folder (~/.figma-mcp/plugin)
npx figma-rust-mcp --setup-plugin
```

1. Open **Figma Desktop**.
2. Go to **Plugins** → **Development** → **Import plugin from manifest...**
3. Select `~/.figma-mcp/plugin/manifest.json` (or `plugin/manifest.json` from this repo).
4. Run the plugin (**Plugins** → **Development** → **Figma MCP Bridge**).
5. Done! You never need to reinstall or re-import the plugin on future updates.

---

### 5. Configure Your MCP Client

#### Google Antigravity (SSE Transport - Recommended)
Add to your `~/.gemini/config/mcp_config.json` or project `.agents/mcp_config.json`:
```json
{
  "mcpServers": {
    "figma-mcp": {
      "serverUrl": "http://127.0.0.1:38451/sse"
    }
  }
}
```

#### Claude Code / Cursor / Windsurf / VS Code / Zed

**Option A: SSE Transport (Recommended for background service)**
```json
{
  "mcpServers": {
    "figma-mcp": {
      "url": "http://127.0.0.1:38451/sse"
    }
  }
}
```

**Option B: Stdio Subprocess (via NPX)**
```json
{
  "mcpServers": {
    "figma-rust-mcp": {
      "command": "npx",
      "args": ["-y", "figma-rust-mcp", "--stdio"]
    }
  }
}
```

---

## 🛠️ MCP Tools Reference

`figma-mcp` provides **15 first-class MCP tools**:

### 1. `figma_status`
Checks live bridge connection status, connected Figma tabs/files, in-memory index health, queue length, and latency statistics.

### 2. `figma_inspect_node`
Inspects a specific node by ID or name, returning CSS styles, flex layout rules, tokens, typography, and fills in a clean format (served in `< 1ms` via in-memory index or direct bridge).

### 3. `figma_to_code`
Compiles any Figma node (Frame, Component, Section, or selection) directly into clean, production-ready component code.
- **`framework`**: `"react-tailwind"` (default), `"react-native"`, `"vue-tailwind"`, `"html"`, `"swiftui"`.
- **`outputPath`**: Directly saves the generated component to your codebase (e.g. `src/components/UserProfileCard.tsx`).
- **`componentName`**: Custom component name override.

### 4. `figma_get_tokens`
Exports Figma Variables (Design Tokens), Color Styles, Typography Styles, and Elevation/Shadow Styles directly into frontend code.
- **`format`**: `"tailwind"` (`tailwind.config.js` theme.extend), `"css"` (`:root` CSS custom properties with light/dark theme modes), `"typescript"` (`tokens.ts`), `"w3c"` (W3C DTCG Token Studio JSON), or `"json"`.
- **`outputPath`**: Writes directly to disk (e.g. `src/styles/tokens.css` or `tailwind.config.js`).
- **`collection`** / **`mode`** / **`prefix`**: Optional filters and naming prefixes.

### 5. `figma_get_selection`
Token-compressed inspection of currently selected layers/frames on the Figma canvas. Returns compacted layout and typography hierarchy with **60–80% fewer tokens**.
- **`detail`**: `"compact"` (recommended), `"minimal"`, `"full"`.
- **`depth`**: Tree depth limit or `"full"`.

### 6. `figma_export_asset`
Exports any Figma node/layer directly into PNG, JPG, or SVG.
- **`outputPath`**: When specified (e.g. `src/assets/logo.png`), saves directly to disk and returns minimal JSON metadata instead of bloating context with large base64 strings.
- **`format`**: `"png"` (default), `"jpg"`, `"svg"`.
- **`scale`**: Scaling factor (default `2` for high-res).

### 7. `figma_export_assets`
Batch extracts all SVG icons and PNG images from a Figma frame/page directly into project directories.
- **`iconDir`**: Directory path for SVG icons (e.g. `src/assets/icons`).
- **`imageDir`**: Directory path for raster images (e.g. `public/images`).
- **`createBarrel`**: Automatically creates an `index.ts` barrel export file in `iconDir`.

### 8. `figma_index`
Instant `< 1ms` in-memory queries against pre-indexed Figma file structures.
- **`operation`**:
  - `"status"`: View index health and node counts.
  - `"search_nodes"`: Search nodes by text name, query, and type (`FRAME`, `TEXT`, `COMPONENT`, `INSTANCE`).
  - `"get_node"`: Instant node lookup by ID.
  - `"search_components"`: Find component sets and variants.
  - `"search_styles"`: Find paint, text, and effect styles.
  - `"search_variables"`: Find design token variables by name or collection.
  - `"refresh"`: Trigger full background re-indexing of the canvas.

### 9. `figma_read`
Universal reader for advanced queries:
- **Design-to-code**: `get_design_context`, `get_css`, `get_component_map`, `get_unmapped_components`.
- **Inspection & Hierarchy**: `get_selection`, `get_design`, `get_page_nodes`, `get_node_detail`, `scan_design`, `search_nodes`.
- **Design Systems**: `get_styles`, `get_variables`, `get_local_components`, `get_tokens`.
- **Visuals & Canvas**: `screenshot`, `export_svg`, `export_image`, `get_viewport`.

### 10. `figma_write`
Executes JavaScript draw commands inside the sandboxed VM to build or modify designs on the Figma canvas.
- Supports Auto-layout (`layoutMode`, `itemSpacing`, `padding`), typography, fills, strokes, corner radius, drop shadows, and component creation.
- Supports icon loading: `figma.loadIcon("ionicons", "heart", { size: 24, fill: "#ff4757" })`.
- Supports image insertion: `figma.loadImage(url, { width: 300, height: 200 })`.
- Supports Design Tokens: `createVariableCollection`, `createVariable`, `addVariableMode`, `applyVariable`.
- Supports Prototyping & Reactions: `setReactions`, `getReactions`, `setScrollBehavior`.

### 11. `figma_rules`
Audits the current Figma document and generates a complete design system rule sheet (color tokens, typography styles, variables, component catalog) in `< 1ms` from cache.

### 12. `figma_docs`
Fetches built-in documentation, design rules, layout guidelines, and code examples for `figma_write`.
- **`section`**: `"rules"` | `"layout"` | `"api"` | `"tokens"` | `"icons"`

### 13. `figma_verify_ui`
Validates and compares actual rendered HTML/React UI styles and layout against Figma design specifications.
- Checks dimensions, padding, flex gap, colors, border-radius, and typography.
- Returns an exact match percentage (`match_percentage`), discrepancy breakdown, and actionable CSS/Tailwind fixes.
- **`nodeId`** / **`nodeName`**: Target Figma node spec.
- **`computedStyles`**: Key-value pairs of computed CSS properties from browser/component inspection.
- **`url`** / **`selector`**: Contextual target URL or CSS selector inspected.

### 14. `figma_match_components`
Scans your local codebase directories (`src/components`, `components/ui`) to discover existing React/Vue components and matches them directly against Figma design layers, preventing duplicate component creation and ensuring reuse of existing design systems.
- **`projectDir`**: Base directory of your project (default: current directory `"."`).
- **`nodeId`**: Optional Figma node ID to check specific match against local components.

### 15. `figma_prepare_design`
**All-In-One Multimodal Grounding Pack for AI**: The ultimate single-call tool for code generation that eliminates missing UI elements and icon guesswork.
- Simultaneously extracts **100% of visible text elements** (greeting, badges, timestamps, labels).
- Automatically **exports all SVG vector icons** into your local project directory (`iconDir`) and provides exact TypeScript import statements.
- Captures high-res **visual canvas screenshots** saved locally for immediate multimodal LLM inspection.
- Discovers existing components in your codebase for direct reuse.
- Generates an actionable **Implementation Checklist** ensuring complete design fidelity in 1 single shot.
- **`nodeId`**: Target Figma screen/frame ID.
- **`iconDir`**: Target directory for SVG icons (e.g. `src/assets/icons` or `assets/images`).
- **`projectDir`**: Base directory for component discovery.

---

## 💻 Development & Testing

```bash
# Run server in debug mode
cargo run

# Build optimized release binary
cargo build --release

# Run automated MCP test suite
./scripts/test-rust-mcp.sh

# Rebuild Figma plugin bundle (if plugin-src/ is modified)
node scripts/build-plugin.js
```

---

## 📄 License

MIT © [BuiHung1612](https://github.com/BuiHung1612) — Free to use, modify, and distribute. See [LICENSE](LICENSE) for details.
