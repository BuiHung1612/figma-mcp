
# figma-rust-mcp — Layout Rules

---

## AUTO LAYOUT (PREFERRED — NON-NEGOTIABLE for complex containers)

```js
await figma.create({
  type: "FRAME", name: "Button", parentId: root.id,
  width: 392, height: 52,
  fill: "#6C5CE7", cornerRadius: 12,
  layoutMode: "HORIZONTAL",           // "HORIZONTAL" | "VERTICAL"
  primaryAxisAlignItems: "CENTER",    // main axis: "MIN"|"CENTER"|"MAX"|"SPACE_BETWEEN"
  counterAxisAlignItems: "CENTER",    // cross axis: "MIN"|"CENTER"|"MAX"
  padding: 16,
  itemSpacing: 8,
})
```

## SIZING — `layoutSizingHorizontal` / `layoutSizingVertical`

One control per axis, on any node:

| Value | Means | Works on |
|---|---|---|
| `"FIXED"` | keep `width` / `height` | everything |
| `"HUG"` | shrink to content | auto-layout frames, text |
| `"FILL"` | stretch across the parent's axis | children of an auto-layout frame |

- An auto-layout frame without `width` hugs horizontally; without `height` it hugs vertically. Give a size (or `"FIXED"`) to stop that.
- Full-width child of a column: `layoutSizingHorizontal: "FILL"`. Child that takes the leftover space in a row: `layoutSizingHorizontal: "FILL"`; in a column: `layoutSizingVertical: "FILL"`.
- Text that wraps: `layoutSizingHorizontal: "FILL"` (or a fixed `width`). Text that sizes to its words: no width.
- `layoutAlign`, `layoutGrow`, `primaryAxisSizingMode` and `counterAxisSizingMode` were removed and throw with this table's replacement.

**Common patterns:**
```
// Button with centered text:
layoutMode: "HORIZONTAL", primaryAxisAlignItems: "CENTER", counterAxisAlignItems: "CENTER"

// Card with icon + text row:
layoutMode: "HORIZONTAL", counterAxisAlignItems: "CENTER", paddingLeft: 16, itemSpacing: 12

// Vertical stack whose children span the width:
layoutMode: "VERTICAL", itemSpacing: 8
// children: layoutSizingHorizontal: "FILL"
```

**Always use Auto Layout for:** buttons, cards with icon+text rows, tab bar items, list items, badge pills.

---

## BUILD WHOLE SCREENS WITH `render`

One call per screen or section instead of one per node. `key` names a node; the result maps keys to ids and `figma.byKey(key)` finds it in later calls.

```js
const { ids } = await figma.render({
  type: "FRAME", key: "list-item", layoutMode: "HORIZONTAL", width: 370, padding: 12, itemSpacing: 10,
  counterAxisAlignItems: "CENTER", fill: "#FFFFFF", cornerRadius: 8,
  children: [
    { type: "ICON", name: "map-pin", size: 20, fill: "#000000" },
    { type: "FRAME", layoutMode: "VERTICAL", itemSpacing: 2, layoutSizingHorizontal: "FILL", children: [
      { type: "TEXT", content: "The UPS Store #4521", fontWeight: "Medium", layoutSizingHorizontal: "FILL" },
      { type: "TEXT", content: "1310 Birchwood Dr", fontSize: 12, fill: "#666666", layoutSizingHorizontal: "FILL" },
    ] },
    { type: "TEXT", key: "distance", content: "0.3 mi", fontWeight: "Medium" },
  ],
}, { parentId: screenId });
```

If any node fails, nothing from the call stays on the canvas.

---

## BUTTON / INPUT CONSTRUCTION RULE

NEVER use RECTANGLE + TEXT as siblings. Always use FRAME with layoutMode:

```js
// WRONG
await figma.create({ type: "RECTANGLE", parentId: frame.id, x: 56, y: 808, width: 488, height: 58, fill: "#00C896", cornerRadius: 30 });
await figma.create({ type: "TEXT", parentId: frame.id, x: 180, y: 827, content: "Submit" }); // not truly centered

// CORRECT
await figma.render({
  type: "FRAME", width: 488, height: 58, fill: "#00C896", cornerRadius: 30,
  layoutMode: "HORIZONTAL", primaryAxisAlignItems: "CENTER", counterAxisAlignItems: "CENTER",
  children: [{ type: "TEXT", content: "Submit", fill: "#FFFFFF", fontSize: 16, fontWeight: "Bold" }],
}, { parentId: frame.id });
```

Applies to: buttons, inputs, tabs, chips, badges, nav items — ALL elements with background + content.

---

## CARD / SCREEN LAYOUT RULE

NEVER use absolute x/y for children inside a card. Use VERTICAL auto-layout:

```js
await figma.render({
  type: "FRAME", name: "Card", width: 480,
  layoutMode: "VERTICAL", padding: 48, itemSpacing: 16,   // no height → hugs content
  children: [
    // Full-width children: FILL, no width needed
    { type: "FRAME", name: "Input", height: 52, layoutSizingHorizontal: "FILL",
      layoutMode: "HORIZONTAL", counterAxisAlignItems: "CENTER", paddingLeft: 20, paddingRight: 20 },
    // "or" divider row: lines take the leftover space
    { type: "FRAME", height: 20, layoutSizingHorizontal: "FILL", layoutMode: "HORIZONTAL",
      counterAxisAlignItems: "CENTER", itemSpacing: 8, children: [
        { type: "RECTANGLE", height: 1, layoutSizingHorizontal: "FILL", fill: "#E0E0E0" },
        { type: "TEXT", content: "or", fontSize: 12, fill: "#888888" },
        { type: "RECTANGLE", height: 1, layoutSizingHorizontal: "FILL", fill: "#E0E0E0" },
    ] },
  ],
});
```

---

## OVERLAYS — DROPDOWNS, BADGES, TOOLTIPS

An overlay must not push its siblings. Draw it last in the auto-layout frame with `layoutPositioning: "ABSOLUTE"` and x/y relative to that frame:

```js
await figma.create({ type: "FRAME", key: "suggestions", parentId: formId, layoutPositioning: "ABSOLUTE",
  x: 0, y: 68, width: 370, layoutMode: "VERTICAL", fill: "#FFFFFF", stroke: "#A7A7A7", cornerRadius: 4,
  effects: [{ type: "DROP_SHADOW", color: "#0000001F", offset: { x: 0, y: 6 }, radius: 16 }] });
```

Move an existing node with `modify({ id, parentId, insertIndex })`.

---

## DOT + TEXT / ICON + TEXT ROW ALIGNMENT RULE

ALWAYS use `counterAxisAlignItems: "CENTER"` for icon/dot next to text:
```
CORRECT: layoutMode: "HORIZONTAL", counterAxisAlignItems: "CENTER", itemSpacing: 12
WRONG:   counterAxisAlignItems: "MIN" → dot sits at top, misaligned
```

**Multi-line exception:** if dot/icon aligns with FIRST line only:
```
counterAxisAlignItems: "MIN"
icon paddingTop = (textLineHeight - iconSize) / 2
// e.g. text 22px, dot 8px → paddingTop = (22 - 8) / 2 = 7
```

---

## PROGRESS BAR RULE (CRITICAL)

Progress bars = TWO overlapping rectangles. Auto-layout stacks them side-by-side, NOT overlapping.
**Wrap them in a frame without layoutMode:**

```js
await figma.render({
  type: "FRAME", name: "progress-bar", width: 352, height: 6,   // NO layoutMode → children overlap via x,y
  children: [
    { type: "RECTANGLE", x: 0, y: 0, width: 352, height: 6, fill: "#E7EAF0", cornerRadius: 3 },
    { type: "RECTANGLE", x: 0, y: 0, width: 211, height: 6, fill: "#6C5CE7", cornerRadius: 3 },
  ],
}, { parentId: autoLayoutParent.id });
```

Applies to: progress bars, score rings, slider tracks.

---

## BADGE / PILL / TAG RULE

**Text inside badge: use auto-layout CENTER/CENTER**
```js
await figma.render({
  type: "FRAME", name: "badge", height: 20, fill: "#E8FBF5", cornerRadius: 10, paddingLeft: 8, paddingRight: 8,
  layoutMode: "HORIZONTAL", primaryAxisAlignItems: "CENTER", counterAxisAlignItems: "CENTER",
  children: [{ type: "TEXT", content: "Free", fontSize: 10, fontWeight: "SemiBold", fill: "#00B894" }],
}, { parentId: parent.id });
```

**Badge on a card corner:** make it a child of the card with `layoutPositioning: "ABSOLUTE"`, `x: cardWidth - badgeWidth - 6`, `y: 6`.

---

## MOBILE BOTTOM ANCHORING RULE

Bottom elements (tab bar, FAB) MUST be calculated from frame bottom:
```
nav_bar_y = frameHeight - safeArea - navHeight  // e.g. 844 - 34 - 64 = 746
cta_y     = nav_bar_y - gap - ctaHeight         // e.g. 746 - 12 - 56 = 678
```

Standard iOS: safeArea = 34px, home indicator at y = frameH - 18.
NEVER hardcode y for bottom elements without calculating from frameHeight.

---

## CENTERED CONTENT MUST USE AUTO-LAYOUT

NEVER use manual `x = (containerW - childW) / 2` — it breaks when content changes.

```js
// CORRECT
await figma.render({ type: "FRAME", width: 108, height: 108, fill: "#0D1229", cornerRadius: 18,
  layoutMode: "VERTICAL", primaryAxisAlignItems: "CENTER", counterAxisAlignItems: "CENTER",
  paddingTop: 16, paddingBottom: 14, itemSpacing: 8, children: [/* no x/y */] });
```

---

## ILLUSTRATION CENTERING + LAYER ORDER RULE

**Draw order: background → rings → center icon (last = on top)**, inside a frame without layoutMode:
```
element_x = centerX - (element_width / 2)
element_y = centerY - (element_height / 2)
```

---

## TEXT ALIGN vs SIZING RULE

`layoutSizingHorizontal: "FILL"` controls the box. `textAlign: "CENTER"` controls the content. Both must be set:

```js
// CORRECT — box fills width AND content is centered
{ type: "TEXT", content: "Centered heading", fontSize: 18, fill: "#FFFFFF",
  textAlign: "CENTER", layoutSizingHorizontal: "FILL", lineHeight: 26 }

// WRONG — box hugs the words, so there is nothing to center within
{ type: "TEXT", content: "Should center but won't", textAlign: "CENTER" }
```

---

## TEXT WRAPPING IN AUTO-LAYOUT RULE

Text without a width grows on one line and overflows. Give text that should wrap a width:

```js
// CORRECT
{ type: "TEXT", content: "Long text...", fontSize: 13, fill: "#E0E6F0", lineHeight: 18, layoutSizingHorizontal: "FILL" }

// WRONG — renders at natural width, overflows the parent
{ type: "TEXT", content: "Long text...", fontSize: 13 }
```

---

## HEADER TITLE CENTERING RULE

Pattern [Left action] [Title] [Right action] — the title fills the middle and centers its text:

```js
await figma.render({ type: "FRAME", layoutMode: "HORIZONTAL", layoutSizingHorizontal: "FILL", height: 48,
  counterAxisAlignItems: "CENTER", children: [
    { type: "ICON", name: "chevron-back", size: 24 },
    { type: "TEXT", content: "Title", fontSize: 17, fontWeight: "Bold", textAlign: "CENTER", layoutSizingHorizontal: "FILL" },
    { type: "FRAME", width: 24, height: 24 },   // balances the left action
  ] }, { parentId: screenId });
```

Applies to: modal headers, nav bars, any [action][title][action] pattern.
