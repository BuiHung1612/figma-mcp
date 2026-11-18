// ─── WRITE HANDLERS ───────────────────────────────────────────────────────────

const handlers = {};

handlers.status = async () => ({
  connected:   true,
  version:     "{{PLUGIN_VERSION}}",
  fileName:    figma.root.name,
  currentPage: figma.currentPage.name,
  pageCount:   figma.root.children.length,
  selection:   figma.currentPage.selection.map(nodeToInfo),
});

handlers.listPages = async () =>
  figma.root.children.map(p => ({ id: p.id, name: p.name }));

handlers.setPage = async (params) => {
  if (typeof figma.loadAllPagesAsync === "function") {
    try { await figma.loadAllPagesAsync(); } catch (e) {}
  }
  params = params || {};
  var name = typeof params === "string" ? params : (params.name || params.pageName || params.title || (typeof params.page === "string" ? params.page : (params.page && typeof params.page.name === "string" ? params.page.name : null)));
  var id = (typeof params === "object" && params !== null) ? (params.id || params.pageId || (params.page && typeof params.page.id === "string" ? params.page.id : null)) : null;
  var index = (typeof params === "number") ? params : ((typeof params === "object" && params !== null) ? (params.index !== undefined ? params.index : params.pageIndex) : undefined);

  var page = null;
  if (id) {
    page = figma.root.children.find(function(p) { return p.id === id; });
  }
  if (!page && name) {
    page = figma.root.children.find(function(p) { return p.name === name; });
    if (!page) {
      // Case-insensitive & trimmed match fallback
      var lowerName = String(name).trim().toLowerCase();
      page = figma.root.children.find(function(p) { return p.name.trim().toLowerCase() === lowerName; });
    }
  }
  if (!page && typeof index === "number" && index >= 0 && index < figma.root.children.length) {
    page = figma.root.children[index];
  }
  if (!page && !name && !id && index === undefined && figma.root.children.length === 1) {
    page = figma.root.children[0];
  }
  if (!page) {
    var requested = name || id || (index !== undefined ? ("index " + index) : null);
    var available = figma.root.children.map(function(p) { return p.name; }).join(", ");
    if (requested) {
      throw new Error("Page not found: \"" + requested + "\". Available: " + available);
    } else {
      throw new Error("setPage requires a valid page name, id, or index. Available: " + available);
    }
  }
  await figma.setCurrentPageAsync(page);
  return { id: page.id, name: page.name };
};

handlers.createPage = async (params) => {
  if (typeof figma.loadAllPagesAsync === "function") {
    try { await figma.loadAllPagesAsync(); } catch (e) {}
  }
  var name = typeof params === "string" ? params : (params && (params.name || params.pageName || params.title));
  if (!name || typeof name !== "string") {
    throw new Error("createPage requires a valid page name string");
  }
  const existing = figma.root.children.find(p => p.name === name || p.name.trim().toLowerCase() === name.trim().toLowerCase());
  if (existing) return { id: existing.id, name: existing.name, existed: true };
  const page = figma.createPage();
  page.name = name;
  return { id: page.id, name: page.name };
};

handlers.query = async (params) => {
  params = params || {};
  var type = params.type;
  var name = params.name || params.nodeName || params.query || params.q;
  var id = params.id || params.nodeId;
  var parentId = params.parentId, limit = params.limit || 500;
  if (id) {
    const n = await findNodeByIdAsync(id);
    return n ? [nodeToInfo(n)] : [];
  }
  // BUG-03: parentId filter scopes search to a subtree; default cap raised 100 → 500.
  var scope = figma.currentPage;
  if (parentId) {
    var parent = await findNodeByIdAsync(parentId);
    if (!parent) throw new Error("parentId not found: " + parentId);
    if (typeof parent.findAll !== "function") return [];
    scope = parent;
  }
  if (!type && !name) {
    // deep: every descendant (e.g. ids inside a frame); default stays direct children.
    var allChildren = params.deep ? scope.findAll(function() { return true; }) : (scope.children || []);
    return allChildren.slice(0, limit).map(nodeToInfo);
  }
  const results = scope.findAll(n => {
    if (type && name) return n.type === type && (n.name === name || (typeof name === "string" && n.name.toLowerCase().includes(name.toLowerCase())));
    if (type) return n.type === type;
    return n.name === name || (typeof name === "string" && n.name.toLowerCase().includes(name.toLowerCase()));
  });
  return results.slice(0, limit).map(nodeToInfo);
};

// ─── shared helpers ────────────────────────────────────────────────────────────

// Apply resize + fill + stroke + effects — shared by FRAME, RECT, ELLIPSE builders.
function applyCommonProps(node, params, width, height, fill, stroke, strokeWeight) {
  node.resize(width, height);
  node.fills = fill ? buildFillArray(fill, params.fillOpacity) : [];
  if (stroke) {
    node.strokes = solidStroke(stroke, params.strokeOpacity);
    node.strokeWeight = strokeWeight;
    if (params.strokeAlign) node.strokeAlign = params.strokeAlign;
    // BUG-17: forward strokeDashPattern to Figma dashPattern
    if (params.strokeDashPattern && Array.isArray(params.strokeDashPattern)) node.dashPattern = params.strokeDashPattern;
  }
  if (params.effects) applyEffects(node, params.effects);
}

// Apply auto-layout props to a FRAME node (create or modify path).
function applyAutoLayout(node, params) {
  if (!params.layoutMode || params.layoutMode === "NONE") return;
  node.layoutMode = params.layoutMode;
  if (params.counterAxisAlignItems === "STRETCH") {
    throw new Error(
      "counterAxisAlignItems does not support \"STRETCH\". " +
      "Set counterAxisAlignItems: \"MIN\" on the container and layoutSizingHorizontal/Vertical: \"FILL\" on each child."
    );
  }
  if (params.primaryAxisAlignItems) node.primaryAxisAlignItems = params.primaryAxisAlignItems;
  if (params.counterAxisAlignItems) node.counterAxisAlignItems = params.counterAxisAlignItems;

  if (params.padding !== undefined) {
    node.paddingTop = node.paddingBottom = node.paddingLeft = node.paddingRight = params.padding;
  }
  if (params.paddingHorizontal !== undefined) { node.paddingLeft = node.paddingRight = params.paddingHorizontal; }
  if (params.paddingVertical   !== undefined) { node.paddingTop  = node.paddingBottom = params.paddingVertical; }
  if (params.paddingTop    !== undefined) node.paddingTop    = params.paddingTop;
  if (params.paddingBottom !== undefined) node.paddingBottom = params.paddingBottom;
  if (params.paddingLeft   !== undefined) node.paddingLeft   = params.paddingLeft;
  if (params.paddingRight  !== undefined) node.paddingRight  = params.paddingRight;
  if (params.itemSpacing   !== undefined) node.itemSpacing   = params.itemSpacing;

  // FIXED base; hugging is requested with layoutSizingHorizontal/Vertical: "HUG".
  node.primaryAxisSizingMode = "FIXED";
  node.counterAxisSizingMode = "FIXED";
  if (params.clipsContent !== undefined) node.clipsContent = params.clipsContent;
}

// ABSOLUTE lets a child of an auto-layout frame float (overlays, badges,
// dropdowns). Figma rejects it outside auto-layout, so say why instead of
// failing with a bare setter error.
function applyLayoutPositioning(node, params) {
  if (params.layoutPositioning === undefined) return;
  var parent = node.parent;
  if (!parent || !parent.layoutMode || parent.layoutMode === "NONE") {
    if (params.layoutPositioning === "AUTO") return;
    throw new Error("layoutPositioning needs an auto-layout parent; \"" + (parent ? parent.name : "none") +
      "\" has no auto-layout, so set x/y directly instead");
  }
  node.layoutPositioning = params.layoutPositioning;
}

// Keys each op accepts; anything else throws before the op changes the file.
var COMMON_KEYS = [
  "name", "key", "fill", "fills", "fillOpacity", "fontColor",
  "stroke", "strokeColor", "strokeOpacity", "strokeWeight", "strokeAlign", "strokeDashPattern",
  "x", "y", "width", "height", "opacity", "visible", "effects",
  "cornerRadius", "topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius",
  "content", "fontFamily", "fontWeight", "fontSize", "lineHeight",
  "textAlign", "textAlignHorizontal", "textAlignVertical", "textAutoResize", "textDecoration", "textCase", "textTransform",
  "layoutMode", "primaryAxisAlignItems", "counterAxisAlignItems",
  "padding", "paddingHorizontal", "paddingVertical", "paddingTop", "paddingBottom", "paddingLeft", "paddingRight",
  "itemSpacing", "clipsContent", "layoutSizingHorizontal", "layoutSizingVertical", "layoutPositioning",
  "parentId", "insertIndex",
];
var MODIFY_KEYS = new Set(COMMON_KEYS.concat(["id", "nodeId", "node_id", "targetId", "target_id", "nodeName", "node_name"]));
var CREATE_KEYS = new Set(COMMON_KEYS.concat(["type", "characters", "strokeCap", "strokeJoin",
  "svg", "d", "path", "paths", "windingRule", "arcData", "imageData", "scaleMode"]));

// A known key on the wrong node type would otherwise do nothing and look applied.
var TEXT_ONLY_KEYS = ["content", "fontFamily", "fontWeight", "fontSize", "lineHeight", "textAlign", "textAlignHorizontal",
  "textAlignVertical", "textAutoResize", "textDecoration", "textCase", "textTransform"];
var AUTO_LAYOUT_KEYS = ["layoutMode", "primaryAxisAlignItems", "counterAxisAlignItems", "padding", "paddingHorizontal",
  "paddingVertical", "paddingTop", "paddingBottom", "paddingLeft", "paddingRight", "itemSpacing"];
function assertKeysFitNode(node, params) {
  var wrong = [];
  if (node.type !== "TEXT") wrong = wrong.concat(TEXT_ONLY_KEYS.filter(function(k) { return params[k] !== undefined; }));
  if (!("layoutMode" in node)) wrong = wrong.concat(AUTO_LAYOUT_KEYS.filter(function(k) { return params[k] !== undefined; }));
  if (wrong.length) throw new Error("modify: " + wrong.join(", ") + " do not apply to " + node.type + " \"" + node.name + "\"");
}

// fills: [] clears; entries may be hex strings, {color, opacity} or raw Figma paints.
function buildFillsParam(fills, fillOpacity) {
  if (!Array.isArray(fills)) throw new Error("fills must be an array");
  return fills.map(function(f) {
    if (typeof f === "string") return buildFillArray(f, fillOpacity)[0];
    if (f && typeof f.color === "string") return buildFillArray(f.color, f.opacity !== undefined ? f.opacity : fillOpacity)[0];
    return f;
  }).filter(Boolean);
}

// Move node under parentId (or within its parent) at insertIndex.
async function reparentNode(node, params) {
  var parent = params.parentId ? await findNodeByIdAsync(params.parentId) : node.parent;
  if (!parent || parent.removed) throw new Error("parentId not found: " + params.parentId);
  if (typeof parent.insertChild !== "function") throw new Error("Node " + parent.id + " cannot have children");
  for (var a = parent; a; a = a.parent) {
    if (a.id === node.id) throw new Error("Cannot move a node into itself or its descendant");
  }
  var count = parent.children.length - (node.parent === parent ? 1 : 0);
  var at = params.insertIndex === undefined ? count : Math.max(0, Math.min(params.insertIndex, count));
  parent.insertChild(at, node);
}

// layoutSizingHorizontal/Vertical is the one sizing control: FIXED keeps the
// width/height, HUG shrinks an auto-layout frame or text to its content, FILL
// stretches a child across its auto-layout parent. Errors are reported, not
// swallowed, so a size that did not apply never looks applied.
function applySizing(node, params) {
  ["layoutSizingHorizontal", "layoutSizingVertical"].forEach(function(axis) {
    if (params[axis] === undefined) return;
    var value = String(params[axis]).toUpperCase();
    var parent = node.parent;
    if (value === "FILL" && !(parent && parent.layoutMode && parent.layoutMode !== "NONE")) {
      throw new Error(axis + ": \"FILL\" needs an auto-layout parent; \"" + (parent ? parent.name : "none") + "\" has none");
    }
    if (value === "HUG" && node.type !== "TEXT" && !(node.layoutMode && node.layoutMode !== "NONE")) {
      throw new Error(axis + ": \"HUG\" needs an auto-layout frame or a text node");
    }
    node[axis] = value;
  });
}

// When TEXT content is changed on a node inside a hug-axis auto-layout parent,
// promote the text node's matching layoutSizing axis to HUG. Otherwise the
// parent's hug request is silently ignored because the child's
// layoutSizingHorizontal defaults to FIXED after parenting — and FIXED beats
// textAutoResize for auto-layout children. Without this, setComponentProperties
// (and direct modify with content) update the text but the button never grows.
//
// Best-effort: skipped when the user passed the axis explicitly, when the
// parent isn't hug-mode, or when Figma rejects the assignment.
function autoPromoteTextHugForReflow(node, params) {
  if (!node || node.type !== "TEXT") return;
  if (params.content === undefined) return;

  var parent = node.parent;
  if (!parent || !parent.layoutMode || parent.layoutMode === "NONE") return;

  var horizontal = parent.layoutMode === "HORIZONTAL";
  var parentHugsPrimary = parent.primaryAxisSizingMode === "AUTO" ||
    (horizontal ? parent.layoutSizingHorizontal === "HUG"
                : parent.layoutSizingVertical   === "HUG");
  if (!parentHugsPrimary) return;

  if (horizontal && params.layoutSizingHorizontal !== undefined) return;
  if (!horizontal && params.layoutSizingVertical !== undefined) return;

  try {
    if (horizontal && "layoutSizingHorizontal" in node) {
      node.layoutSizingHorizontal = "HUG";
    } else if (!horizontal && "layoutSizingVertical" in node) {
      node.layoutSizingVertical = "HUG";
    }
  } catch (e) {}
}

// Reusable base64 lookup table — built once, not per-call.
var _B64_LOOKUP = (function() {
  var chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  var t = {};
  for (var i = 0; i < chars.length; i++) t[chars[i]] = i;
  return t;
})();

// ─── node builders (one per type) ─────────────────────────────────────────────

function buildFrame(params, width, height, fill, stroke, strokeWeight) {
  var node = figma.createFrame();
  applyCommonProps(node, params, width, height, fill, stroke, strokeWeight);
  applyCornerRadii(node, params);
  applyAutoLayout(node, params);
  return node;
}

function buildRectangle(params, width, height, fill, stroke, strokeWeight) {
  var node = figma.createRectangle();
  applyCommonProps(node, params, width, height, fill, stroke, strokeWeight);
  applyCornerRadii(node, params);
  return node;
}

function buildEllipse(params, width, height, fill, stroke, strokeWeight) {
  var node = figma.createEllipse();
  applyCommonProps(node, params, width, height, fill, stroke, strokeWeight);
  if (params.arcData) {
    var arc = params.arcData;
    node.arcData = {
      startingAngle: arc.startingAngle !== undefined ? arc.startingAngle : (arc.startAngle !== undefined ? arc.startAngle : 0),
      endingAngle:   arc.endingAngle   !== undefined ? arc.endingAngle   : (arc.endAngle   !== undefined ? arc.endAngle   : Math.PI * 2),
      innerRadius:   arc.innerRadius   !== undefined ? arc.innerRadius   : 0,
    };
  }
  return node;
}

function buildLine(params, width, stroke, strokeWeight) {
  var node = figma.createLine();
  node.resize(width || 100, 0);
  node.fills = [];
  if (stroke) { node.strokes = solidStroke(stroke, params.strokeOpacity); node.strokeWeight = strokeWeight; }
  if (params.effects) applyEffects(node, params.effects);
  return node;
}

async function buildText(params, width, height, fill, fontSize, fontWeight, lineHeight) {
  var content = params.content !== undefined ? params.content : (params.characters !== undefined ? params.characters : "");
  var family = params.fontFamily || "Inter";
  var style = await resolveFontStyle(family, fontWeight);
  await ensureFontLoaded(family, style);
  var node = figma.createText();
  node.fontName = { family: family, style: style };
  node.fontSize = fontSize;
  node.characters = content;

  var textFill = fill || params.fontColor;
  if (textFill) {
    node.fills = solidFill(textFill, params.fillOpacity);
  } else if (params.fills && Array.isArray(params.fills) && params.fills.length > 0) {
    var firstColor = params.fills[0] && params.fills[0].color;
    if (firstColor) node.fills = buildFillArray(firstColor, params.fillOpacity);
  }

  if (params.effects) applyEffects(node, params.effects);

  if (lineHeight) {
    node.lineHeight = (typeof lineHeight === "object" && lineHeight.unit)
      ? lineHeight
      : { value: lineHeight, unit: "PIXELS" };
  }

  var textAlignValue = params.textAlignHorizontal || params.textAlign;
  if (textAlignValue) node.textAlignHorizontal = String(textAlignValue).toUpperCase();
  if (params.textAlignVertical) node.textAlignVertical = String(params.textAlignVertical).toUpperCase();

  // Auto-resize: explicit > both dims fixed > centered/right width > width-only > default hug
  if (params.textAutoResize) {
    node.textAutoResize = params.textAutoResize;
    if (params.textAutoResize === "NONE" && width && height) node.resize(width, height);
  } else if (width && height) {
    // BUG-01 fix: both dimensions given → fixed box, resize after setting NONE
    node.textAutoResize = "NONE";
    node.resize(width, height);
  } else if (width && textAlignValue) {
    var upAlign = String(textAlignValue).toUpperCase();
    if (upAlign === "CENTER" || upAlign === "RIGHT" || upAlign === "JUSTIFIED") {
      // Fixed width so centering has room; height stays auto (HEIGHT mode wraps)
      node.textAutoResize = "HEIGHT";
      node.resize(width, node.height);
    } else {
      node.textAutoResize = "HEIGHT";
    }
  } else if (width) {
    // BUG-TEXT-01: width-only → set width but let height auto-fit content.
    // Previously left node.height at default 100 → caused vertical overflow.
    node.textAutoResize = "HEIGHT";
    node.resize(width, node.height);
  } else {
    // BUG-TEXT-01: neither width nor height → hug both axes (natural text bounds).
    // Default text size in Figma is 100x100 if textAutoResize stays at its initial value;
    // WIDTH_AND_HEIGHT makes the box shrink-wrap the actual rendered glyphs.
    node.textAutoResize = "WIDTH_AND_HEIGHT";
  }

  if (params.textDecoration) node.textDecoration = String(params.textDecoration).toUpperCase();

  var tc = params.textCase || (params.textTransform ? (params.textTransform.toLowerCase() === "uppercase" ? "UPPER" : params.textTransform.toLowerCase() === "lowercase" ? "LOWER" : params.textTransform.toLowerCase() === "capitalize" ? "TITLE" : "ORIGINAL") : null);
  if (tc) {
    var upperTc = String(tc).toUpperCase();
    if (upperTc === "UPPERCASE") upperTc = "UPPER";
    if (upperTc === "LOWERCASE") upperTc = "LOWER";
    if (upperTc === "TITLECASE" || upperTc === "CAPITALIZE") upperTc = "TITLE";
    try { node.textCase = upperTc; } catch(e) {}
  }

  return node;
}

function buildSvg(params, width, height, fill, stroke, strokeWeight) {
  var svgStr = params.svg;
  if (!svgStr) throw new Error("SVG type requires 'svg' param with SVG markup string");
  var node = figma.createNodeFromSvg(svgStr);
  if (width && height) node.resize(width, height);

  if ((fill || stroke) && node.findAll) {
    var allVectors = node.findAll(function(n) { return n.type === "VECTOR"; });
    for (var vi = 0; vi < allVectors.length; vi++) {
      var vec = allVectors[vi];
      var hasFill   = vec.fills   && vec.fills.length   > 0 && vec.fills[0].type === "SOLID";
      var hasStroke = vec.strokes && vec.strokes.length  > 0;
      if (fill) {
        if (hasFill || !hasStroke) vec.fills   = solidFill(fill);
        else                        vec.strokes = solidStroke(fill);
      }
      if (stroke) { vec.strokes = solidStroke(stroke); vec.strokeWeight = strokeWeight; }
    }
  }
  return node;
}

function buildVector(params, width, height, fill, stroke, strokeWeight) {
  var pathData = params.d || params.path;
  var pathsArr = params.paths;
  if (!pathData && !pathsArr) {
    throw new Error('VECTOR type requires "d" (path data string) or "paths" (array of {d, windingRule})');
  }

  var node = figma.createVector();
  try {
    node.resize(width, height);
    if (pathsArr && Array.isArray(pathsArr)) {
      node.vectorPaths = pathsArr.map(function(p) {
        var raw = typeof p === "string" ? p : p.d;
        return { data: normalizeSvgPath(raw), windingRule: (typeof p === "object" && p.windingRule) ? p.windingRule : "NONZERO" };
      });
    } else {
      node.vectorPaths = [{ data: normalizeSvgPath(pathData), windingRule: params.windingRule || "NONZERO" }];
    }
    // BUG-04: Figma resets bounding box to path geometry after setVectorPaths.
    // Re-apply resize() so the node matches the requested dimensions.
    if (width && height) node.resize(width, height);
    // BUG-02: VECTOR fill now supports gradient objects (same as FRAME/RECTANGLE)
    node.fills = fill ? buildFillArray(fill, params.fillOpacity) : [];
    if (stroke) { node.strokes = solidStroke(stroke); node.strokeWeight = strokeWeight; }
    if (params.strokeCap)  node.strokeCap  = params.strokeCap;
    if (params.strokeJoin) node.strokeJoin = params.strokeJoin;
  } catch (vectorErr) {
    try { node.remove(); } catch(e) {}
    throw new Error("VECTOR path error: " + vectorErr.message + ". Check that 'd' is valid SVG path data (commas and A/arc are now supported).");
  }
  return node;
}

function buildImage(params, width, height, cornerRadius, stroke, strokeWeight) {
  var imgData = params.imageData;
  if (!imgData) throw new Error("IMAGE type requires 'imageData' param with base64 string");

  var strippedData = imgData.indexOf(",") !== -1 ? imgData.substring(imgData.indexOf(",") + 1) : imgData;
  var cleanData = strippedData.replace(/[^A-Za-z0-9+/=]/g, "");
  var outLen = Math.floor(cleanData.replace(/=/g, "").length * 3 / 4);
  var raw = new Uint8Array(outLen);
  var j = 0;
  for (var ci = 0; ci < cleanData.length; ci += 4) {
    // Use explicit undefined check — lookup['A']===0 is valid, lookup[undefined] must not default to 0
    var a = (_B64_LOOKUP[cleanData[ci]]     !== undefined) ? _B64_LOOKUP[cleanData[ci]]     : 0;
    var b = (_B64_LOOKUP[cleanData[ci + 1]] !== undefined) ? _B64_LOOKUP[cleanData[ci + 1]] : 0;
    var c = (_B64_LOOKUP[cleanData[ci + 2]] !== undefined) ? _B64_LOOKUP[cleanData[ci + 2]] : 0;
    var d = (_B64_LOOKUP[cleanData[ci + 3]] !== undefined) ? _B64_LOOKUP[cleanData[ci + 3]] : 0;
    raw[j++] = (a << 2) | (b >> 4);
    if (j < outLen) raw[j++] = ((b & 15) << 4) | (c >> 2);
    if (j < outLen) raw[j++] = ((c & 3) << 6) | d;
  }

  var image = figma.createImage(raw);
  var node = figma.createRectangle();
  node.resize(width, height);
  if (cornerRadius !== undefined) node.cornerRadius = cornerRadius;
  node.fills = [{ type: "IMAGE", imageHash: image.hash, scaleMode: params.scaleMode || "FILL" }];
  if (stroke) { node.strokes = solidStroke(stroke); node.strokeWeight = strokeWeight; }
  return node;
}

// ─── handlers.create ──────────────────────────────────────────────────────────

handlers.create = async (params) => {
  if (!params || !params.type) {
    var keys = Object.keys(params || {});
    throw new Error("create: 'type' is required. Received keys: [" + keys.join(", ") + "]. Use type: FRAME|RECTANGLE|ELLIPSE|LINE|TEXT|SVG|VECTOR|IMAGE");
  }
  assertKnownKeys("create", params, CREATE_KEYS);

  const {
    type, parentId, name,
    x = 0, y = 0,
    strokeWeight = 1, cornerRadius,
    fontSize = 14, fontWeight = "Regular", lineHeight,
    opacity, visible,
  } = params;
  var hasExplicitWidth  = params.width  !== undefined;
  var hasExplicitHeight = params.height !== undefined;
  var width  = hasExplicitWidth  ? params.width  : 100;
  var height = hasExplicitHeight ? params.height : 100;
  var fill = params.fill;
  // BUG-18/19: accept both `stroke` and `strokeColor` aliases
  var stroke = params.stroke || params.strokeColor || null;
  var warnings = [];

  let parent = figma.currentPage;
  if (parentId) {
    const p = await findNodeByIdAsync(parentId);
    if (!p) throw new Error(
      "parentId \"" + parentId + "\" not found in the current scene. " +
      "If you just created the parent in a previous figma_write call, find it again with " +
      "figma.byKey(key) or figma.query() at the start of this call."
    );
    parent = p;
  }
  var parentAutoLayout = !!(parent.layoutMode && parent.layoutMode !== "NONE");
  // Fail before building the node so a bad call leaves nothing behind.
  if (params.layoutPositioning === "ABSOLUTE" && !parentAutoLayout) {
    throw new Error("layoutPositioning needs an auto-layout parent; \"" + parent.name +
      "\" has no auto-layout, so set x/y directly instead");
  }
  ["layoutSizingHorizontal", "layoutSizingVertical"].forEach(function(axis) {
    if (String(params[axis]).toUpperCase() === "FILL" && !parentAutoLayout) {
      throw new Error(axis + ": \"FILL\" needs an auto-layout parent; \"" + parent.name + "\" has none");
    }
  });

  let node;
  switch (type) {
    case "FRAME":
    case "GROUP":
      node = buildFrame(params, width, height, fill, stroke, strokeWeight);
      break;
    case "RECTANGLE":
      node = buildRectangle(params, width, height, fill, stroke, strokeWeight);
      break;
    case "ELLIPSE":
      node = buildEllipse(params, width, height, fill, stroke, strokeWeight);
      break;
    case "LINE":
      node = buildLine(params, width, stroke, strokeWeight);
      break;
    case "TEXT":
      // BUG-TEXT-01: pass `undefined` for w/h that the caller didn't set,
      // so buildText can default to WIDTH_AND_HEIGHT hug instead of being stuck at 100×100.
      node = await buildText(
        params,
        hasExplicitWidth  ? width  : undefined,
        hasExplicitHeight ? height : undefined,
        fill, fontSize, fontWeight, lineHeight
      );
      break;
    case "SVG":
      node = buildSvg(params, width, height, fill, stroke, strokeWeight);
      break;
    case "VECTOR":
      node = buildVector(params, width, height, fill, stroke, strokeWeight);
      break;
    case "IMAGE":
      node = buildImage(params, width, height, cornerRadius, stroke, strokeWeight);
      break;
    default:
      throw new Error('Unsupported node type: "' + type + '". Use FRAME, RECTANGLE, ELLIPSE, LINE, TEXT, SVG, VECTOR, IMAGE.');
  }

  try {
    if (name)    node.name    = name;
    if (opacity !== undefined) node.opacity = opacity;
    if (visible !== undefined) node.visible = visible;
    if (params.key !== undefined) node.setPluginData(NODE_KEY_DATA, String(params.key));

    if (parent !== figma.currentPage) {
      if (parent.removed) throw new Error("Parent node no longer exists (was it deleted?): " + params.parentId);
      if (parentAutoLayout && params.layoutPositioning !== "ABSOLUTE" && (params.x !== undefined || params.y !== undefined)) {
        warnings.push("x/y ignored: \"" + parent.name + "\" uses auto-layout. Add layoutPositioning: \"ABSOLUTE\" to place it freely.");
      }
      if (params.insertIndex !== undefined) {
        parent.insertChild(Math.max(0, Math.min(params.insertIndex, parent.children.length)), node);
      } else {
        parent.appendChild(node);
      }
    }
    applyLayoutPositioning(node, params);

    // BUG-13/16/17/19: Re-apply TEXT sizing after appendChild — Figma auto-layout can reset
    // textAutoResize to WIDTH_AND_HEIGHT after parenting, causing the node to flash to 100x100.
    if (type === "TEXT") {
      var textAutoResize = params.textAutoResize;
      if (!textAutoResize) {
        if (hasExplicitWidth && hasExplicitHeight) textAutoResize = "NONE";
        else if (hasExplicitWidth)                 textAutoResize = "HEIGHT";
        else                                       textAutoResize = "WIDTH_AND_HEIGHT";
      }
      node.textAutoResize = textAutoResize;
      if (textAutoResize === "NONE" && hasExplicitWidth && hasExplicitHeight) {
        node.resize(width, height);
      } else if (textAutoResize === "HEIGHT" && hasExplicitWidth) {
        node.resize(width, node.height);
      }
    }

    // Sizing last: it must win over the text auto-resize above. An auto-layout
    // frame without an explicit width/height hugs its content on that axis.
    var sizing = { layoutSizingHorizontal: params.layoutSizingHorizontal, layoutSizingVertical: params.layoutSizingVertical };
    if ((type === "FRAME" || type === "GROUP") && params.layoutMode && params.layoutMode !== "NONE") {
      if (!hasExplicitWidth  && sizing.layoutSizingHorizontal === undefined) sizing.layoutSizingHorizontal = "HUG";
      if (!hasExplicitHeight && sizing.layoutSizingVertical   === undefined) sizing.layoutSizingVertical   = "HUG";
    }
    applySizing(node, sizing);

    // x/y set after appendChild — Figma resets position on reparent
    node.x = x;
    node.y = y;
  } catch (e) {
    node.remove();
    throw e;
  }

  journalCreated(params, node);
  var info = nodeToInfo(node);
  if (warnings.length) info.warnings = warnings;
  return info;
};

handlers.modify = async (params) => {
  const node = await resolveNode(params);
  var nodeRef = params.id || params.nodeId || params.targetId || params.name || params.nodeName;
  if (!node) {
    var keys = Object.keys(params || {});
    throw new Error("Node not found: " + nodeRef + ". Received params keys: [" + keys.join(", ") + "]. Use id or name field.");
  }
  if (node.removed) throw new Error("Node was deleted: " + nodeRef);

  assertKnownKeys("modify", params, MODIFY_KEYS);
  assertKeysFitNode(node, params);
  if (params.key !== undefined) node.setPluginData(NODE_KEY_DATA, String(params.key));

  if (params.fontColor !== undefined && params.fill === undefined) params.fill = params.fontColor;
  if (params.fill     !== undefined && "fills"   in node) node.fills   = buildFillArray(params.fill, params.fillOpacity);
  if (params.fills    !== undefined && params.fill === undefined && "fills" in node) node.fills = buildFillsParam(params.fills, params.fillOpacity);
  if (params.fillOpacity !== undefined && params.fill === undefined && "fills" in node && node.fills && node.fills.length) {
    var existingFills = JSON.parse(JSON.stringify(node.fills));
    existingFills[0].opacity = params.fillOpacity;
    node.fills = existingFills;
  }
  // BUG-18: accept both `stroke` and `strokeColor` aliases in modify
  var modifyStroke = params.stroke !== undefined ? params.stroke : (params.strokeColor !== undefined ? params.strokeColor : undefined);
  if (modifyStroke !== undefined && "strokes" in node) {
    node.strokes = solidStroke(modifyStroke, params.strokeOpacity);
    if (params.strokeWeight !== undefined) node.strokeWeight = params.strokeWeight;
    if (params.strokeAlign !== undefined) node.strokeAlign = params.strokeAlign;
    if (params.strokeDashPattern && Array.isArray(params.strokeDashPattern)) node.dashPattern = params.strokeDashPattern;
  }
  // Reparent and float before x/y: Figma resets position on reparent and
  // ignores x/y for in-flow auto-layout children.
  if (params.parentId !== undefined || params.insertIndex !== undefined) await reparentNode(node, params);
  applyLayoutPositioning(node, params);
  if (params.x       !== undefined) node.x = params.x;
  if (params.y       !== undefined) node.y = params.y;
  if (params.opacity !== undefined) node.opacity = params.opacity;
  if (params.visible !== undefined) node.visible = params.visible;
  if (params.name    !== undefined) node.name = params.name;
  applyCornerRadii(node, params);
  if (params.effects !== undefined) {
    if (params.effects === null || (Array.isArray(params.effects) && params.effects.length === 0)) {
      if ("effects" in node) node.effects = [];
    } else {
      applyEffects(node, params.effects);
    }
  }

  if ((params.width !== undefined || params.height !== undefined) && "resize" in node) {
    node.resize(
      params.width  !== undefined ? params.width  : node.width,
      params.height !== undefined ? params.height : node.height
    );
  }

  if (node.type === "TEXT") {
    if (params.content !== undefined || params.fontWeight !== undefined || params.fontFamily !== undefined) {
      const family = params.fontFamily || node.fontName.family;
      const style = params.fontWeight !== undefined ? await resolveFontStyle(family, params.fontWeight) : node.fontName.style;
      await ensureFontLoaded(family, style);
      node.fontName = { family, style };
      if (params.content !== undefined) {
        node.characters = params.content;
        if (params.width === undefined && params.textAutoResize === undefined &&
            node.textAutoResize !== "NONE") {
          node.textAutoResize = "WIDTH_AND_HEIGHT";
        }
        autoPromoteTextHugForReflow(node, params);
      }
    }
    if (params.fontSize !== undefined) node.fontSize = params.fontSize;
    var textAlignH = params.textAlignHorizontal !== undefined ? params.textAlignHorizontal : params.textAlign;
    if (textAlignH !== undefined || params.textAlignVertical !== undefined || params.lineHeight !== undefined ||
        params.textAutoResize !== undefined || params.textDecoration !== undefined) {
      if (node.fontName && node.fontName.family) {
        await ensureFontLoaded(node.fontName.family, node.fontName.style);
      }
      if (params.textAutoResize !== undefined) node.textAutoResize = String(params.textAutoResize).toUpperCase();
      if (params.textDecoration !== undefined) node.textDecoration = String(params.textDecoration).toUpperCase();
      if (textAlignH               !== undefined) node.textAlignHorizontal = String(textAlignH).toUpperCase();
      if (params.textAlignVertical !== undefined) node.textAlignVertical   = params.textAlignVertical.toUpperCase();
      if (params.lineHeight !== undefined) {
        node.lineHeight = (typeof params.lineHeight === "object" && params.lineHeight.unit)
          ? params.lineHeight
          : { value: params.lineHeight, unit: "PIXELS" };
      }
    }
    var modTc = params.textCase || (params.textTransform ? (params.textTransform.toLowerCase() === "uppercase" ? "UPPER" : params.textTransform.toLowerCase() === "lowercase" ? "LOWER" : params.textTransform.toLowerCase() === "capitalize" ? "TITLE" : "ORIGINAL") : null);
    if (modTc) {
      var upperModTc = String(modTc).toUpperCase();
      if (upperModTc === "UPPERCASE") upperModTc = "UPPER";
      if (upperModTc === "LOWERCASE") upperModTc = "LOWER";
      if (upperModTc === "TITLECASE" || upperModTc === "CAPITALIZE") upperModTc = "TITLE";
      try { node.textCase = upperModTc; } catch(e) {}
    }
  }

  // Auto Layout properties (frames, components, instances)
  if ("layoutMode" in node) {
    var removingLayout = params.layoutMode === "NONE" || params.layoutMode === null || params.layoutMode === "";
    if (params.layoutMode !== undefined) {
      node.layoutMode = removingLayout ? "NONE" : params.layoutMode;
    }
    // Only apply align/spacing when the frame actually has auto-layout active
    if (node.layoutMode !== "NONE") {
      if (params.primaryAxisAlignItems !== undefined) node.primaryAxisAlignItems = params.primaryAxisAlignItems;
      if (params.counterAxisAlignItems === "STRETCH") {
        throw new Error(
          "counterAxisAlignItems does not support \"STRETCH\". " +
          "Set counterAxisAlignItems: \"MIN\" and layoutSizingHorizontal/Vertical: \"FILL\" on each child."
        );
      }
      if (params.counterAxisAlignItems !== undefined) node.counterAxisAlignItems = params.counterAxisAlignItems;
    }
    if (params.padding !== undefined) {
      node.paddingTop = node.paddingBottom = node.paddingLeft = node.paddingRight = params.padding;
    }
    if (params.paddingHorizontal !== undefined) { node.paddingLeft = node.paddingRight = params.paddingHorizontal; }
    if (params.paddingVertical   !== undefined) { node.paddingTop  = node.paddingBottom = params.paddingVertical; }
    if (params.paddingTop    !== undefined) node.paddingTop    = params.paddingTop;
    if (params.paddingBottom !== undefined) node.paddingBottom = params.paddingBottom;
    if (params.paddingLeft   !== undefined) node.paddingLeft   = params.paddingLeft;
    if (params.paddingRight  !== undefined) node.paddingRight  = params.paddingRight;
    if (params.itemSpacing   !== undefined) node.itemSpacing   = params.itemSpacing;
    if (params.clipsContent  !== undefined) node.clipsContent  = params.clipsContent;
  }

  if (params.clipsContent !== undefined && !("layoutMode" in node) && "clipsContent" in node) node.clipsContent = params.clipsContent;

  applySizing(node, params);

  return nodeToInfo(node);
};

// BUG-DEL-01: warn when deleting frames containing COMPONENT masters.
// Deleting a COMPONENT cascades: every INSTANCE of that component in the document
// is broken. Figma also has reported edge cases where deleting a frame containing
// component masters that bind variables can leave variable collections in an odd
// state. We don't block — the user might know what they're doing — but we surface
// the warning in the response so AI agents can confirm before proceeding.
function findComponentDescendants(node) {
  if (!node || node.removed || typeof node.findAll !== "function") return [];
  try {
    return node.findAll(function(n) { return n.type === "COMPONENT" || n.type === "COMPONENT_SET"; });
  } catch (e) { return []; }
}

function describeDeleteImpact(node) {
  var impact = { hasComponents: false, componentCount: 0, componentNames: [] };
  if (!node) return impact;
  if (node.type === "COMPONENT" || node.type === "COMPONENT_SET") {
    impact.hasComponents = true;
    impact.componentCount = 1;
    impact.componentNames = [node.name];
    return impact;
  }
  var found = findComponentDescendants(node);
  if (found.length > 0) {
    impact.hasComponents = true;
    impact.componentCount = found.length;
    impact.componentNames = found.slice(0, 10).map(function(c) { return c.name; });
  }
  return impact;
}

// "delete" is a JS reserved keyword — use bracket notation
var DELETE_KEYS = new Set(["id", "nodeId", "ids", "force"]);

// Checks every target before removing any, so a blocked id leaves the rest intact.
handlers["delete"] = async (params) => {
  assertKnownKeys("delete", params, DELETE_KEYS);
  var ids = Array.isArray(params.ids) ? params.ids : [params.id || params.nodeId];
  if (!ids.length || !ids[0]) throw new Error("delete: pass id or ids");
  var targets = [], alreadyGone = [];
  for (var di = 0; di < ids.length; di++) {
    var n = await findNodeByIdAsync(ids[di]);
    if (!n || n.removed) { alreadyGone.push(ids[di]); continue; }
    var impact = describeDeleteImpact(n);
    if (impact.hasComponents && params.force !== true) {
      throw new Error("delete: " + n.id + " contains " + impact.componentCount + " component master(s) (" +
        impact.componentNames.join(", ") + "); deleting breaks every instance in the file. Pass force: true to override.");
    }
    targets.push(n);
  }
  var deleted = targets.map(function(n) { var id = n.id; n.remove(); return id; });
  return { deleted: deleted, alreadyGone: alreadyGone };
};

handlers.append = async function(params) {
  var parentId   = params.parentId   || null;
  var parentName = params.parentName || null;
  var childId    = params.childId    || null;
  var childName  = params.childName  || null;
  var parent = parentId   ? (await findNodeByIdAsync(parentId))   : (parentName ? findNodeByName(parentName) : null);
  var child  = childId    ? (await findNodeByIdAsync(childId))    : (childName  ? findNodeByName(childName)  : null);
  if (!parent || !child) throw new Error("Parent or child not found");
  parent.appendChild(child);
  return { parentId: parent.id, childId: child.id };
};

handlers.listComponents = async () => {
  await figma.loadAllPagesAsync();
  const comps = figma.root.findAllWithCriteria({ types: ["COMPONENT"] });
  return comps.map(c => ({ id: c.id, name: c.name, key: c.key || null }));
};

// Expose Figma's loadAllPagesAsync so user code can opt-in before queries that
// span pages. Required when the plugin runs under documentAccess: dynamic-page
// (see plugin/manifest.json) — figma.root.findOne / findAll only sees pages
// that have been loaded. Internal handlers call this themselves where needed
// (listComponents, instantiate); this op lets user code do the same from
// figma_write blocks.
handlers.loadAllPagesAsync = async function() {
  await figma.loadAllPagesAsync();
  return { loaded: true, pageCount: figma.root.children.length };
};

var INSTANTIATE_KEYS = new Set(["componentId", "componentName", "parentId", "insertIndex", "x", "y", "name", "key", "overrides"]);
var OVERRIDE_KEYS = new Set(["text", "fill", "stroke", "opacity", "visible", "fontSize", "cornerRadius"]);

handlers.instantiate = async function(params) {
  assertKnownKeys("instantiate", params, INSTANTIATE_KEYS);
  var componentId   = params.componentId   || null;
  var componentName = params.componentName || null;
  var parentId      = params.parentId      || null;
  var x = params.x !== undefined ? params.x : 0;
  var y = params.y !== undefined ? params.y : 0;
  var overrides = params.overrides || null;

  // Under documentAccess: dynamic-page, figma.root.findOne only sees already-
  // loaded pages. Without this, lookup silently fails for any component that
  // lives on an unvisited page — even when called by id. listComponents does
  // the same dance for the same reason.
  await figma.loadAllPagesAsync();

  var comp = null;
  if (componentId) {
    comp = figma.root.findOne(function(n) { return n.id === componentId && n.type === "COMPONENT"; });
  } else if (componentName) {
    comp = figma.root.findOne(function(n) { return n.name === componentName && n.type === "COMPONENT"; });
  }
  if (!comp) throw new Error("Component " + (componentId || componentName) + " not found");

  var parent = parentId ? await findNodeByIdAsync(parentId) : null;
  if (parentId && !parent) throw new Error("parentId not found: " + parentId);
  var inst = comp.createInstance();
  try {
    if (parent) parent.insertChild(params.insertIndex === undefined ? parent.children.length : Math.max(0, Math.min(params.insertIndex, parent.children.length)), inst);
    inst.x = x;
    inst.y = y;
    if (params.name) inst.name = params.name;
    if (params.key !== undefined) inst.setPluginData(NODE_KEY_DATA, String(params.key));
    await applyInstanceOverrides(inst, overrides);
  } catch (e) {
    inst.remove();
    throw e;
  }
  journalCreated(params, inst);
  return nodeToInfo(inst);
};

async function applyInstanceOverrides(inst, overrides) {
  if (overrides && typeof overrides === "object") {
    var overrideKeys = Object.keys(overrides);
    for (var oi = 0; oi < overrideKeys.length; oi++) {
      var layerName = overrideKeys[oi];
      var ov = overrides[layerName];
      // Use IIFE to capture layerName for synchronous findOne callback
      var target = (function(lName) {
        return inst.findOne(function(n) { return n.name === lName; });
      })(layerName);
      if (!target) throw new Error("overrides: no layer named \"" + layerName + "\" in instance of \"" + inst.name + "\"");
      assertKnownKeys("overrides." + layerName, ov, OVERRIDE_KEYS);

      if (ov.text !== undefined && target.type === "TEXT") {
        if (target.fontName && target.fontName.family) {
          await ensureFontLoaded(target.fontName.family, target.fontName.style);
        }
        target.characters = String(ov.text);
      }
      if (ov.fill !== undefined) {
        var fillNorm = normalizeHex(ov.fill);
        if (fillNorm) target.fills = solidFill(fillNorm);
      }
      if (ov.stroke !== undefined) {
        var strokeNorm = normalizeHex(ov.stroke);
        if (strokeNorm) target.strokes = solidStroke(strokeNorm);
      }
      if (ov.opacity    !== undefined) target.opacity    = ov.opacity;
      if (ov.visible    !== undefined) target.visible    = Boolean(ov.visible);
      if (ov.fontSize   !== undefined && target.type === "TEXT") {
        if (target.fontName && target.fontName.family) {
          await ensureFontLoaded(target.fontName.family, target.fontName.style);
        }
        target.fontSize = ov.fontSize;
      }
      if (ov.cornerRadius !== undefined && "cornerRadius" in target) {
        target.cornerRadius = ov.cornerRadius;
      }
    }
  }
}

// ─── INSTANCE PROPERTY OPERATIONS ─────────────────────────────────────────────
// These three ops were on the WRITE_OPS / READ_OPS allowlist since v2.4.0 but
// never had plugin-side handlers — calls would fail with `Unknown operation
// "setComponentProperties"`. Without setComponentProperties in particular,
// addComponentProperty + bindComponentPropertyToText (PR #8) couldn't be
// exercised end-to-end: instance text overrides had no way to flow through
// the bound property, so auto-layout never recalculated.

// getMainComponentSafe lives in utils.js — shared with the read handlers.

// setComponentProperties — set property values on a component instance.
// Wraps Figma's InstanceNode.setProperties({ "name#id": value, ... }).
// Values: TEXT/VARIANT → string, BOOLEAN → boolean, INSTANCE_SWAP → component key string.
// Accepts bare property names (e.g. "label") and resolves to "label#5:0".
handlers.setComponentProperties = async function(params) {
  var nodeId = params.id || params.nodeId;
  var properties = params.properties;

  if (!nodeId) throw new Error("id is required");
  if (!properties || typeof properties !== "object") {
    throw new Error("properties object is required");
  }

  var node = await findNodeByIdAsync(nodeId);
  if (!node) throw new Error("Node not found: " + nodeId);
  if (node.type !== "INSTANCE") {
    throw new Error("setComponentProperties requires an INSTANCE node, got: " + node.type);
  }

  // Resolve names against the instance itself: componentProperties covers
  // variant and non-variant mains alike, while main.componentPropertyDefinitions
  // throws for any variant.
  var defs = {};
  try { defs = node.componentProperties || {}; } catch (e) {}

  var resolvedMap = {};
  var unresolved = [];
  var keys = Object.keys(properties);
  for (var i = 0; i < keys.length; i++) {
    var name = keys[i];
    var resolved = resolvePropertyKey(defs, name);
    if (!resolved) {
      unresolved.push(name);
      continue;
    }
    resolvedMap[resolved] = properties[name];
  }

  if (unresolved.length > 0) {
    var available = Object.keys(defs);
    throw new Error(
      "Unknown component property: " + unresolved.join(", ") +
      ". Available on main component: " + (available.length ? available.join(", ") : "(none — call addComponentProperty first)")
    );
  }

  node.setProperties(resolvedMap);

  // Promote any bound TEXT child's layoutSizing axis to HUG so a hug-width
  // auto-layout parent actually reflows in response to the property change.
  // Without this, setProperties updates `characters` but `layoutSizingHorizontal:
  // FIXED` (Figma's default after parenting) overrides textAutoResize and
  // pins the child width — the parent never sees a size change to react to.
  //
  // Targeted at TEXT-type properties only; BOOLEAN / INSTANCE_SWAP don't
  // change child dimensions in a way the auto-layout needs help with.
  //
  // Inline rather than calling a shared helper because the layout-sizing
  // exposure + auto-promote helper lives in a separate PR (feat/layout-
  // sizing-axes) that may merge after this one. Once both land, this block
  // can be replaced with a call to the shared autoPromoteTextHugForReflow.
  if (typeof node.findAll === "function") {
    var changedKeys = Object.keys(resolvedMap);
    for (var ck = 0; ck < changedKeys.length; ck++) {
      var key = changedKeys[ck];
      var def = defs[key];
      if (!def || def.type !== "TEXT") continue;
      var boundTexts = node.findAll(function(t) {
        return t.type === "TEXT"
          && t.componentPropertyReferences
          && t.componentPropertyReferences.characters === key;
      });
      for (var bi = 0; bi < boundTexts.length; bi++) {
        var textNode = boundTexts[bi];
        var p = textNode.parent;
        if (!p || !p.layoutMode || p.layoutMode === "NONE") continue;
        var horizontal = p.layoutMode === "HORIZONTAL";
        var parentHugs = p.primaryAxisSizingMode === "AUTO" ||
          (horizontal ? p.layoutSizingHorizontal === "HUG"
                      : p.layoutSizingVertical   === "HUG");
        if (!parentHugs) continue;
        try {
          if (horizontal && "layoutSizingHorizontal" in textNode) {
            textNode.layoutSizingHorizontal = "HUG";
          } else if (!horizontal && "layoutSizingVertical" in textNode) {
            textNode.layoutSizingVertical = "HUG";
          }
        } catch (e) {}
      }
    }
  }

  return {
    id: node.id,
    name: node.name,
    properties: node.componentProperties,
    appliedKeys: Object.keys(resolvedMap),
  };
};

// getComponentProperties — read current property values from an instance.
// Returns Figma's InstanceNode.componentProperties as-is.
handlers.getComponentProperties = async function(params) {
  var nodeId = params.id || params.nodeId;
  if (!nodeId) throw new Error("id is required");

  var node = await findNodeByIdAsync(nodeId);
  if (!node) throw new Error("Node not found: " + nodeId);
  if (node.type !== "INSTANCE") {
    throw new Error("getComponentProperties requires an INSTANCE node, got: " + node.type);
  }

  return {
    id: node.id,
    name: node.name,
    properties: node.componentProperties,
  };
};

// swapComponent — point an instance at a different main component.
// Wraps Figma's InstanceNode.swapComponent(targetComponent).
// Different from INSTANCE_SWAP component properties (which are nested swaps
// inside a main component's slots) — this swaps the instance's own main.
handlers.swapComponent = async function(params) {
  var nodeId = params.id || params.nodeId;
  var componentId = params.componentId || params.targetComponentId;

  if (!nodeId) throw new Error("id is required");
  if (!componentId) throw new Error("componentId is required");

  var instance = await findNodeByIdAsync(nodeId);
  if (!instance) throw new Error("Instance not found: " + nodeId);
  if (instance.type !== "INSTANCE") {
    throw new Error("swapComponent source must be an INSTANCE, got: " + instance.type);
  }

  var target = await findNodeByIdAsync(componentId);
  if (!target) throw new Error("Target component not found: " + componentId);
  if (target.type !== "COMPONENT") {
    throw new Error("swapComponent target must be a COMPONENT, got: " + target.type);
  }

  instance.swapComponent(target);

  return {
    id: instance.id,
    name: instance.name,
    newMainComponentId: target.id,
    newMainComponentName: target.name,
  };
};

// ─── ALIASES FOR WRITE HANDLERS ──────────────────────────────────────────────
handlers.list_pages = handlers.listPages;
handlers.set_page = handlers.setPage;
handlers.create_page = handlers.createPage;
handlers.list_components = handlers.listComponents;
handlers.load_all_pages_async = handlers.loadAllPagesAsync;
handlers.loadAllPages = handlers.loadAllPagesAsync;
handlers.load_all_pages = handlers.loadAllPagesAsync;
handlers.set_component_properties = handlers.setComponentProperties;
handlers.get_component_properties = handlers.getComponentProperties;
handlers.swap_component = handlers.swapComponent;
handlers.update = handlers.modify;
handlers.remove = handlers["delete"];

