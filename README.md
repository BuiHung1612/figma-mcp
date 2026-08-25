# figma-mcp

High-performance, bidirectional Model Context Protocol (MCP) server written in **Rust** connecting AI coding assistants directly to **Figma Desktop**.

Enables AI agents (Claude Code, Cursor, Windsurf, Antigravity, VS Code, Zed) to **draw UI directly on the Figma canvas** and **read existing designs back** as structured JSON trees, design tokens, CSS, and screenshots — with zero external API keys needed.

---

## ⚡ Highlights

- **Pure Rust Native Performance**: Starts in `< 1ms`, uses `~3MB RAM`, zero GC pauses.
- **Bidirectional Bridge**: AI can both write UI (draw vectors, components, frames, autolayout) and read designs (extract hierarchy, compute styles, capture canvas).
- **Sandboxed JS Runtime**: Powered by **Boa** (pure Rust ECMAScript engine) for safe execution of design scripts.
- **Built-in Asset CDN**: Auto-fetches and normalizes SVG icons from 7 icon libraries (Ionicons, Lucide, Tabler, Bootstrap, Fluent, Phosphor).
- **Multi-Tab / Multi-Session Support**: Seamlessly connects to multiple open Figma files simultaneously.
- **100% Localhost Privacy**: All communication stays strictly on `127.0.0.1:38451` via HTTP long-polling.

---

## 🏛️ Architecture

```text
[ AI Assistant ]  (Claude Code / Cursor / Antigravity)
       │
       ▼  (Stdio JSON-RPC 2.0)
[ figma-mcp (Rust) ]
  ├── MCP Protocol Handler
  ├── Sandboxed JS Engine (Boa)
  ├── Asset & Icon Resolver
  └── Async HTTP Bridge (Axum / Tokio)
       │
       ▼  (HTTP Long-Polling: 127.0.0.1:38451)
[ Figma MCP Bridge Plugin ]
       │
       ▼  (Figma Plugin Scenegraph API)
[ Figma Canvas / Document ]
```

---

## 🚀 Quick Start

### 1. Build the Rust Server

Prerequisites: [Rust toolchain](https://rustup.rs/) (`cargo >= 1.80`).

```bash
git clone https://github.com/BuiHung1612/figma-mcp.git
cd figma-mcp
cargo build --release
```

The optimized static binary will be generated at `target/release/figma-mcp`.

---

### 2. Install the Figma Plugin

1. Open **Figma Desktop**.
2. Go to **Plugins** → **Development** → **Import plugin from manifest...**
3. Select the file: `plugin/manifest.json` from this repository.
4. Run the plugin (**Plugins** → **Development** → **Figma MCP Bridge**).
5. The plugin UI will show a green dot (**Connected**) once `figma-mcp` is running.

---

### 3. Configure Your MCP Client

#### Claude Code
Add to your Claude Code settings or run:
```bash
claude mcp add figma-mcp -- /path/to/figma-mcp/target/release/figma-mcp
```

#### Cursor / Windsurf / VS Code
Add to your `mcp.json` / `settings.json`:
```json
{
  "mcpServers": {
    "figma-mcp": {
      "command": "/path/to/figma-mcp/target/release/figma-mcp"
    }
  }
}
```

#### Google Antigravity
Add to your global or project `mcp_config.json`:
```json
{
  "mcpServers": {
    "figma-mcp": {
      "command": "/path/to/figma-mcp/target/release/figma-mcp"
    }
  }
}
```

---

## 🛠️ MCP Tools Reference

`figma-mcp` exposes 5 MCP tools:

### `figma_status`
Checks the live bridge connection status, connected Figma tabs, active file name, and latency statistics.

### `figma_write`
Executes JavaScript draw commands inside the sandboxed VM to build or modify designs on the Figma canvas.
- Supports Auto-layout (`layoutMode`, `itemSpacing`, `padding`), typography, fills, strokes, corner radius, drop shadows, and component creation.
- Supports icon loading: `figma.loadIcon("ionicons", "heart", { size: 24, fill: "#ff4757" })`.
- Supports image insertion: `figma.loadImage(url, { width: 300, height: 200 })`.

### `figma_read`
Extracts rich design data from the canvas back to the AI.
- `get_selection`: Returns node trees and CSS for currently selected elements.
- `get_design`: Extracts full or subtree layout hierarchy.
- `scan_design`: Compact scan with typography, color palette, and component list.
- `screenshot`: Renders canvas node directly into an inline image preview.
- `export_svg`: Exports vector markup for icons/illustrations.
- `get_variables`: Retrieves all Design Token collections and variable modes.
- `get_styles`: Retrieves paint styles and text styles.

### `figma_docs`
Fetches built-in documentation, design rules, layout guidelines, and code examples for `figma_write`.
- `section`: `"all"` | `"rules"` | `"layout"` | `"api"` | `"tokens"` | `"icons"`

### `figma_rules`
Audits the current Figma document and generates consistency rules and token definitions to guide AI generation.

---

## 💻 Development & Testing

```bash
# Run server directly in debug mode
cargo run

# Build release binary
cargo build --release

# Run automated MCP test suite
./scripts/test-rust-mcp.sh

# Rebuild Figma plugin bundle (if plugin-src/ is modified)
node scripts/build-plugin.js
```

---

## 📄 License

MIT © [BuiHung1612](https://github.com/BuiHung1612) — Free to use, modify, and distribute. See [LICENSE](LICENSE) for details.
