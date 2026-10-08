# figma-rust-mcp

High-performance, bidirectional Model Context Protocol (MCP) server written in **Rust** connecting AI coding assistants directly to **Figma Desktop**.

Enables AI agents (Google Antigravity, Claude Code, Cursor, Windsurf, VS Code, Zed) to **compile Figma designs into production code**, **extract design tokens**, **draw UI directly on canvas**, and **inspect hierarchy, styles, and assets** — with zero external API keys needed.

---

## ⚡ Highlights

- **Lossless Rust Tree Compression**: Compact/minimal tree responses share identical style bundles when the complete JSON becomes smaller. Node IDs, hierarchy, text, geometry and state differences remain intact. Full detail bypasses Rust compression; no heuristic merging of screens or repeated items.
- **Pure Rust Native Performance**: Single native binary, no runtime or GC. Measure it on your machine with `npm run bench` or the plugin's **Bench** tab (see [Benchmarks](#-benchmarks)).
- **Scoped In-Memory Index (`figma_index`)**: Indexes page roots first and loads frame descendants on demand. Search reports the indexed scope and completeness.
- **Binary IPC & Chunk Streaming**: Powered by **MessagePack** (`rmp-serde`) and progressive subtree chunking for instant transfers of massive design files.
- **Revisioned Updates**: Property patches update affected nodes; revision gaps trigger a fresh snapshot.
- **Instant Design-to-Code (`figma_to_code`)**: Compiles Figma frames into clean, semantic components (**React + Tailwind**, **React Native**, **Vue 3 + Tailwind**, **HTML**, **SwiftUI**).
- **Design Token Exporter (`figma_get_tokens`)**: Exports variables and styles to **Tailwind config**, **CSS custom properties** (`:root` & dark mode), **TypeScript consts**, or **W3C DTCG Token Studio JSON**.
- **Batch Asset Extractor (`figma_export_assets` / `figma_export_asset`)**: Extracts SVG icons and raster images directly to project folders (`outputPath`) with auto-generated TypeScript barrel exports (`index.ts`) — zero chat token bloating.
- **Sandboxed JS Drawing Engine (`figma_write`)**: Powered by **Boa** (pure Rust ECMAScript engine) with built-in asset resolvers and 7 icon packs (*Ionicons, Lucide, Tabler, Bootstrap, Fluent, Phosphor*).
- **Multi-Tab / Multi-Session Support**: Seamlessly connects to multiple open Figma files simultaneously.
- **100% Localhost Privacy**: All communication stays strictly on `127.0.0.1:41730`.

---

## 🏛️ Architecture & Zero-Touch Connection

```text
┌────────────────────────────────────────────────────────────────────────┐
│                      Terminal / Background Service                     │
│                       figma-rust-mcp (Pure Rust Engine)                     │
│  • MCP Streamable HTTP (/mcp) & legacy SSE (/sse, /message)             │
│  • Dynamic Runtime Server (/plugin/code.js, /plugin/ui.html)           │
│  • In-Memory Fast Index (Scoped Lookups & Incremental Diffs)           │
│  • Design-to-Code Compiler (React/Tailwind, Vue, RN, SwiftUI)          │
│  • Design Token Transformer (CSS, Tailwind, TypeScript, W3C DTCG)      │
│  • Sandboxed JS Runtime (Boa ECMAScript Engine)                        │
│  • Binary MessagePack & Progressive Subtree Chunk Receiver             │
└───────────────▲────────────────────────────────────────▲───────────────┘
                │ (ws://127.0.0.1:41730/ws)              │ (http://127.0.0.1:41730/mcp)
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
2. **Dynamic Runtime Streaming**: Upon launch, the Thin Loader contacts `http://127.0.0.1:41730/plugin/code.js` to fetch and execute the latest runtime in memory.
3. **Live WebSocket Hot-Reload**: The plugin connects to `ws://127.0.0.1:41730/ws`. When you upgrade `figma-rust-mcp` or restart the daemon, the plugin automatically detects the new server version and hot-reloads seamlessly — **no need to re-import or restart the plugin in Figma**.
4. **Auto-Reconnect & Offline Buffer**: If Figma is opened before the Rust service starts, the plugin displays a waiting screen and connects automatically the instant the service is up.
5. **Port Range & Auto-Switch**: The bridge uses ports `41730–41739`. On start it reuses a running server of the same version, skips any port held by another app or a different figma-rust-mcp version, and starts on the first free port. The plugin scans the same range, so it follows automatically. MCP clients configured by URL point at `41730`; the server prints a warning with the new port if it had to move.

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
url = "http://127.0.0.1:41730/mcp"
```

For Codex CLI, the equivalent command is:

```bash
codex mcp add figma-rust-mcp --url http://127.0.0.1:41730/mcp
```

#### Google Antigravity (SSE Transport)
Add to your `~/.gemini/config/mcp_config.json` or project `.agents/mcp_config.json`:
```json
{
  "mcpServers": {
    "figma-rust-mcp": {
      "serverUrl": "http://127.0.0.1:41730/sse"
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
      "url": "http://127.0.0.1:41730/mcp"
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
curl http://127.0.0.1:41730/plugin/version
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
Reads a specific node by ID or name through the canonical flat `read_nodes` response. Select fields explicitly to control the work performed.

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
In-memory queries against pre-indexed Figma file structures (latency for your file: plugin **Bench** tab).
- **`operation`**:
  - `"status"`: View index health and node counts.
  - `"search_nodes"`: Search nodes by text name, query, and type (`FRAME`, `TEXT`, `COMPONENT`, `INSTANCE`).
  - `"get_node"`: Instant node lookup by ID.
  - `"search_components"`: Find component sets and variants.
  - `"search_styles"`: Find paint, text, and effect styles.
  - `"search_variables"`: Find design token variables by name or collection.
  - `"refresh"`: Refresh page roots, or expand the frame specified by `nodeId`.
  - `"subtree"`: Read/cache one `nodeId`, default depth 2 and 1000 nodes. Increase `depth` or `maxNodes` only for the section you need; check `meta` for truncation.
  - `"typography"`: Rust-generated text/font table for one `nodeId`, with mixed-style runs preserved. Default limit 200 rows. Null font fields mean unknown/mixed, not a guessed default.

`status` includes per-tab dispatch cache hits/misses, bridge calls and total bridge wait time, plus process-wide tool call counts and total/average milliseconds. Node changes invalidate related subtrees; style changes, unknown events and global writes invalidate the whole cache. Detailed subtree data is loaded on demand. The initial index remains shallow.

For typography verification, call `figma_verify_ui` with a text `nodeId` and browser `computedStyles` (`font-size` in px, numeric `font-weight`, `font-family`). Mixed text accepts `computedStyles.segments`, an ordered array of CSS objects matching the Figma runs. Size checks preserve fractional pixels.

### 9. `figma_read`
Universal reader for advanced queries:
- **Design-to-code**: `read_nodes`, `get_css`, `get_component_map`, `get_unmapped_components`.
- **Inspection & Hierarchy**: `read_nodes`, `get_selection`, `get_page_nodes`, `scan_design`, `search_nodes`.
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
Audits the current Figma document and generates a complete design system rule sheet (color tokens, typography styles, variables, component catalog) from the in-memory cache.

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

### Startup indexing and v4 migration

Version 4 requires both server and plugin runtime protocol 3. Update both and
restart the plugin. Opening the bridge indexes only visible top-level nodes on
the active page, plus local styles and variables. Selection expands the selected
frame. Component catalogue discovery is deferred; request it with
`figma_index({ operation: "refresh", includeComponents: true })`.

Search covers indexed scopes only. Inspect `scope` and `complete` in search
responses; an empty result does not prove a layer is absent from the file.
Refresh a specific frame to expand its scope:

```js
figma_index({ operation: "refresh", nodeId: "123:456", expandInstances: true })
figma_read({ operation: "read_nodes", nodeId: "123:456",
  fields: ["geometry", "content", "text", "style", "layout", "tokens", "component"],
  limit: 200, expandInstances: true })
// Continue using the returned cursor; do not repeat or change traversal options.
figma_read({ operation: "read_nodes", cursor: nextCursor, limit: 200 })
```

`read_nodes` returns a flat envelope:
`{ schemaVersion: 4, pageId, revision, scope, nodes, nextCursor, complete, totalRead, budgetReached }`.
Identity and topology are always included. Default fields are geometry and
content; typography, paints, layout, variable/style references and component
metadata are opt-in. Token fields contain references, not resolved CSS values.
Page reads default to depth 0; frame reads default to full traversal (maximum
256 levels). Limit is 1?500 nodes per response; traversal stops at 50,000 visited
nodes. `complete` describes the requested scope, including its depth and filters.
Restart from the root if the page or document revision changes between cursor
requests. Cursors are single-use and at most 16 traversals can remain open.

Instances are opaque by default: `opaque: true` and no descendant traversal.
Set `expandInstances: true` to read the actual instance children and overrides.
Hidden nodes require `includeHidden: true`. Expansion and visibility are separate
options; no master component is substituted for an instance.

Breaking output changes: public `get_design`, `get_design_context` and
`get_node_detail` aliases now return the same envelope as `read_nodes`.
`figma_inspect_node` and index `get_node`, `subtree`, `typography` also return
that envelope. Replace nested `children` consumers with `nodes` and parent/child
IDs. Replace index `maxNodes` with `limit` and cursor pagination. Internal
script helpers used for code generation retain their specialized formats.

Canvas traversal reads each child list once, yields after an 8ms slice, and
streams chunks of up to 500 nodes. Snapshots commit atomically; frame refreshes
preserve other indexed frames. Revisioned create/delete/property patches
invalidate affected cached reads, while unrelated nodes remain queryable.
Structural additions mark coverage incomplete until the scope is reread.
Revision gaps request resynchronization. Page switches rebuild the shallow
index without preloading every page. Exact-read caching evicts one least
recently used entry when its 64-entry capacity is reached.

## 📏 Benchmarks

`npm run bench` builds the release binary and prints a JSON report from a fresh
server: cold start, process startup, server RAM and MCP dispatch latency. A fresh
server has no Figma session, so plugin and index measurements are explicitly
skipped.

To measure real work, connect the Figma plugin and use the existing server:

```bash
# Server report, including its current index and plugin round trip
node scripts/bench.mjs --url http://127.0.0.1:41730

# Read all descendants of a frame, index that frame, and export it as a 1x PNG
node scripts/bench.mjs --url http://127.0.0.1:41730 \
  --target 'SESSION_ID=FRAME_ID' --samples 3 > benchmark.json

# Run the same workloads concurrently on two tabs
node scripts/bench.mjs --url http://127.0.0.1:41730 \
  --target 'FIRST_SESSION=FIRST_FRAME' --target 'SECOND_SESSION=SECOND_FRAME'
```

Find session IDs at `GET /sessions` and frame IDs in the plugin selection card.
Use one frame per session and 1–10 samples per workload. Workloads read Figma
content; they do not modify canvas nodes. Indexing updates the bridge's cache,
and PNG export may briefly move the viewport before restoring it.

| Measurement | What it includes |
|---|---|
| MCP `tools/list` | 200 in-process dispatches, p50/p95 |
| Index search | 200 searches over the current index, when available |
| Plugin round trip | 10 `get_viewport` calls, when connected |
| Frame read | All paginated `read_nodes` responses, node/page counts and JSON bytes |
| Frame index | Plugin scan and response time; bridge commit acknowledgement is separate |
| PNG export | Figma export, transfer and decoded image bytes |
| Concurrent tabs | Workloads run sequentially per tab and concurrently across tabs |
| RAM | Server RSS before/after workloads, not peak RAM or Figma's memory |

Reports contain each sample plus p50/p95/max. Reads use the normal cache, so the
samples are not guaranteed cold reads. Truncated reads remain marked incomplete;
failed workloads retain their error and the CLI exits with status 1. Real Figma
results depend on your file; simulated integration tests only verify the path.

The plugin's existing **Bench** tab retains the basic `POST /benchmark` report.
Use the CLI for the heavier frame and concurrent-tab workloads.

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
