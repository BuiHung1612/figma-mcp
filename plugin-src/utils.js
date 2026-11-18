// ─── UTILS ────────────────────────────────────────────────────────────────────

// Map common CSS color names to hex (AI sometimes passes color names instead of hex)
var CSS_COLOR_MAP = {
  "white": "#FFFFFF", "black": "#000000", "red": "#FF0000", "green": "#008000",
  "blue": "#0000FF", "yellow": "#FFFF00", "orange": "#FFA500", "purple": "#800080",
  "pink": "#FFC0CB", "gray": "#808080", "grey": "#808080", "transparent": "NONE",
  "teal": "#008080", "cyan": "#00FFFF", "magenta": "#FF00FF", "lime": "#00FF00",
  "navy": "#000080", "brown": "#A52A2A", "silver": "#C0C0C0", "gold": "#FFD700",
};

function normalizeHex(value) {
  if (!value || String(value).toLowerCase() === "none" || String(value).toLowerCase() === "transparent") return null;
  var c = parseColorValue(value);
  return [c.r, c.g, c.b].map(function(v) { return Math.round(v * 255).toString(16).padStart(2, "0"); }).join("");
}

function extractColorAlpha(value) {
  if (!value) return null;
  var s = String(value).trim().replace(/^#/, "");
  var explicit = typeof value === "object" ? value.a !== undefined
    : /^(?:[0-9a-f]{4}|[0-9a-f]{8})$/i.test(s) || /^(?:rgba|hsla)\(/i.test(s) || s.indexOf("/") >= 0 || (/^(?:rgb|hsl)\(/i.test(s) && s.split(",").length === 4) || s.toLowerCase() === "transparent";
  return explicit ? parseColorValue(value).a : null;
}

function hexToRgb(value) {
  var c = parseColorValue(value);
  return { r: c.r, g: c.g, b: c.b };
}

function colorToCss(color, opacity) {
  if (!color) return null;
  var r = Math.round((color.r !== undefined ? color.r : 0) * 255);
  var g = Math.round((color.g !== undefined ? color.g : 0) * 255);
  var b = Math.round((color.b !== undefined ? color.b : 0) * 255);
  var a = opacity !== undefined ? opacity : (color.a !== undefined ? color.a : 1);
  if (a !== undefined && a < 1 && a >= 0) {
    var aFormatted = Math.round(a * 1000000) / 1000000;
    return "rgba(" + r + ", " + g + ", " + b + ", " + aFormatted + ")";
  }
  return "#" + [r, g, b].map(function(v) { return v.toString(16).padStart(2, "0"); }).join("");
}

function rgbToHex(color, opacity) {
  if (!color) return "#000000";
  return colorToCss(color, opacity);
}

function solidFill(hex, fillOpacity) {
  // "NONE", empty, transparent → no fill
  if (!hex) return [];
  var normalized = normalizeHex(hex);
  if (!normalized) return [];
  var fill = { type: "SOLID", color: hexToRgb(hex) };
  // BUG-02: extract alpha from 8-digit hex or rgba() automatically (explicit fillOpacity wins)
  if (fillOpacity !== undefined) {
    fill.opacity = fillOpacity;
  } else {
    var extracted = extractColorAlpha(hex);
    if (extracted !== null && extracted < 1) fill.opacity = extracted;
  }
  return [fill];
}

function solidStroke(hex, strokeOpacity) {
  if (!hex) return [];
  var normalized = normalizeHex(hex);
  if (!normalized) return [];
  var stroke = { type: "SOLID", color: hexToRgb(hex) };
  if (strokeOpacity !== undefined) {
    stroke.opacity = strokeOpacity;
  } else {
    var extracted = extractColorAlpha(hex);
    if (extracted !== null && extracted < 1) stroke.opacity = extracted;
  }
  return [stroke];
}

// ── Variable & Token Deep Resolver ───────────────────────────────────────────
var variableCache = new Map();
var stalledVariableLookups = [];

// Some Figma variable lookups (e.g. an alias into a library this file can no
// longer reach) never settle. Bound them so one lookup can't stall a request
// and, through the serial request queue, every request after it.
function settleWithin(promise, ms, fallback, label) {
  return new Promise(function(resolve) {
    var timer = setTimeout(function() {
      if (label && stalledVariableLookups.length < 50) stalledVariableLookups.push(label);
      resolve(fallback);
    }, ms);
    Promise.resolve(promise).then(
      function(v) { clearTimeout(timer); resolve(v); },
      function() { clearTimeout(timer); resolve(fallback); });
  });
}

async function getVariableSafeAsync(id) {
  if (!id) return null;
  if (variableCache.has(id)) return variableCache.get(id);
  try {
    if (figma.variables && typeof figma.variables.getVariableByIdAsync === "function") {
      var v = await settleWithin(figma.variables.getVariableByIdAsync(id), 2000, null, "variable " + id);
      // Cache misses too, so an unreachable id costs one timeout per runtime.
      variableCache.set(id, v || null);
      return v;
    }
  } catch(e) {}
  return null;
}

function getVariableCollectionSafeAsync(id) {
  if (!id || !figma.variables || typeof figma.variables.getVariableCollectionByIdAsync !== "function") return Promise.resolve(null);
  return settleWithin(figma.variables.getVariableCollectionByIdAsync(id), 2000, null, "collection " + id);
}

// Recursively resolves a variable (and its VARIABLE_ALIAS chains) to concrete primitive value & hex
async function resolveVariableValueAsync(variableOrId, contextNodeOrModeMap, depth, visited) {
  if (!variableOrId) return null;
  depth = depth || 0;
  visited = visited || {};
  if (depth > 6) return null;

  var variable = null;
  if (typeof variableOrId === "string") {
    if (visited[variableOrId]) return null;
    visited[variableOrId] = true;
    variable = await getVariableSafeAsync(variableOrId);
  } else {
    variable = variableOrId;
    if (variable && variable.id) {
      if (visited[variable.id]) return null;
      visited[variable.id] = true;
    }
  }

  if (!variable || !variable.valuesByMode) return null;

  // Determine active mode
  var modeId = null;
  var colId = variable.variableCollectionId;
  if (contextNodeOrModeMap) {
    if (typeof contextNodeOrModeMap === "object" && contextNodeOrModeMap.resolvedVariableModes) {
      modeId = contextNodeOrModeMap.resolvedVariableModes[colId];
    } else if (typeof contextNodeOrModeMap === "object" && contextNodeOrModeMap[colId]) {
      modeId = contextNodeOrModeMap[colId];
    } else if (typeof contextNodeOrModeMap === "string") {
      modeId = contextNodeOrModeMap;
    }
  }
  var availableModes = Object.keys(variable.valuesByMode);
  if (!modeId || variable.valuesByMode[modeId] === undefined) {
    var collection = await getVariableCollectionSafeAsync(colId);
    modeId = collection ? collection.defaultModeId : (availableModes.length === 1 ? availableModes[0] : null);
  }
  if (!modeId) return null;

  // Walk aliases through each collection's active mode so alias names are
  // reported; Figma's resolveForConsumer only supplies the final value.
  var rawVal = variable.valuesByMode[modeId];
  var isAliasVal = rawVal && typeof rawVal === "object" && rawVal.type === "VARIABLE_ALIAS";
  if (!isAliasVal && contextNodeOrModeMap && contextNodeOrModeMap.resolvedVariableModes && typeof variable.resolveForConsumer === "function") {
    try {
      var consumerValue = variable.resolveForConsumer(contextNodeOrModeMap).value;
      if (consumerValue !== undefined) rawVal = consumerValue;
    } catch (e) {}
  }
  if (rawVal === undefined || rawVal === null) return null;

  // If alias, follow recursively down to primitive token
  if (typeof rawVal === "object" && rawVal.type === "VARIABLE_ALIAS" && rawVal.id) {
    var targetResult = await resolveVariableValueAsync(rawVal.id, contextNodeOrModeMap, depth + 1, visited);
    return {
      type: "ALIAS",
      name: variable.name,
      variableId: variable.id,
      resolvedType: variable.resolvedType,
      targetId: rawVal.id,
      targetName: targetResult ? (targetResult.primitiveName || targetResult.name) : null,
      primitiveName: targetResult ? (targetResult.primitiveName || targetResult.name) : variable.name,
      resolvedValue: targetResult ? targetResult.resolvedValue : null,
      hex: targetResult ? targetResult.hex : null,
      raw: rawVal,
    };
  }

  // If RGBA color
  if (typeof rawVal === "object" && "r" in rawVal && "g" in rawVal && "b" in rawVal) {
    var hex = rgbToHex(rawVal, rawVal.a);
    var alpha = rawVal.a !== undefined ? Math.round(rawVal.a * 1000) / 1000 : 1;
    return {
      type: "COLOR",
      name: variable.name,
      variableId: variable.id,
      resolvedType: "COLOR",
      resolvedValue: hex,
      hex: hex,
      alpha: alpha,
      primitiveName: variable.name,
    };
  }

  // Primitive value
  return {
    type: variable.resolvedType || typeof rawVal,
    name: variable.name,
    variableId: variable.id,
    resolvedType: variable.resolvedType || typeof rawVal,
    resolvedValue: rawVal,
    primitiveName: variable.name,
  };
}

// Paint/effect helpers (buildFillArray, buildGradientPaint, buildEffect,
// applyEffects, applyCornerRadii) live in paint-and-effects.js.

// figma.mixed is a Symbol. Reading a per-segment property (fontSize, fontName,
// fills, letterSpacing, …) on a TEXT node with more than one style returns it
// WITHOUT throwing, so every read path must test for it explicitly — a plain
// truthiness check happily passes the Symbol straight into the payload.
function isMixed(value) {
  return typeof value === "symbol";
}

// Top-most visible SOLID paint (Figma paint arrays are ordered bottom→top).
function firstSolidHex(paints) {
  if (!paints || isMixed(paints) || !paints.length) return null;
  for (var i = paints.length - 1; i >= 0; i--) {
    if (paints[i].type === "SOLID" && paints[i].visible !== false) {
      return colorToCss(paints[i].color, paints[i].opacity);
    }
  }
  return null;
}

function getFillHex(node) {
  if (!node) return null;
  // Mixed fills (multi-style text): recover the colour of the first styled segment.
  if (isMixed(node.fills)) {
    try {
      if (typeof node.getStyledTextSegments === "function") {
        var segs = node.getStyledTextSegments(["fills"]);
        for (var i = 0; i < segs.length; i++) {
          var hex = firstSolidHex(segs[i].fills);
          if (hex) return hex;
        }
      }
    } catch(e) {}
    return null;
  }
  return firstSolidHex(node.fills);
}

function getStrokeHex(node) {
  if (!node || isMixed(node.strokes)) return null;
  return firstSolidHex(node.strokes);
}

// Under documentAccess: dynamic-page (set in plugin/manifest.json), reading
// `instance.mainComponent` synchronously THROWS — it doesn't return null.
// Always go through getMainComponentAsync when available; fall back to the
// sync getter only for older plugin runtimes that don't have the async API.
async function getMainComponentSafe(instance) {
  if (!instance || instance.type !== "INSTANCE") return null;
  try {
    if (typeof instance.getMainComponentAsync === "function") {
      return await instance.getMainComponentAsync();
    }
  } catch(e) { return null; }
  try { return instance.mainComponent; } catch(e) { return null; }
}

// SVG path helpers (normalizeSvgPath, arcToCubicSegments) live in svg-path-helpers.js.

// fontWeight → style names families actually use. Families disagree ("Semi Bold"
// in Inter, "Demi Bold" in Avenir Next), so the first one the family has wins.
const FONT_STYLE_CANDIDATES = {
  Thin: ["Thin", "Hairline"], ExtraLight: ["Extra Light", "ExtraLight", "Ultra Light", "UltraLight"],
  Light: ["Light"], Regular: ["Regular", "Roman", "Book", "Normal"], Medium: ["Medium"],
  SemiBold: ["Semi Bold", "SemiBold", "Demi Bold", "DemiBold", "Demi"],
  Bold: ["Bold"], ExtraBold: ["Extra Bold", "ExtraBold", "Ultra Bold", "UltraBold"],
  Heavy: ["Heavy"], Black: ["Black"],
};
FONT_STYLE_CANDIDATES["Semi Bold"] = FONT_STYLE_CANDIDATES["Demi Bold"] = FONT_STYLE_CANDIDATES.DemiBold = FONT_STYLE_CANDIDATES.SemiBold;
FONT_STYLE_CANDIDATES["Extra Bold"] = FONT_STYLE_CANDIDATES.UltraBold = FONT_STYLE_CANDIDATES.ExtraBold;

var availableFontStyles = null; // family → [style], loaded once per plugin session
async function fontStylesFor(family) {
  if (!availableFontStyles) {
    availableFontStyles = new Map();
    var fonts = typeof figma.listAvailableFontsAsync === "function" ? await figma.listAvailableFontsAsync() : [];
    for (var i = 0; i < fonts.length; i++) {
      var f = fonts[i].fontName;
      if (!availableFontStyles.has(f.family)) availableFontStyles.set(f.family, []);
      availableFontStyles.get(f.family).push(f.style);
    }
  }
  return availableFontStyles.get(family) || null;
}

// Resolve a fontWeight ("SemiBold") or literal style ("Demi Bold") to a style
// the family has. Throws instead of substituting another weight or family.
async function resolveFontStyle(family, weight) {
  var wanted = weight || "Regular";
  var styles = await fontStylesFor(family);
  if (!styles) return (FONT_STYLE_CANDIDATES[wanted] || [wanted])[0]; // unknown list: trust the caller
  var candidates = [wanted].concat(FONT_STYLE_CANDIDATES[wanted] || []);
  for (var i = 0; i < candidates.length; i++) {
    if (styles.indexOf(candidates[i]) !== -1) return candidates[i];
  }
  throw new Error("Font \"" + family + "\" has no " + wanted + " style. Available: " + styles.join(", "));
}

// ── Font Loading Cache ────────────────────────────────────────────────────────
var loadedFonts = new Set();
var fontLoadingPromises = new Map();

// Loads exactly family/style or throws. No fallback: a silent swap to Inter
// produced the wrong font while reporting success.
async function ensureFontLoaded(family, style) {
  if (!family) family = "Inter";
  if (!style) style = "Regular";
  var key = family + ":" + style;
  if (loadedFonts.has(key)) return;
  if (fontLoadingPromises.has(key)) return fontLoadingPromises.get(key);

  var promise = (async function() {
    try {
      await figma.loadFontAsync({ family: family, style: style });
      loadedFonts.add(key);
    } catch (e) {
      var styles = await fontStylesFor(family);
      throw new Error("Font \"" + family + " " + style + "\" is not available. " +
        (styles ? "Styles of " + family + ": " + styles.join(", ") : "Family \"" + family + "\" is not installed; use figma.listAvailableFonts({ query })") + ".");
    } finally {
      fontLoadingPromises.delete(key);
    }
  })();

  fontLoadingPromises.set(key, promise);
  return promise;
}

// ── Asset & SVG Memory Cache ──────────────────────────────────────────────────
var assetCache = new Map();
function getCachedAsset(key) { return assetCache.get(key) || null; }
function setCachedAsset(key, data) {
  if (assetCache.size > 500) {
    var firstKey = assetCache.keys().next().value;
    assetCache.delete(firstKey);
  }
  assetCache.set(key, data);
}

// ── Node Lookup Cache ────────────────────────────────────────────────────────
var nodeCache = new Map();

function cacheNode(node) {
  if (node && node.id) {
    nodeCache.set(node.id, node);
    if (nodeCache.size > 1000) {
      var firstKey = nodeCache.keys().next().value;
      nodeCache.delete(firstKey);
    }
  }
  return node;
}

function normalizeNodeId(id) {
  if (!id || typeof id !== "string") return id;
  var clean = id.trim().replace(/^['"]|['"]$/g, "");
  try { clean = decodeURIComponent(clean); } catch(e) {}
  // Convert URL hyphenated IDs (e.g. 2715-40862) to colon notation (2715:40862)
  clean = clean.replace(/(\d+)-(\d+)/g, "$1:$2");
  return clean;
}

function getNodeNotFoundContext(id, name) {
  var currentPageName = figma.currentPage ? figma.currentPage.name : "unknown";
  var availablePages = figma.root && figma.root.children ? figma.root.children.map(function(p) { return p.name; }).join(", ") : "none";
  var selInfo = figma.currentPage && figma.currentPage.selection && figma.currentPage.selection.length > 0
    ? figma.currentPage.selection.map(function(s) { return s.id + " (" + s.name + ")"; }).join(", ")
    : "none";
  var target = id || name || "no id/name given";
  return "Node not found: " + target + ". [Active Page: \"" + currentPageName + "\", Available Pages: [" + availablePages + "], Current Selection: [" + selInfo + "]]. Use figma_read get_page_nodes to list nodes or set_page to switch pages.";
}

function findNodeById(id) {
  if (!id) return null;
  var cleanId = normalizeNodeId(id);
  if (figma.currentPage.id === cleanId || figma.currentPage.id === id) return figma.currentPage;
  if (figma.root.id === cleanId || figma.root.id === id) return figma.root;
  if (nodeCache.has(cleanId)) {
    var cached = nodeCache.get(cleanId);
    if (!cached.removed) return cached;
    nodeCache.delete(cleanId);
  }
  if (nodeCache.has(id)) {
    var cached2 = nodeCache.get(id);
    if (!cached2.removed) return cached2;
    nodeCache.delete(id);
  }
  try {
    if (typeof figma.getNodeById === "function") {
      var node = figma.getNodeById(cleanId) || (cleanId !== id ? figma.getNodeById(id) : null);
      if (node && !node.removed) return cacheNode(node);
    }
  } catch(e) {}
  // Check selection
  for (var i = 0; i < figma.currentPage.selection.length; i++) {
    var sel = figma.currentPage.selection[i];
    if (sel.id === cleanId || sel.id === id) return cacheNode(sel);
  }
  return null;
}

async function findNodeByIdAsync(id) {
  if (!id) return null;
  var cleanId = normalizeNodeId(id);
  if (figma.currentPage.id === cleanId || figma.currentPage.id === id) return figma.currentPage;
  if (figma.root.id === cleanId || figma.root.id === id) return figma.root;
  if (nodeCache.has(cleanId)) {
    var cached = nodeCache.get(cleanId);
    if (!cached.removed) return cached;
    nodeCache.delete(cleanId);
  }
  if (nodeCache.has(id)) {
    var cached2 = nodeCache.get(id);
    if (!cached2.removed) return cached2;
    nodeCache.delete(id);
  }

  // 1. Try synchronous lookup first (0ms)
  try {
    if (typeof figma.getNodeById === "function") {
      var syncNode = figma.getNodeById(cleanId) || (cleanId !== id ? figma.getNodeById(id) : null);
      if (syncNode && !syncNode.removed) return cacheNode(syncNode);
    }
  } catch(e) {}

  // 2. Try async lookup
  try {
    var node = await figma.getNodeByIdAsync(cleanId);
    if (!node && cleanId !== id) node = await figma.getNodeByIdAsync(id);
    if (node && !node.removed) return cacheNode(node);
  } catch(e) {}

  // 3. Check selection
  for (var i = 0; i < figma.currentPage.selection.length; i++) {
    var s = figma.currentPage.selection[i];
    if (s.id === cleanId || s.id === id) return cacheNode(s);
  }

  // Instance sub-layer ids ("I1:2;3:4") resolve in step 2 when the sublayer
  // exists; a missing sublayer must not fall back to its outer instance.
  return null;
}

function findNodeByName(name) {
  if (!name) return null;
  if (figma.currentPage.name === name) return figma.currentPage;
  // Search in selection first, then shallow children, then findOne as fallback
  for (var i = 0; i < figma.currentPage.selection.length; i++) {
    if (figma.currentPage.selection[i].name === name) return cacheNode(figma.currentPage.selection[i]);
  }
  for (var j = 0; j < figma.currentPage.children.length; j++) {
    if (figma.currentPage.children[j].name === name) return cacheNode(figma.currentPage.children[j]);
  }
  var found = figma.currentPage.findOne(function(n) { return n.name === name; });
  if (found) return cacheNode(found);

  // Cross-page fallback
  if (figma.root && figma.root.children) {
    for (var p = 0; p < figma.root.children.length; p++) {
      var page = figma.root.children[p];
      if (page === figma.currentPage) continue;
      if (page.name === name) return page;
      // ponytail: sync callers (incl. get_design_context) can't await page.loadAsync(),
      // so only already-loaded pages are searched; unloaded pages throw and are skipped.
      try {
        var pageFound = page.findOne ? page.findOne(function(n) { return n.name === name; }) : null;
        if (pageFound) return cacheNode(pageFound);
      } catch (e) {}
    }
  }

  return null;
}

async function resolveNode(params) {
  if (!params) return null;
  var id   = params.id || params.nodeId || params.node_id || params.targetId || params.target_id;
  var name = params.name || params.nodeName || params.node_name;
  var node = null;
  if (id)   node = await findNodeByIdAsync(id);
  if (!node && name) node = findNodeByName(name);
  return node;
}

// Component properties live on the COMPONENT_SET for variants; Figma throws when
// componentPropertyDefinitions is read on a variant, so hop to the set first.
function propertyOwner(node) {
  return node && node.type === "COMPONENT" && node.parent && node.parent.type === "COMPONENT_SET" ? node.parent : node;
}

function propertyDefinitions(node) {
  var owner = propertyOwner(node);
  try { return (owner && owner.componentPropertyDefinitions) || {}; } catch (e) { return {}; }
}

// Resolve a bare property name ("Label") to Figma's qualified key ("Label#132:39").
function resolvePropertyKey(defs, propertyName) {
  if (!defs) return null;
  if (defs[propertyName]) return propertyName;
  var keys = Object.keys(defs);
  for (var i = 0; i < keys.length; i++) {
    if (keys[i].split("#")[0] === propertyName) return keys[i];
  }
  return null;
}


// The one node shape every write op and query returns.
var NODE_KEY_DATA = "mcpKey";
function nodeToInfo(node) {
  if (!node) return null;
  const info = {
    id:       node.id,
    name:     node.name,
    type:     node.type,
    parentId: node.parent ? node.parent.id : null,
  };
  if ("x" in node)      info.x = Math.round(node.x);
  if ("y" in node)      info.y = Math.round(node.y);
  if ("width" in node)  info.width  = Math.round(node.width);
  if ("height" in node) info.height = Math.round(node.height);
  if (node.children) info.childIds = node.children.map(function(c) { return c.id; });
  if (node.layoutPositioning === "ABSOLUTE") info.layoutPositioning = "ABSOLUTE";
  try { var key = node.getPluginData && node.getPluginData(NODE_KEY_DATA); if (key) info.key = key; } catch (e) {}
  return info;
}

// ── Strict parameters ─────────────────────────────────────────────────────────
// Legacy layout keys get a migration hint instead of the generic message.
var LEGACY_KEY_HINTS = {
  layoutAlign: "layoutAlign was removed: use layoutSizingHorizontal/Vertical: \"FILL\" (stretch) on the child",
  layoutGrow: "layoutGrow was removed: use layoutSizingHorizontal (row) or layoutSizingVertical (column): \"FILL\"",
  primaryAxisSizingMode: "primaryAxisSizingMode was removed: use layoutSizingHorizontal/Vertical: \"HUG\" or \"FIXED\" on the frame",
  counterAxisSizingMode: "counterAxisSizingMode was removed: use layoutSizingHorizontal/Vertical: \"HUG\" or \"FIXED\" on the frame",
};

// Throws before anything is applied, so a call either does everything it was
// asked or nothing. Keys starting with "_" are internal (e.g. _taskId).
function assertKnownKeys(op, params, allowed) {
  var bad = Object.keys(params || {}).filter(function(k) { return !allowed.has(k) && k.charAt(0) !== "_"; });
  if (!bad.length) return;
  var hints = bad.map(function(k) { return LEGACY_KEY_HINTS[k]; }).filter(Boolean);
  throw new Error(op + ": unsupported " + (bad.length > 1 ? "keys " : "key ") + bad.join(", ") + "." +
    (hints.length ? " " + hints.join(". ") + "." : "") + " See figma_docs { section: \"api\" }.");
}

// ── Created-node journal ──────────────────────────────────────────────────────
// Nodes created since the last commit_undo, per task, so a failed figma_write
// or batch can remove what it drew. Edits to existing nodes are not journaled:
// Figma gives plugins no way to undo them.
var createdJournal = new Map();
function journalFor(params) {
  var key = (params && params._taskId) || "";
  if (!createdJournal.has(key)) createdJournal.set(key, []);
  return createdJournal.get(key);
}
function journalCreated(params, node) { journalFor(params).push(node); }
function rollbackJournal(params, from) {
  var list = journalFor(params), removed = 0;
  for (var i = list.length - 1; i >= (from || 0); i--) {
    var n = list[i];
    try { if (n && !n.removed) { n.remove(); removed++; } } catch (e) {}
  }
  list.length = from || 0;
  return removed;
}

// Yield execution back to Figma main thread loop so UI never freezes/blocks
function yieldToUI(delayMs) {
  return new Promise(function(resolve) {
    setTimeout(resolve, delayMs !== undefined ? delayMs : 0);
  });
}

// Scope this flag to synchronous traversal; never leave it changed across await.
function withInstanceVisibility(includeHidden, read) {
  var previous = figma.skipInvisibleInstanceChildren;
  figma.skipInvisibleInstanceChildren = !includeHidden;
  try { return read(); } finally { figma.skipInvisibleInstanceChildren = previous; }
}

// Depth-first walk with an explicit stack that yields to the UI every ~8ms
// (same slicing as read_nodes). Instance visibility is re-applied per slice.
// visit(node, ctx) returns "skip" (no children), "stop" (end walk), or a new
// ctx for the node's children (undefined keeps the current ctx).
async function walkTreeSliced(roots, includeHidden, visit, ctx) {
  var stack = [{ children: roots, next: 0, ctx: ctx }], stopped = false;
  while (stack.length && !stopped) {
    var started = Date.now();
    withInstanceVisibility(includeHidden, function() {
      while (stack.length && Date.now() - started < 8) {
        var cursor = stack[stack.length - 1];
        if (cursor.next >= cursor.children.length) { stack.pop(); continue; }
        var node = cursor.children[cursor.next++];
        if (!node || typeof node !== "object" || node.removed || (!includeHidden && node.visible === false)) continue;
        var r = visit(node, cursor.ctx);
        if (r === "stop") { stopped = true; return; }
        if (r === "skip" || !("children" in node)) continue;
        var children = node.children;
        if (children && children.length) stack.push({ children: children, next: 0, ctx: r === undefined ? cursor.ctx : r });
      }
    });
    if (stack.length && !stopped) await yieldToUI(0);
  }
}

// Flexible operation handler resolver with camelCase/snake_case and alias support
function resolveOperationHandler(operation) {
  if (!operation || typeof operation !== "string") return null;
  if (typeof handlers !== "object" || !handlers) return null;
  if (handlers[operation]) return handlers[operation];

  // 1. Convert snake_case -> camelCase (e.g. create_variable -> createVariable, list_pages -> listPages)
  var camel = operation.replace(/_([a-z0-9])/gi, function(_, c) { return c.toUpperCase(); });
  if (handlers[camel]) return handlers[camel];

  // 2. Convert camelCase -> snake_case (e.g. getPageNodes -> get_page_nodes, getNodeDetail -> get_node_detail)
  var snake = operation.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
  if (handlers[snake]) return handlers[snake];

  // 3. Common semantic synonyms & aliases
  var normalized = operation.toLowerCase().replace(/[-_\s]+/g, "");
  var aliasMap = {
    "getnode": handlers.get_node_detail,
    "getnodedetail": handlers.get_node_detail,
    "getnodeinfo": handlers.get_node_detail,
    "nodedetail": handlers.get_node_detail,
    "nodeinfo": handlers.get_node_detail,
    "inspectnode": handlers.get_design_context,
    "inspect": handlers.get_design_context,
    "getdesigncontext": handlers.get_design_context,
    "designcontext": handlers.get_design_context,
    "getdesign": handlers.get_design,
    "getselection": handlers.get_selection,
    "selection": handlers.get_selection,
    "getpagenodes": handlers.get_page_nodes,
    "pagenodes": handlers.get_page_nodes,
    "getstyles": handlers.get_styles,
    "styles": handlers.get_styles,
    "getvariables": handlers.get_variables,
    "gettokens": handlers.get_variables,
    "tokens": handlers.get_variables,
    "variables": handlers.get_variables,
    "getvariabletokens": handlers.get_variables,
    "getlocalcomponents": handlers.get_local_components,
    "localcomponents": handlers.get_local_components,
    "listcomponents": handlers.listComponents,
    "components": handlers.get_local_components,
    "getcomponentmap": handlers.get_component_map,
    "getunmappedcomponents": handlers.get_unmapped_components,
    "scandesign": handlers.scan_design,
    "searchnodes": handlers.search_nodes,
    "getviewport": handlers.get_viewport,
    "setviewport": handlers.set_viewport,
    "exportsvg": handlers.export_svg,
    "exportimage": handlers.export_image,
    "exportassets": handlers.export_assets,
    "listpages": handlers.listPages,
    "setpage": handlers.setPage,
    "createpage": handlers.createPage,
    "loadallpages": handlers.loadAllPagesAsync,
    "loadallpagesasync": handlers.loadAllPagesAsync,
    "update": handlers.modify,
    "remove": handlers["delete"]
  };

  if (aliasMap[normalized]) return aliasMap[normalized];

  return null;
}
