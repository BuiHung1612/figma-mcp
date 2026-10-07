# figma-rust-mcp

High-performance, bidirectional Model Context Protocol (MCP) server written in **Rust** connecting AI coding assistants directly to **Figma Desktop**.

Enables AI agents (Google Antigravity, Claude Code, Cursor, Windsurf, VS Code, Zed) to **compile Figma designs into production code**, **extract design tokens**, **draw UI directly on canvas**, and **inspect hierarchy, styles, and assets** — with zero external API keys needed.

---

## ⚡ Highlights

- **Lossless Rust Tree Compression**: Compact/minimal tree responses share identical style bundles when the complete JSON becomes smaller. Node IDs, hierarchy, text, geometry and state differences remain intact. Full detail bypasses Rust compression; no heuristic merging of screens or repeated items.
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

## 🏛️ Architecture & Zero-Touch Connection

```text
┌────────────────────────────────────────────────────────────────────────┐
│                      Terminal / Background Service                     │
│                       figma-rust-mcp (Pure Rust Engine)                     │
│  • MCP Streamable HTTP (/mcp) & legacy SSE (/sse, /message)             │
│  • Dynamic Runtime Server (/plugin/code.js, /plugin/ui.html)           │
│  • In-Memory Fast Index (<1ms Lookups & Incremental Diffs)             │
│  • Design-to-Code Compiler (React/Tailwind, Vue, RN, SwiftUI)          │
│  • Design Token Transformer (CSS, Tailwind, TypeScript, W3C DTCG)      │
│  • Sandboxed JS Runtime (Boa ECMAScript Engine)                        │
│  • Binary MessagePack & Progressive Subtree Chunk Receiver             │
└───────────────▲────────────────────────────────────────▲───────────────┘
                │ (ws://127.0.0.1:38451/ws)              │ (http://127.0.0.1:38451/mcp)
                │ (Dynamic Code Streaming & Hot-Reload)  │ (JSON-RPC 2.0 / Streamable HTTP)
      ┌─────────┴─────────┐                    ┌─────────┴─────────┐
      │   Figma Desktop   │                    │ AI Assistant(s)   │
      │ (Thin Loader)     │                    │ Google Antigravity│
      │                   │                    │ Cursor / Windsurf │
      │ • Zero-Touch Sync │                    │ Claude Code / Zed │
      │ • Live Hot-Reload │                    └───────────────────┘
      │ • SVG/PNG Render  │
      └───────────────────┘
```

### 🔌 How the Zero-Touch Connection Works:
1. **Permanent Thin Loader**: You only import the Figma plugin **once** (`~/.figma-rust-mcp/plugin/manifest.json`).
2. **Dynamic Runtime Streaming**: Upon launch, the Thin Loader contacts `http://127.0.0.1:38451/plugin/code.js` to fetch and execute the latest runtime in memory.
3. **Live WebSocket Hot-Reload**: The plugin connects to `ws://127.0.0.1:38451/ws`. When you upgrade `figma-rust-mcp` or restart the daemon, the plugin automatically detects the new server version and hot-reloads seamlessly — **no need to re-import or restart the plugin in Figma**.
4. **Auto-Reconnect & Offline Buffer**: If Figma is opened before the Rust service starts, the plugin displays a waiting screen and connects automatically the instant the service is up.

---

## 🚀 Quick Start

### 1. One-command setup with NPX (No Rust toolchain needed)

```bash
npx -y figma-rust-mcp@latest --init
```
The setup detects installed Codex, Claude Code, Antigravity, Cursor, Windsurf,
VS Code/Copilot, and Zed clients. Choose an action from the menu:

```text
1) Quick setup — plugin + background service + detected agents
2) Install/update Figma plugin only
3) Start server in this terminal
4) Configure detected agents
5) Check background service
6) Upgrade package and refresh setup
7) Remove background service
0) Exit
```

Choose **1** for the recommended setup. It downloads the platform binary,
installs the permanent plugin loader, starts the background service at login,
then lets you select detected agents (`a` for all, or comma-separated numbers)
and writes each client's MCP config while preserving existing servers. Restart
or reload those clients afterward. Then import the manifest into Figma once:
open **Plugins → Development → Import plugin from manifest...**, select
`~/.figma-rust-mcp/plugin/manifest.json`, and run **Figma Rust MCP Bridge**. Future
updates are streamed to the plugin automatically.

The runner automatically downloads the precompiled native binary for macOS
(Apple Silicon or Intel), Linux, or Windows. Use `--configure` as an alias for
`--init`.

For scripts and direct use, the individual options remain available:

```bash
npx -y figma-rust-mcp@latest --setup-plugin      # install/update plugin
npx -y figma-rust-mcp@latest --install-service   # plugin + background service
npx -y figma-rust-mcp@latest --service-status
npx -y figma-rust-mcp@latest --upgrade
npx -y figma-rust-mcp@latest --alias
npx -y figma-rust-mcp@latest --uninstall-service
```

Run `npx -y figma-rust-mcp@latest` to launch the interactive server directly.

---

### Windows background mode

Double-clicking `figma-rust-mcp.exe` starts the server in a detached process and closes
its launcher console. From PowerShell or Command Prompt, use:

```powershell
.\figma-rust-mcp.exe --background
```

The server keeps running after the terminal closes. Logs are appended to
`%LOCALAPPDATA%\figma-rust-mcp\logs\server.log`. Use `--server` for the foreground
terminal dashboard, or `--stdio` for an MCP client subprocess.

`--install-service` through the npm runner configures background startup at login.
Re-run the installation command to update an existing Windows task. Removing the
task disables future login startup; stop an already detached server through Task
Manager (`figma-rust-mcp.exe`). The launcher console may appear briefly when opening
the executable; the server itself does not retain a console window.

### 2. Build from Source (Optional)

If you prefer building directly with the [Rust toolchain](https://rustup.rs/) (`cargo >= 1.80`):

```bash
git clone https://github.com/BuiHung1612/figma-mcp.git
cd figma-rust-mcp
cargo build --release
./target/release/figma-rust-mcp
```

---

### 3. Configure Your MCP Client

The `--init` quick setup can add the local server to detected clients. To
configure one manually, use the matching section below.

#### Codex and clients using Streamable HTTP (Recommended)

Use the `/mcp` endpoint. It accepts `POST` JSON-RPC requests directly:

```toml
[mcp_servers.figma-rust-mcp]
url = "http://127.0.0.1:38451/mcp"
```

For Codex CLI, the equivalent command is:

```bash
codex mcp add figma-rust-mcp --url http://127.0.0.1:38451/mcp
```

#### Google Antigravity (SSE Transport)
Add to your `~/.gemini/config/mcp_config.json` or project `.agents/mcp_config.json`:
```json
{
  "mcpServers": {
    "figma-rust-mcp": {
      "serverUrl": "http://127.0.0.1:38451/sse"
    }
  }
}
```

#### Claude Code / Cursor / Windsurf / VS Code / Zed

These clients can use the Streamable HTTP `/mcp` endpoint:
```json
{
  "mcpServers": {
    "figma-rust-mcp": {
      "url": "http://127.0.0.1:38451/mcp"
    }
  }
}
```

For Zed, place the server entry under `context_servers` in `settings.json`.
Alternatively, configure a stdio subprocess via NPX:
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

### 6. Diagnose a stale Figma runtime

Check the runtime bundle currently served to Figma Desktop:

```bash
curl http://127.0.0.1:38451/plugin/version
```

The response includes `version` and `runtimeHash`. If the hash changes after an
upgrade, the dynamic plugin runtime is refreshed automatically. Restart the
Figma plugin only if the reported hash does not change.

---

## 🛠️ MCP Tools Reference

`figma-rust-mcp` provides **15 first-class MCP tools**:

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

Token exports use schema v2 and preserve full paint/effect stacks, alpha, gradient
transforms, collection modes and raw alias values. CSS, Tailwind, TypeScript and
JSON share one normalized model. The `w3c` output uses DTCG 2025.10 color,
dimension, gradient-stop and shadow values, with Figma-specific geometry and
unsupported values retained in `$extensions`.

Linear gradients on a node are projected using its actual dimensions. A diagonal
gradient style has no target aspect ratio: use `get_css` on its consuming node.
Radial/angular/diamond paints, unsupported blend modes and progressive blurs
retain their raw data and report diagnostics recommending SVG instead of emitting
approximate CSS. Unknown modes, unresolved aliases and sanitized name collisions
fail explicitly. CSS exports include non-default modes under `data-theme`
selectors; opacity/font-weight variables stay unitless.

The plugin UI badge is stamped from the build version and updated from the
connected server handshake. Package/Cargo/tag mismatches fail the build pipeline.
Run `npm run build:plugin` after changing plugin source, and `npm run test:tokens`
for the token/contract/version regression suite. Existing background servers need
a restart with the new binary to load the updated runtime.


### 5. `figma_get_selection`
Inspection of currently selected layers/frames, with shared-style compression when it reduces response size.
- **`detail`**: `"compact"` (default), `"minimal"`, `"full"`.

Compact/minimal `get_selection` and `get_design` responses may contain a root
`_compression` object with `version: 1` and a `styles` dictionary. For each node
with `styleRef`, merge `_compression.styles[node.styleRef]` into that node and
remove `styleRef`; remove root `_compression` to reconstruct the plugin payload
exactly. Style references are dictionary lookups, not inherited CSS. Payloads
with reserved-field collisions or no net byte savings stay inline. `detail: "full"` bypasses this Rust transformation and queries the plugin rather than a
compact selection cache. Plugin detail, visibility, precision, depth and node
budgets still determine the source payload; compression cannot restore fields
omitted there. Code generation continues to consume inline styles.

No token-reduction or AI-latency percentage is claimed without a tokenizer and
end-to-end benchmark on real designs.
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
  - `"subtree"`: Read/cache one `nodeId`, default depth 2 and 1000 nodes. Increase `depth` or `maxNodes` only for the section you need; check `meta` for truncation.
  - `"typography"`: Rust-generated text/font table for one `nodeId`, with mixed-style runs preserved. Default limit 200 rows. Null font fields mean unknown/mixed, not a guessed default.

`status` includes per-tab dispatch cache hits/misses, bridge calls and total bridge wait time, plus process-wide tool call counts and total/average milliseconds. Node changes invalidate related subtrees; style changes, unknown events and global writes invalidate the whole cache. Detailed subtree data is loaded on demand. The initial index remains shallow.

For typography verification, call `figma_verify_ui` with a text `nodeId` and browser `computedStyles` (`font-size` in px, numeric `font-weight`, `font-family`). Mixed text accepts `computedStyles.segments`, an ordered array of CSS objects matching the Figma runs. Size checks preserve fractional pixels.

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

### Multiple tabs and frame tasks

Each plugin runtime has its own `sessionId`; tabs of the same file share a
`documentId`. Use `figma_status` to discover tabs. With multiple connected tabs,
pass an exact `sessionId`; the server rejects ambiguous routing.

For multiple agents editing one file, call `figma_task` with `action: "start"`,
`sessionId` and `frameId`, then pass the returned `taskId` on subsequent tools.
Each task reserves a separate FRAME directly under a PAGE or SECTION, without
shared component definitions. Writes stay inside that frame; global styles,
variables, page and selection changes are blocked. A frame cannot be reserved
twice, including across tabs of the same document. Release it with `figma_task`
using `action: "end"` and `taskId`; restart tasks after a plugin reload.

Tools on one tab run sequentially, including every awaited operation in a
`figma_write` call. Different tabs run independently. `npm run test:sessions`
(Node >=22) checks real WebSocket/HTTP transport with a simulated plugin canvas;
it does not replace validation in Figma Desktop.

### Startup indexing

Opening the bridge indexes top-level nodes on the active page and local tokens.
It does not preload every page or scan the document-wide component catalogue.
`figma_index` status exposes `components_indexed: false` until a full reindex;
this means deferred, not an empty catalogue. Live component queries populate
the Rust cache for subsequent reads while the index remains valid. Component reads/searches and rules
that require the complete catalogue fall back to a live query.

The plugin listens to active-page `nodechange` and global `stylechange` events,
and rebuilds the lightweight index when the active page changes. It does not
subscribe to document-wide node changes by preloading the whole file. Explicit
reindexing still includes all local components; discovery loads pages one at a
time and yields after at most 100 nodes or an 8ms traversal slice. Figma's native
page load itself cannot be interrupted by the plugin. Concurrent identical
index requests share one scan, and results from a previously active page are
not published as the current index.

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
