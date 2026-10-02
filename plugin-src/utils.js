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
  // Node consumers have inherited modes across collections. Let Figma resolve
  // the value; manual mode-id reuse across an alias chain is not valid.
  var consumerValue;
  if (contextNodeOrModeMap && contextNodeOrModeMap.resolvedVariableModes && typeof variable.resolveForConsumer === "function") {
    consumerValue = variable.resolveForConsumer(contextNodeOrModeMap).value;
  }

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
    var collection = await figma.variables.getVariableCollectionByIdAsync(colId);
    modeId = collection ? collection.defaultModeId : (availableModes.length === 1 ? availableModes[0] : null);
  }
  if (!modeId) return null;

  var rawVal = consumerValue !== undefined ? consumerValue : variable.valuesByMode[modeId];
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

function firstSolidHex(paints) {
  if (!paints || isMixed(paints) || !paints.length) return null;
  for (var i = 0; i < paints.length; i++) {
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

function normalizeNodeId(id) {
  if (!id || typeof id !== "string") return id;
  var clean = id.trim().replace(/^['"]|['"]$/g, "");
  try { clean = decodeURIComponent(clean); } catch(e) {}
  // Convert URL hyphenated IDs (e.g. 2715-40862) to colon notation (2715:40862)
  clean = clean.replace(/(\d+)-(\d+)/g, "$1:$2");
  return clean;
}

var allPagesLoaded = false;
async function ensureAllPagesLoaded() {
  if (allPagesLoaded) return;
  if (typeof figma.loadAllPagesAsync === "function") {
    try {
      await figma.loadAllPagesAsync();
      allPagesLoaded = true;
    } catch(e) {}
  }
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

  // 4. Fallback: If not found and pages are lazily loaded, load all pages and retry
  if (!allPagesLoaded && typeof figma.loadAllPagesAsync === "function") {
    await ensureAllPagesLoaded();
    try {
      var retryNode = await figma.getNodeByIdAsync(cleanId);
      if (!retryNode && cleanId !== id) retryNode = await figma.getNodeByIdAsync(id);
      if (retryNode && !retryNode.removed) return cacheNode(retryNode);
    } catch(e) {}
  }

  // 5. Fallback for instance sub-layer IDs (e.g. "I2715:40862;123:456")
  if (cleanId.indexOf(";") !== -1) {
    var parts = cleanId.split(";");
    for (var p = 0; p < parts.length; p++) {
      var partId = parts[p].replace(/^I/, "");
      try {
        var subNode = await figma.getNodeByIdAsync(partId);
        if (subNode && !subNode.removed) return cacheNode(subNode);
      } catch(e) {}
    }
  }

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
      var pageFound = page.findOne ? page.findOne(function(n) { return n.name === name; }) : null;
      if (pageFound) return cacheNode(pageFound);
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
