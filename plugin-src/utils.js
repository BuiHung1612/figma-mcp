// ─── UTILS ────────────────────────────────────────────────────────────────────

// Map common CSS color names to hex (AI sometimes passes color names instead of hex)
var CSS_COLOR_MAP = {
  "white": "#FFFFFF", "black": "#000000", "red": "#FF0000", "green": "#008000",
  "blue": "#0000FF", "yellow": "#FFFF00", "orange": "#FFA500", "purple": "#800080",
  "pink": "#FFC0CB", "gray": "#808080", "grey": "#808080", "transparent": "NONE",
  "teal": "#008080", "cyan": "#00FFFF", "magenta": "#FF00FF", "lime": "#00FF00",
  "navy": "#000080", "brown": "#A52A2A", "silver": "#C0C0C0", "gold": "#FFD700",
};

function normalizeHex(hex) {
  if (!hex) return null;
  var s = String(hex).trim();
  // CSS color name
  var mapped = CSS_COLOR_MAP[s.toLowerCase()];
  if (mapped) s = mapped;
  // Transparent / none
  if (s.toUpperCase() === "NONE" || s.toUpperCase() === "TRANSPARENT") return null;
  // rgba(r,g,b,a) or rgb(r,g,b) → convert to hex (alpha discarded here — use extractColorAlpha for alpha)
  var rgbaMatch = s.match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/);
  if (rgbaMatch) {
    var rr = Math.min(255, Math.max(0, parseInt(rgbaMatch[1])));
    var gg = Math.min(255, Math.max(0, parseInt(rgbaMatch[2])));
    var bb = Math.min(255, Math.max(0, parseInt(rgbaMatch[3])));
    s = "#" + ((1 << 24) + (rr << 16) + (gg << 8) + bb).toString(16).slice(1);
  }
  // Strip #
  s = s.replace(/^#/, "");
  // 8-char hex with alpha → take first 6 (alpha handled separately by extractColorAlpha)
  if (s.length === 8 && /^[0-9a-fA-F]{8}$/.test(s)) s = s.slice(0, 6);
  // 4-char hex shorthand with alpha → take first 3
  if (s.length === 4 && /^[0-9a-fA-F]{4}$/.test(s)) s = s.slice(0, 3);
  // Expand 3-char shorthand
  if (s.length === 3) s = s[0]+s[0]+s[1]+s[1]+s[2]+s[2];
  // Must be 6 hex chars now
  if (!/^[0-9a-fA-F]{6}$/.test(s)) {
    throw new Error("Invalid color value: \"" + hex + "\". Use 6-digit hex like #FF0000, 8-digit #RRGGBBAA, rgba(r,g,b,a), or a CSS name.");
  }
  return s;
}

// Extract alpha (0..1) from an 8-digit hex "#RRGGBBAA" or rgba(r,g,b,a) string.
// Returns null when the input has no alpha component.
function extractColorAlpha(hex) {
  if (!hex) return null;
  var s = String(hex).trim();
  // rgba(r,g,b,a) — capture 4th component
  var rgbaMatch = s.match(/^rgba\(\s*\d+\s*,\s*\d+\s*,\s*\d+\s*,\s*(\d*\.?\d+)\s*\)/i);
  if (rgbaMatch) return Math.min(1, Math.max(0, parseFloat(rgbaMatch[1])));
  // #RRGGBBAA → alpha = AA/255
  var cleaned = s.replace(/^#/, "");
  if (cleaned.length === 8 && /^[0-9a-fA-F]{8}$/.test(cleaned)) {
    return parseInt(cleaned.slice(6, 8), 16) / 255;
  }
  // #RGBA (4-char shorthand)
  if (cleaned.length === 4 && /^[0-9a-fA-F]{4}$/.test(cleaned)) {
    var a4 = cleaned[3];
    return parseInt(a4 + a4, 16) / 255;
  }
  return null;
}

function hexToRgb(hex) {
  var h = normalizeHex(hex);
  if (!h) return { r: 0, g: 0, b: 0 };
  return {
    r: parseInt(h.slice(0, 2), 16) / 255,
    g: parseInt(h.slice(2, 4), 16) / 255,
    b: parseInt(h.slice(4, 6), 16) / 255,
  };
}

function rgbToHex({ r, g, b }) {
  return "#" + [r, g, b]
    .map(v => Math.round(v * 255).toString(16).padStart(2, "0"))
    .join("");
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

async function getVariableSafeAsync(id) {
  if (!id) return null;
  if (variableCache.has(id)) return variableCache.get(id);
  try {
    if (figma.variables && typeof figma.variables.getVariableByIdAsync === "function") {
      var v = await figma.variables.getVariableByIdAsync(id);
      if (v) variableCache.set(id, v);
      return v;
    }
  } catch(e) {}
  return null;
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
    modeId = availableModes.length > 0 ? availableModes[0] : null;
  }
  if (!modeId) return null;

  var rawVal = variable.valuesByMode[modeId];
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
    var hex = rgbToHex(rawVal);
    var alpha = rawVal.a !== undefined ? Math.round(rawVal.a * 100) / 100 : 1;
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

function firstSolidHex(paints) {
  if (!paints || isMixed(paints) || !paints.length) return null;
  for (var i = 0; i < paints.length; i++) {
    if (paints[i].type === "SOLID" && paints[i].visible !== false) return rgbToHex(paints[i].color);
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

const FONT_STYLE_MAP = {
  Regular: "Regular", Medium: "Medium",
  SemiBold: "Semi Bold", Bold: "Bold", Light: "Light",
  Thin: "Thin", Heavy: "Heavy",
  // BUG-02 fix: map "Black" and aliases to nearest available Inter weight
  Black: "Bold", ExtraBold: "Extra Bold", UltraBold: "Extra Bold",
  "Extra Bold": "Extra Bold", "Ultra Bold": "Extra Bold",
  "Semi Bold": "Semi Bold",
  "Condensed Heavy": "Condensed Heavy",
  "Thin Italic": "Thin Italic",
  "Light Italic": "Light Italic",
};

// ── Font Loading Cache ────────────────────────────────────────────────────────
var loadedFonts = new Set();
var fontLoadingPromises = new Map();

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
      // If specific style fails, try Regular or fallback to Inter
      try {
        await figma.loadFontAsync({ family: family, style: "Regular" });
        loadedFonts.add(family + ":Regular");
      } catch (e2) {
        try {
          await figma.loadFontAsync({ family: "Inter", style: "Regular" });
          loadedFonts.add("Inter:Regular");
        } catch (e3) {}
      }
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

function findNodeById(id) {
  if (!id) return null;
  if (figma.currentPage.id === id) return figma.currentPage;
  if (figma.root.id === id) return figma.root;
  if (nodeCache.has(id)) {
    var cached = nodeCache.get(id);
    if (!cached.removed) return cached;
    nodeCache.delete(id);
  }
  try {
    if (typeof figma.getNodeById === "function") {
      var node = figma.getNodeById(id);
      if (node && !node.removed) return cacheNode(node);
    }
  } catch(e) {}
  // Check selection
  for (var i = 0; i < figma.currentPage.selection.length; i++) {
    if (figma.currentPage.selection[i].id === id) return cacheNode(figma.currentPage.selection[i]);
  }
  return null;
}

async function findNodeByIdAsync(id) {
  if (!id) return null;
  if (figma.currentPage.id === id) return figma.currentPage;
  if (figma.root.id === id) return figma.root;
  if (nodeCache.has(id)) {
    var cached = nodeCache.get(id);
    if (!cached.removed) return cached;
    nodeCache.delete(id);
  }
  // Try synchronous lookup first (0ms)
  try {
    if (typeof figma.getNodeById === "function") {
      var syncNode = figma.getNodeById(id);
      if (syncNode && !syncNode.removed) return cacheNode(syncNode);
    }
  } catch(e) {}
  // Fallback to async
  try {
    var node = await figma.getNodeByIdAsync(id);
    if (node && !node.removed) return cacheNode(node);
  } catch(e) {}
  return null;
}

function findNodeByName(name) {
  if (figma.currentPage.name === name) return figma.currentPage;
  // Search in selection first, then shallow children, then findOne as fallback
  for (var i = 0; i < figma.currentPage.selection.length; i++) {
    if (figma.currentPage.selection[i].name === name) return cacheNode(figma.currentPage.selection[i]);
  }
  for (var j = 0; j < figma.currentPage.children.length; j++) {
    if (figma.currentPage.children[j].name === name) return cacheNode(figma.currentPage.children[j]);
  }
  var found = figma.currentPage.findOne(n => n.name === name);
  if (found) cacheNode(found);
  return found;
}

async function resolveNode(params) {
  var id   = params.id || params.nodeId || params.targetId;
  var name = params.name || params.nodeName;
  var node = null;
  if (id)   node = await findNodeByIdAsync(id);
  if (!node && name) node = findNodeByName(name);
  return node;
}

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
  return info;
}

// Yield execution back to Figma main thread loop so UI never freezes/blocks
function yieldToUI(delayMs) {
  return new Promise(function(resolve) {
    setTimeout(resolve, delayMs !== undefined ? delayMs : 0);
  });
}
