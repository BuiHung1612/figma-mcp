// ─── READ HANDLERS ────────────────────────────────────────────────────────────

// Node budget for the tree walkers: maxDepth alone doesn't stop a wide frame
// from producing tens of MB, which then hits the bridge op timeout and floods
// the model's context. Callers can raise/lower it with `maxNodes`, and
// `absolute: true` forces absoluteBoundingBox on every node.
var DEFAULT_NODE_BUDGET = 3000;

function makeWalkState(params) {
  var p = params || {};
  var budget = (p.maxNodes !== undefined && Number(p.maxNodes) > 0) ? Number(p.maxNodes) : DEFAULT_NODE_BUDGET;
  return { remaining: budget, budget: budget, truncated: false, absolute: p.absolute === true };
}

function walkStateMeta(walkState) {
  if (!walkState) return {};
  var meta = { nodeCount: walkState.budget - walkState.remaining, nodeBudget: walkState.budget };
  if (walkState.truncated) {
    meta.nodesTruncated = true;
    meta.truncatedHint = "Node budget reached — subtrees past it are summarized. Re-read a specific child id, or raise maxNodes.";
  }
  return meta;
}

// get_selection — returns full design data for current selection (or specified node)
handlers.get_selection = async function(params) {
  var id = params ? params.id : null;
  var nodeName = params ? params.name : null;
  var nodes;
  if (id) {
    nodes = [await findNodeByIdAsync(id)].filter(Boolean);
  } else if (nodeName) {
    nodes = [findNodeByName(nodeName)].filter(Boolean);
  } else {
    nodes = [].concat(figma.currentPage.selection);
  }

  if (!nodes.length) return { nodes: [], message: "Nothing selected" };

  var maxDepth = (params && params.depth !== undefined) ? (params.depth === "full" ? 50 : Number(params.depth)) : 15;
  var detailLevel = (params && params.detail) || "full";
  var filterInvisible = !(params && params.includeHidden === true);
  var tokenCollector = (detailLevel !== "minimal") ? { colors: new Set(), fonts: new Set(), sizes: new Set() } : null;
  var instanceCollector = (detailLevel !== "minimal") ? [] : null;
  var walkState = makeWalkState(params);
  var trees = nodes.map(function(n) { return extractDesignTree(n, 0, maxDepth, detailLevel, filterInvisible, tokenCollector, instanceCollector, walkState); });
  // mainComponent is async-only under documentAccess: dynamic-page — resolve
  // every collected INSTANCE after the (synchronous) tree walk.
  var instanceInfo = await resolveInstanceComponents(instanceCollector);
  return {
    nodes: trees,
    meta: walkStateMeta(walkState),
    instances: instanceInfo.truncated
      ? { resolved: instanceInfo.resolved, truncated: true, hint: "Too many instances to resolve — narrow the node or lower depth for full component references." }
      : undefined,
    tokens: tokenCollector ? {
      colors: Array.from(tokenCollector.colors),
      fonts: Array.from(tokenCollector.fonts),
      sizes: Array.from(tokenCollector.sizes).sort(function(a, b) { return a - b; }),
    } : null,
  };
};

// get_design — full node tree with configurable depth
// depth: number (default 10) or "full" for unlimited
handlers.get_design = async function(params) {
  var p = params || {};
  var id = p.id, name = p.name;
  var depthParam = p.depth !== undefined ? p.depth : 10;
  var detailLevel = p.detail || "full"; // "minimal" | "compact" | "full"
  var filterInvisible = !(p.includeHidden === true);

  var root;
  if (id)   root = await findNodeByIdAsync(id);
  else if (name) root = findNodeByName(name);
  else      root = figma.currentPage;

  if (!root) throw new Error("Node not found: id=" + (id || "none") + " name=" + (name || "none"));

  var maxDepth = (depthParam === "full") ? 50 : Number(depthParam);
  if (isNaN(maxDepth) || maxDepth < 1) maxDepth = 10;

  try {
    var tokenCollector = (detailLevel !== "minimal") ? { colors: new Set(), fonts: new Set(), sizes: new Set() } : null;
    var instanceCollector = (detailLevel !== "minimal") ? [] : null;
    var walkState = makeWalkState(p);
    var tree = extractDesignTree(root, 0, maxDepth, detailLevel, filterInvisible, tokenCollector, instanceCollector, walkState);
    var instanceInfo = await resolveInstanceComponents(instanceCollector);

    // SVG inlining: opt-in via inlineIcons/inlineSvg to avoid multi-second freezes on large designs
    var inlineIcons = (p.inlineIcons === true || p.inlineSvg === true);
    var iconCount = 0;
    var iconNodesFound = 0;
    if (inlineIcons && detailLevel === "full" && tree) {
      var iconNodes = [];
      var INLINE_ICON_CAP = 10;
      function collectIcons(node) {
        if (!node) return;
        if (node.isIcon && node.id) {
          iconNodesFound++;
          if (iconNodes.length < INLINE_ICON_CAP) iconNodes.push(node);
        }
        if (node.children) {
          for (var i = 0; i < node.children.length; i++) collectIcons(node.children[i]);
        }
      }
      collectIcons(tree);

      if (iconNodes.length > 0) {
        var exportPromises = iconNodes.map(async function(node) {
          try {
            var figNode = await findNodeByIdAsync(node.id);
            if (figNode) {
              var svg = await exportNodeSvg(figNode);
              if (svg && svg.length < 5000) {
                node.svgMarkup = svg;
                delete node.iconHint;
                iconCount++;
              }
            }
          } catch(e) {}
        });
        await Promise.all(exportPromises);
      }
    }

    var tokens = tokenCollector ? {
      colors: Array.from(tokenCollector.colors),
      fonts: Array.from(tokenCollector.fonts),
      sizes: Array.from(tokenCollector.sizes).sort(function(a, b) { return a - b; }),
    } : undefined;

    var meta = { maxDepth: maxDepth, detail: detailLevel, nodeType: root.type };
    Object.assign(meta, walkStateMeta(walkState));
    if (inlineIcons && detailLevel === "full") {
      meta.inlinedIcons = iconCount;
      if (iconNodesFound > iconCount) {
        meta.inlineIconsTruncated = true;
        meta.iconsFound = iconNodesFound;
      }
    }
    if (instanceInfo.resolved) meta.resolvedInstances = instanceInfo.resolved;
    if (instanceInfo.truncated) meta.instancesTruncated = true;
    return { tree: tree, tokens: tokens, meta: meta };
  } catch(e) {
    throw new Error("[get_design] " + e.message + " nodeType=" + root.type + " id=" + root.id);
  }
};

// scan_design — progressive scan for large/complex designs
// Returns a structured summary: sections, all text content, all colors, component list, image nodes
// Works on any size file without token overflow
handlers.scan_design = async function(params) {
  var p = params || {};
  var id = p.id, name = p.name;
  var root;
  if (id) root = await findNodeByIdAsync(id);
  else if (name) root = findNodeByName(name);
  else root = figma.currentPage;
  if (!root) throw new Error("Node not found");

  // Output caps — every capped list reports its real total plus a truncated
  // flag, so a caller can tell "that's all of it" from "that's the first N".
  var SCAN_LIMITS = { text: 500, images: 50, icons: 50, components: 50, colors: 30, fonts: 30 };
  var maxNodes = (p.maxNodes !== undefined && Number(p.maxNodes) > 0) ? Number(p.maxNodes) : 50000;

  var summary = {
    rootId: root.id,
    rootName: root.name,
    rootType: root.type,
    width: Math.round(root.width),
    height: Math.round(root.height),
    totalNodes: 0,
    sections: [],      // top-level children with their text content
    allText: [],       // every text node: id, content, font, color, position
    allColors: {},     // color → count (usage frequency)
    allFonts: {},      // "Inter/Bold/16px" → count
    images: [],        // nodes with image fills
    icons: [],         // likely icon nodes
    components: [],    // component instances with names
  };

  // Real counts across the whole subtree, independent of the output caps above.
  var totals = { textNodes: 0, imageNodes: 0, iconNodes: 0, instances: 0 };
  var scanIncludeHidden = !!(p.includeHidden);
  var instanceEntries = [];
  var nodeBudgetHit = false;

  // section = the top-level child whose subtree we are inside (null at the root)
  function walkCount(node, section) {
    if (!node || typeof node !== "object") return;
    if (!scanIncludeHidden && node.visible === false) return;
    if (summary.totalNodes >= maxNodes) { nodeBudgetHit = true; return; }
    summary.totalNodes++;

    // Collect text
    if (node.type === "TEXT") {
      totals.textNodes++;
      var textInfo = {
        id: node.id, name: node.name,
        x: Math.round(node.x), y: Math.round(node.y),
        width: Math.round(node.width), height: Math.round(node.height),
      };
      var textStyle = resolveTextStyle(node, { segments: false });
      if (textStyle) {
        textInfo.content = textStyle.content;
        textInfo.fill = textStyle.fill || null;
        textInfo.fontSize = textStyle.fontSize !== undefined ? textStyle.fontSize : null;
        textInfo.fontFamily = textStyle.fontFamily || null;
        textInfo.fontWeight = textStyle.fontWeight || null;
        if (textStyle.mixed) textInfo.mixedStyles = true;
      }
      if (summary.allText.length < SCAN_LIMITS.text) summary.allText.push(textInfo);

      // Section text preview — collected here instead of re-walking the subtree
      if (section && textInfo.content && textInfo.content.trim()) {
        if (!section.textContent) section.textContent = [];
        if (section.textContent.length < 20) section.textContent.push(textInfo.content.trim().substring(0, 60));
        else section.textContentTruncated = true;
      }

      // Count font usage
      if (textInfo.fontFamily) {
        var fontKey = textInfo.fontFamily + "/" + (textInfo.fontWeight || "Regular") + "/" + (textInfo.fontSize || "?") + "px";
        summary.allFonts[fontKey] = (summary.allFonts[fontKey] || 0) + 1;
      }
    }

    // Collect colors
    try {
      var hex = getFillHex(node);
      if (hex) summary.allColors[hex] = (summary.allColors[hex] || 0) + 1;
    } catch(e) {}
    try {
      var strokeHex = getStrokeHex(node);
      if (strokeHex) summary.allColors[strokeHex] = (summary.allColors[strokeHex] || 0) + 1;
    } catch(e) {}

    // Collect images
    if (hasImageFill(node)) {
      totals.imageNodes++;
      if (section) section.imageCount++;
      if (summary.images.length < SCAN_LIMITS.images) {
        summary.images.push({
          id: node.id, name: node.name,
          x: Math.round(node.x), y: Math.round(node.y),
          width: Math.round(node.width), height: Math.round(node.height),
        });
      }
    }

    // Collect icons
    if (isLikelyIcon(node)) {
      totals.iconNodes++;
      if (section) section.iconCount++;
      if (summary.icons.length < SCAN_LIMITS.icons) {
        summary.icons.push({ id: node.id, name: node.name, width: Math.round(node.width), height: Math.round(node.height) });
      }
    }

    // Collect component instances — mainComponent resolved after the walk
    // (async-only under documentAccess: dynamic-page).
    if (node.type === "INSTANCE") {
      totals.instances++;
      if (summary.components.length < SCAN_LIMITS.components) {
        var compEntry = {
          id: node.id, name: node.name,
          componentName: null, componentId: null,
          width: Math.round(node.width), height: Math.round(node.height),
        };
        summary.components.push(compEntry);
        instanceEntries.push({ info: compEntry, node: node });
      }
    }

    // Recurse
    if ("children" in node && Array.isArray(node.children)) {
      for (var i = 0; i < node.children.length; i++) walkCount(node.children[i], section);
    }
  }

  // Build the sections up front so the walk can attribute icons/images/text to
  // the top-level child they actually live under.
  if ("children" in root && Array.isArray(root.children)) {
    summary.totalNodes++;  // the root itself
    for (var ci = 0; ci < root.children.length; ci++) {
      var child = root.children[ci];
      if (!scanIncludeHidden && child.visible === false) continue;
      var section = {
        id: child.id, name: child.name, type: child.type,
        x: Math.round(child.x), y: Math.round(child.y),
        width: Math.round(child.width), height: Math.round(child.height),
        childCount: ("children" in child && Array.isArray(child.children)) ? child.children.length : 0,
        iconCount: 0,
        imageCount: 0,
      };
      summary.sections.push(section);
      walkCount(child, section);
    }
  } else {
    walkCount(root, null);
  }

  await resolveInstanceComponents(instanceEntries);

  // Sort colors by usage
  var colorEntries = Object.keys(summary.allColors).map(function(k) { return { color: k, count: summary.allColors[k] }; });
  colorEntries.sort(function(a, b) { return b.count - a.count; });
  summary.allColors = colorEntries.slice(0, SCAN_LIMITS.colors);

  // Sort fonts by usage, cap at 30 (same as allColors)
  var fontEntries = Object.keys(summary.allFonts).map(function(k) { return { font: k, count: summary.allFonts[k] }; });
  fontEntries.sort(function(a, b) { return b.count - a.count; });
  summary.allFonts = fontEntries.slice(0, SCAN_LIMITS.fonts);

  summary.totals = {
    textNodes: totals.textNodes,
    imageNodes: totals.imageNodes,
    iconNodes: totals.iconNodes,
    instances: totals.instances,
    uniqueColors: colorEntries.length,
    uniqueFonts: fontEntries.length,
  };

  var truncated = {};
  if (totals.textNodes > summary.allText.length)    truncated.allText = true;
  if (totals.imageNodes > summary.images.length)    truncated.images = true;
  if (totals.iconNodes > summary.icons.length)      truncated.icons = true;
  if (totals.instances > summary.components.length) truncated.components = true;
  if (colorEntries.length > summary.allColors.length) truncated.allColors = true;
  if (fontEntries.length > summary.allFonts.length)  truncated.allFonts = true;
  if (nodeBudgetHit) truncated.nodes = true;
  if (Object.keys(truncated).length) {
    summary.truncated = truncated;
    summary.truncatedHint = "Lists above are capped — compare with `totals` and re-scan a specific section id for the rest.";
  }

  return summary;
};

// search_nodes — find nodes by properties (color, type, font, name pattern)
handlers.search_nodes = async function(params) {
  var p = params || {};
  var results = [];
  var maxResults = p.limit || 50;

  // Search criteria
  var criteria = {
    type: p.type || null,               // "TEXT", "FRAME", "RECTANGLE", etc.
    namePattern: p.namePattern || null,  // wildcard pattern: "*header*"
    fill: p.fill || null,               // hex color: "#FF0000"
    fontFamily: p.fontFamily || null,    // "Inter"
    fontWeight: p.fontWeight || null,    // "Bold"
    fontSize: p.fontSize || null,        // 14
    text: p.text || null,               // exact or partial TEXT content match
    hasImage: p.hasImage || false,       // true = nodes with image fills
    hasIcon: p.hasIcon || false,         // true = likely icon nodes
    includeHidden: p.includeHidden || false, // false = skip visible:false nodes (default)
    minWidth: p.minWidth || null,
    maxWidth: p.maxWidth || null,
    minHeight: p.minHeight || null,
    maxHeight: p.maxHeight || null,
  };

  // Convert wildcard pattern to simple matcher
  function matchName(name, pattern) {
    if (!pattern) return true;
    var parts = pattern.toLowerCase().split("*");
    var str = name.toLowerCase();
    var pos = 0;
    for (var i = 0; i < parts.length; i++) {
      if (parts[i] === "") continue;
      var idx = str.indexOf(parts[i], pos);
      if (idx === -1) return false;
      pos = idx + parts[i].length;
    }
    return true;
  }

  function matchNode(node) {
    if (criteria.type && node.type !== criteria.type) return false;
    if (criteria.namePattern && !matchName(node.name, criteria.namePattern)) return false;
    if (criteria.fill) {
      var nodeFill = getFillHex(node);
      if (!nodeFill || nodeFill.toLowerCase() !== criteria.fill.toLowerCase()) return false;
    }
    // BUG-16: text filter only applies to TEXT nodes; non-TEXT nodes are excluded when text is set
    if (criteria.text && node.type !== "TEXT") return false;
    if (node.type === "TEXT") {
      try {
        if (criteria.text && node.characters.indexOf(criteria.text) === -1) return false;
        if (criteria.fontFamily && node.fontName && node.fontName.family !== criteria.fontFamily) return false;
        if (criteria.fontWeight && node.fontName && node.fontName.style !== criteria.fontWeight) return false;
        if (criteria.fontSize && node.fontSize !== criteria.fontSize) return false;
      } catch(e) { /* mixed styles, skip font filter */ }
    } else {
      if (criteria.fontFamily || criteria.fontWeight || criteria.fontSize) return false;
    }
    if (criteria.hasImage && !hasImageFill(node)) return false;
    if (criteria.hasIcon && !isLikelyIcon(node)) return false;
    if (criteria.minWidth && node.width < criteria.minWidth) return false;
    if (criteria.maxWidth && node.width > criteria.maxWidth) return false;
    if (criteria.minHeight && node.height < criteria.minHeight) return false;
    if (criteria.maxHeight && node.height > criteria.maxHeight) return false;
    return true;
  }

  function walkAndMatch(node) {
    // Guard: 'in' operator requires a non-null object — null/undefined/primitives crash here
    if (!node || typeof node !== "object") return;
    // Skip invisible nodes unless caller explicitly requests hidden elements
    if (!criteria.includeHidden && node.visible === false) return;
    if (results.length >= maxResults) return;
    try {
      if (matchNode(node)) {
        var info = {
          id: node.id, name: node.name, type: node.type,
          x: Math.round(node.x), y: Math.round(node.y),
          width: Math.round(node.width), height: Math.round(node.height),
        };
        try { info.fill = getFillHex(node); } catch(e) {}
        if (node.type === "TEXT") {
          try {
            info.content = node.characters;
            info.fontSize = node.fontSize;
            info.fontFamily = node.fontName ? node.fontName.family : null;
            info.fontWeight = node.fontName ? node.fontName.style : null;
          } catch(e) { try { info.content = node.characters; } catch(e2) {} }
        }
        // Find page path for context
        var path = [];
        var parent = node.parent;
        while (parent && parent.type !== "PAGE" && path.length < 5) {
          path.unshift(parent.name);
          parent = parent.parent;
        }
        if (path.length) info.path = path.join(" > ");
        results.push(info);
      }
    } catch(e) { /* skip inaccessible nodes */ }
    if (node && typeof node === "object" && "children" in node && Array.isArray(node.children)) {
      for (var i = 0; i < node.children.length; i++) {
        if (results.length >= maxResults) return;
        walkAndMatch(node.children[i]);
      }
    }
  }

  // Search scope: specific node or current page (no cross-page load — too slow on large files)
  var root;
  if (p.id) root = await findNodeByIdAsync(p.id);
  else if (p.name) root = findNodeByName(p.name);
  else root = figma.currentPage;

  walkAndMatch(root);

  return {
    results: results,
    total: results.length,
    criteria: criteria,
    truncated: results.length >= maxResults,
  };
};

// get_page_nodes — shallow list of top-level frames on current page
handlers.get_page_nodes = async () => {
  const page = figma.currentPage;
  return {
    page: page.name,
    nodes: page.children.map(function(n) {
      return Object.assign(nodeToInfo(n), { childCount: "children" in n ? n.children.length : 0 });
    }),
  };
};

// Exporting an unrendered node returns a blank image, so the renderer is
// nudged with scrollAndZoomIntoView — an unwanted canvas jump for what is a
// read operation, hence the save/restore pair around it.
function forceRenderNode(node, params) {
  var keep = !(params && params.keepViewport === false);
  var saved = null;
  try {
    if (keep) {
      saved = { center: { x: figma.viewport.center.x, y: figma.viewport.center.y }, zoom: figma.viewport.zoom };
    }
    figma.viewport.scrollAndZoomIntoView([node]);
  } catch(e) { /* non-fatal */ }
  return saved;
}

function restoreViewport(saved) {
  if (!saved) return;
  try {
    figma.viewport.center = saved.center;
    figma.viewport.zoom = saved.zoom;
  } catch(e) { /* non-fatal */ }
}

// Shared base64 encoder — Figma sandbox has no btoa/TextEncoder.
// A @2x PNG of a large frame runs to tens of MB, and appending one character at
// a time to a single string froze the plugin for seconds; encode into fixed
// chunks and join instead. figma.base64Encode (native) wins when present.
var _B64_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
var _B64_CHUNK_BYTES = 12288; // multiple of 3 — keeps chunks padding-free
function uint8ArrayToBase64(bytes) {
  var arr = (typeof Uint8Array !== "undefined" && !(bytes instanceof Uint8Array)) ? new Uint8Array(bytes) : bytes;
  try {
    if (typeof figma !== "undefined" && typeof figma.base64Encode === "function") {
      return figma.base64Encode(arr);
    }
  } catch(e) { /* fall through to the manual encoder */ }

  var len = arr.length;
  var parts = [];
  for (var start = 0; start < len; start += _B64_CHUNK_BYTES) {
    var stop = Math.min(start + _B64_CHUNK_BYTES, len);
    var chunk = [];
    for (var j = start; j < stop; j += 3) {
      var b0 = arr[j];
      var b1 = j + 1 < len ? arr[j + 1] : 0;
      var b2 = j + 2 < len ? arr[j + 2] : 0;
      chunk.push(
        _B64_CHARS[b0 >> 2],
        _B64_CHARS[((b0 & 3) << 4) | (b1 >> 4)],
        j + 1 < len ? _B64_CHARS[((b1 & 15) << 2) | (b2 >> 6)] : "=",
        j + 2 < len ? _B64_CHARS[b2 & 63] : "="
      );
    }
    parts.push(chunk.join(""));
  }
  return parts.join("");
}

// screenshot — export node as PNG base64 (v1.2.5)
handlers.screenshot = async function(params) {
  var id = params && params.id ? params.id : null;
  var nodeName = params && params.name ? params.name : null;
  var s = params && params.scale ? params.scale : 1;

  var page = figma.currentPage;
  var children = page.children;
  var node = null;
  var i;

  // Search by ID — direct fast lookup
  if (id) {
    node = await findNodeByIdAsync(id);
  }

  // Search by name — fast hierarchical lookup
  if (node === null && nodeName) {
    node = findNodeByName(nodeName);
  }
  // Fallback: any exportable top-level node (FRAME, COMPONENT, COMPONENT_SET, SECTION)
  if (node === null) {
    var exportableTypes = ["FRAME", "COMPONENT", "COMPONENT_SET", "SECTION", "INSTANCE", "GROUP"];
    for (i = 0; i < children.length; i++) {
      if (exportableTypes.indexOf(children[i].type) !== -1) { node = children[i]; break; }
    }
  }
  if (node === null) {
    return Promise.reject(new Error("[v1.2.5] No exportable node found. children=" + children.length));
  }

  // BUG-05 fix: nodes created in the current session may not have been rendered yet.
  // scrollAndZoomIntoView forces the Figma renderer to paint the node before exportAsync,
  // preventing blank/white PNG output on freshly-created nodes. It moves the
  // user's canvas though, so the previous viewport is restored afterwards
  // (pass keepViewport: false to leave the canvas on the exported node).
  var savedViewport = forceRenderNode(node, params);

  try {
    var bytes = await node.exportAsync({ format: "PNG", constraint: { type: "SCALE", value: s } });
    restoreViewport(savedViewport);
  } catch(exportErr) {
    restoreViewport(savedViewport);
    return Promise.reject(new Error("[v1.9.1-export] " + exportErr.message + " type=" + node.type + " id=" + node.id));
  }

  try {
    return { dataUrl: "data:image/png;base64," + uint8ArrayToBase64(bytes), nodeId: node.id, width: node.width, height: node.height };
  } catch(encodeErr) {
    return Promise.reject(new Error("[v1.9.1-encode] " + encodeErr.message));
  }
};

// Manual UTF-8 decode for Figma sandbox (no TextDecoder available).
// Decodes into batches of code units and flushes them through one
// String.fromCharCode call — per-character string concatenation made large SVG
// exports quadratic in practice.
var _UTF8_FLUSH_UNITS = 8192;
function uint8ArrayToString(arr) {
  var parts = [];
  var units = [];
  var i = 0;
  while (i < arr.length) {
    var byte1 = arr[i++];
    if (byte1 < 0x80) {
      units.push(byte1);
    } else if (byte1 < 0xE0) {
      var byte2 = arr[i++] & 0x3F;
      units.push(((byte1 & 0x1F) << 6) | byte2);
    } else if (byte1 < 0xF0) {
      var byte2b = arr[i++] & 0x3F;
      var byte3 = arr[i++] & 0x3F;
      units.push(((byte1 & 0x0F) << 12) | (byte2b << 6) | byte3);
    } else {
      var byte2c = arr[i++] & 0x3F;
      var byte3c = arr[i++] & 0x3F;
      var byte4 = arr[i++] & 0x3F;
      var codePoint = ((byte1 & 0x07) << 18) | (byte2c << 12) | (byte3c << 6) | byte4;
      codePoint -= 0x10000;
      units.push(0xD800 + (codePoint >> 10), 0xDC00 + (codePoint & 0x3FF));
    }
    if (units.length >= _UTF8_FLUSH_UNITS) {
      parts.push(String.fromCharCode.apply(String, units));
      units = [];
    }
  }
  if (units.length) parts.push(String.fromCharCode.apply(String, units));
  return parts.join("");
}

// Export node SVG — helper used by export_svg and inline icon extraction
async function exportNodeSvg(node) {
  var bytes = await node.exportAsync({ format: "SVG" });
  var arr = (typeof Uint8Array !== "undefined" && !(bytes instanceof Uint8Array)) ? new Uint8Array(bytes) : bytes;
  return uint8ArrayToString(arr);
}

// export_svg — export node as SVG string
handlers.export_svg = async function(params) {
  var id = params ? params.id : null;
  var nodeName = params ? params.name : null;
  var node = null;
  if (id) node = await findNodeByIdAsync(id);
  if (!node && nodeName) {
    node = findNodeByName(nodeName);
  }
  if (!node) node = figma.currentPage;
  if (!node) throw new Error("Node not found");
  var svg = await exportNodeSvg(node);
  return { svg: svg, nodeId: node.id, width: Math.round(node.width), height: Math.round(node.height) };
};

// export_image — export node as base64 PNG/JPG (for saving to disk, not for inline display)
handlers.export_image = async function(params) {
  var id = params ? params.id : null;
  var nodeName = params ? params.name : null;
  var format = (params && params.format) ? params.format.toUpperCase() : "PNG";
  var scale = (params && params.scale) ? params.scale : 2;

  if (format !== "PNG" && format !== "JPG") format = "PNG";

  var node = null;
  if (id) node = await findNodeByIdAsync(id);
  if (!node && nodeName) {
    node = findNodeByName(nodeName);
  }
  if (!node) throw new Error("Node not found for export");

  // BUG-05 fix: same as screenshot — force render before export, then put the
  // user's viewport back where it was.
  var savedViewport = forceRenderNode(node, params);
  var bytes;
  try {
    bytes = await node.exportAsync({ format: format, constraint: { type: "SCALE", value: scale } });
  } finally {
    restoreViewport(savedViewport);
  }
  var b64 = uint8ArrayToBase64(bytes);

  return {
    base64: b64,
    format: format.toLowerCase(),
    width: Math.round(node.width * scale),
    height: Math.round(node.height * scale),
    nodeId: node.id,
    nodeName: node.name,
    sizeBytes: bytes.length,
  };
};

// index_scan — aggregate page nodes, styles, variables, and components in one call for pre-indexing (concurrently)
handlers.index_scan = async function() {
  var pageNodesPromise = (async function() {
    try {
      if (handlers.get_page_nodes) {
        var res = await handlers.get_page_nodes({ depth: 6, detail: "compact", maxNodes: 15000 });
        return (res && res.nodes) ? res.nodes : (Array.isArray(res) ? res : []);
      }
    } catch (e) {}
    return [];
  })();

  var stylesPromise = (async function() {
    try {
      if (handlers.get_styles) return await handlers.get_styles({});
    } catch (e) {}
    return null;
  })();

  var varsPromise = (async function() {
    try {
      if (handlers.get_variables) return await handlers.get_variables({});
    } catch (e) {}
    return null;
  })();

  var compsPromise = (async function() {
    try {
      if (handlers.get_local_components) return await handlers.get_local_components({});
    } catch (e) {}
    return null;
  })();

  var results = await Promise.all([pageNodesPromise, stylesPromise, varsPromise, compsPromise]);
  var pageNodes = results[0];
  var styles = results[1];
  var variables = results[2];
  var components = results[3];

  return {
    fileName: figma.root ? figma.root.name : "unknown",
    sessionId: figma.root ? figma.root.id : "_default",
    pageNodes: pageNodes,
    styles: styles,
    variables: variables,
    components: components,
  };
};
