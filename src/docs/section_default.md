## Quick start (every session)

```js
// 1. Design tokens — idempotent; these are the default palette and scale
var tokens = await figma.setupDesignTokens({
  collectionName: "Design Tokens",
  colors: {
    "accent": "#3B82F6", "accent-dim": "#1D4ED8",
    "bg-base": "#08090E", "bg-surface": "#0F1117", "bg-card": "#111318",
    "bg-elevated": "#0D0F14", "border": "#1E2030",
    "text-primary": "#F0F2F5", "text-secondary": "#8B8FA3", "text-muted": "#555872",
    "positive": "#00DC82", "negative": "#FF4757", "warning": "#FFB547",
  },
  numbers: { "radius-sm": 8, "radius-md": 12, "radius-lg": 16, "spacing-xs": 4, "spacing-sm": 8, "spacing-md": 16, "spacing-lg": 24 }
});

// 2. Variable lookup
var vars = await figma.get_variables();
var varMap = {};
vars.collections.forEach(c => c.variables.forEach(v => { varMap[v.name] = v.id; }));

// 3. "🎨 Design Library" reference frame (off-canvas at x: -2000)
await figma.ensure_library();
```

Text style scale: heading-2xl 32 Bold · heading-xl 24 Bold · heading-lg 20 Bold · heading-md 16 SemiBold · body-md 14 · body-sm 12 · caption 11 · label 11 Medium. Create them with `setupDesignTokens({ textStyles })`, apply with `applyTextStyle` (section `tokens`).

## Rules

1. **Tokens, not hex.** Create with a hex, then bind: `await figma.applyVariable({ nodeId: node.id, field: "fill", variableId: varMap["accent"] })`. Same for font size/family/weight via text styles.
2. **Components for anything repeated.** Check `await figma.listComponents()`; if missing, build once, `figma.createComponent({ nodeId, name: "btn/primary" })`, then `figma.instantiate({ componentName, parentId, x, y })`.
3. **Layer order:** the last child drawn renders on top. Background first, then overlays, then content.
4. **Text must contrast its container:** never the same color for a fill and the text inside it.
5. **Icons:** `figma.loadIcon(name, { size, fill })`, never emoji. Icon size = container × 0.5.
6. **Auto-layout:** icon+text rows use `counterAxisAlignItems: "CENTER"`. `"STRETCH"` is invalid there — use `"MIN"` on the parent and `layoutAlign: "STRETCH"` on each child. Do not combine `layoutGrow: 1` with `primaryAxisAlignItems: "CENTER"`; use `"SPACE_BETWEEN"`. No overlapping rectangles inside auto-layout (progress bars need a non-layout wrapper).
7. **Hide with `fillOpacity: 0`**, not `opacity: 0` on a wrapper (that hides all children).
8. **Centered text:** pass both `width` and `textAlign: "CENTER"`. For fontSize ≥ 48 set `lineHeight` ≈ fontSize. Prefer `primaryAxisSizingMode: "AUTO"` so containers fit content.
9. **SVG paths:** no `H`/`V` commands — use `L x y`.
10. **Naming:** frames PascalCase ("Signal Card"), components `type/name` ("btn/primary-lg"), colors `color/name`.
11. **QA:** after drawing, `figma_read { operation: "screenshot", nodeId, scale: 0.4 }`; fix overlaps, clipping and misalignment, repeat.

When the user says "this frame" or "the selected one", read it first: `figma_read { operation: "get_selection" }`.

## Sandbox

- Each `figma_write` call is a fresh sandbox: redeclare constants and IDs every call.
- Modern JS works (`const`, arrow functions, `?.`, `??`, spread, template literals). No `require`, `fetch`, `setTimeout`, `process` or `fs`.
- Every `figma.*` call returns a Promise — `await` it. `return` a value to get it back as JSON.
- Reading the canvas inside the sandbox is limited to `figma.query`, `figma.get_*` helpers (section `api`); use the `figma_read` tool for node trees.

## Reading

- Hidden layers ("hidden", "invisible", "ẩn"): add `includeHidden: true`, e.g. `figma_read({ operation: "read_nodes", nodeId: "1:2", includeHidden: true })`.
- Several tabs open: `figma_status` lists `sessions` (id + fileName). Confirm the file, then pass that `sessionId` on every call.
