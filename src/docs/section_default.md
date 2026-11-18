## Quick start (every session)

```js
// 1. Use the file's own design system first — never invent a palette for a file that has one.
const vars = await figma.getVariables();        // collections → variables (id, name, values)
const styles = await figma.getStyles();          // paint/text styles
const comps = await figma.listComponents();      // reusable components
const varMap = {};
vars.collections.forEach(c => c.variables.forEach(v => { varMap[v.name] = v.id; }));

// 2. Draw a screen in one call. `key` names a node; the result maps keys to ids.
const { root, ids } = await figma.render({
  type: "FRAME", key: "card", name: "Card", width: 360, layoutMode: "VERTICAL",
  padding: 16, itemSpacing: 8, fill: "#FFFFFF", cornerRadius: 12,
  children: [
    { type: "TEXT", key: "title", content: "Carrier locations", fontSize: 20, fontWeight: "SemiBold" },
    { type: "FRAME", key: "row", layoutMode: "HORIZONTAL", itemSpacing: 8, layoutSizingHorizontal: "FILL",
      counterAxisAlignItems: "CENTER", children: [
        { type: "ICON", name: "map-pin", size: 20, fill: "#000000" },
        { type: "TEXT", content: "1298 Birchwood Dr", layoutSizingHorizontal: "FILL" },
      ] },
  ],
}, { parentId: "1:2" });

// 3. Later calls find nodes by key instead of re-reading the tree.
const title = await figma.byKey("title");
```

Only when the file has no variables: create a palette with `setupDesignTokens` (section `tokens`) and a reference frame with `ensureLibrary()`. Ask before adding either to someone else's file.

## Rules

1. **The file's tokens, not hex.** Bind existing variables: `await figma.applyVariable({ nodeId, field: "fill", variableId: varMap["Background/Brand/White"] })`. Use existing text styles with `applyTextStyle`.
2. **Components for anything repeated.** Check `listComponents()` first; reuse with `figma.instantiate({ componentId, parentId, overrides })`. `detach({ id })` turns an instance into a frame when its layers must change.
3. **Sizing is `layoutSizingHorizontal` / `layoutSizingVertical` only:** `"FIXED"` keeps width/height, `"HUG"` fits content (auto-layout frames, text), `"FILL"` stretches across an auto-layout parent. An auto-layout frame without width/height hugs that axis.
4. **Layer order:** the last child drawn renders on top. Overlays (dropdowns, badges) go last with `layoutPositioning: "ABSOLUTE"` and x/y.
5. **Text must contrast its container:** never the same color for a fill and the text inside it.
6. **Icons:** `{ type: "ICON", name, size, fill }` in `render`, or `figma.loadIcon(name, { size, fill, parentId })`; never emoji. Pin a library with `"lucide:map-pin"`.
7. **Auto-layout:** icon+text rows use `counterAxisAlignItems: "CENTER"`. `"STRETCH"` is invalid there — use `layoutSizing…: "FILL"` on the child. Spread items with `primaryAxisAlignItems: "SPACE_BETWEEN"`.
8. **Fonts:** `fontFamily` + `fontWeight` (`"SemiBold"` finds "Semi Bold" or "Demi Bold" in that family). A font that is not installed throws; check with `figma.listAvailableFonts({ query })`.
9. **SVG paths:** no `H`/`V` commands — use `L x y`.
10. **Naming:** frames PascalCase ("Signal Card"), components `type/name` ("btn/primary-lg"), colors `color/name`.
11. **QA:** after drawing, `figma_read { operation: "screenshot", nodeId, scale: 0.5 }`; fix overlaps, clipping and misalignment, repeat.

When the user says "this frame" or "the selected one", read it first: `figma_read { operation: "get_selection" }`.

## Sandbox

- Each `figma_write` call is a fresh sandbox. Find earlier nodes with `figma.byKey(key)`; ids also work.
- Every call is checked strictly: an unknown or misplaced key (a text key on a frame, `layoutAlign`) throws before anything changes.
- A call that throws removes the nodes it created; edits to existing nodes stay. `figma.batch([...])` is all-or-nothing the same way.
- Each `figma_write` is one undo step in Figma.
- Modern JS works (`const`, arrow functions, `?.`, `??`, spread, template literals). No `require`, `fetch`, `setTimeout`, `process` or `fs`.
- Every `figma.*` call returns a Promise — `await` it. `return` a value to get it back as JSON.
- Writes return one node shape: `{ id, name, type, parentId, childIds, x, y, width, height, key?, layoutPositioning?, warnings? }`.
- The sandbox is not the Figma Plugin API: `figma.createFrame()`, `figma.currentPage` and old snake_case names throw with the call to use instead.

## Reading

- Hidden layers ("hidden", "invisible", "ẩn"): add `includeHidden: true`, e.g. `figma_read({ operation: "read_nodes", nodeId: "1:2", includeHidden: true })`.
- Several tabs open: `figma_status` lists `sessions` (id + fileName). Confirm the file, then pass that `sessionId` on every call.
