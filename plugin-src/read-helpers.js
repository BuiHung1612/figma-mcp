// ─── READ HELPERS ─────────────────────────────────────────────────────────────

// Detect if a node is likely an icon (small vector/group/instance)
function isLikelyIcon(node) {
  if (!node || !("width" in node)) return false;
  var w = node.width, h = node.height;
  // Icons are typically small (8-64px) and roughly square
  if (w < 8 || w > 80 || h < 8 || h > 80) return false;
  var ratio = Math.max(w, h) / Math.min(w, h);
  if (ratio > 1.5) return false;
  var iconTypes = ["VECTOR", "BOOLEAN_OPERATION", "STAR", "POLYGON", "LINE"];
  if (iconTypes.indexOf(node.type) !== -1) return true;
  // Small instance or group with only vectors inside
  if (node.type === "INSTANCE" || node.type === "GROUP" || node.type === "FRAME") {
    if (!node.children || node.children.length === 0) return false;
    if (node.children.length > 10) return false;
    var allVectors = true;
    for (var i = 0; i < node.children.length; i++) {
      var ct = node.children[i].type;
      if (iconTypes.indexOf(ct) === -1 && ct !== "GROUP" && ct !== "FRAME" && ct !== "BOOLEAN_OPERATION") {
        allVectors = false; break;
      }
    }
    return allVectors;
  }
  return false;
}

// Check if node has image fill
function hasImageFill(node) {
  try {
    if (!node.fills || !node.fills.length) return false;
    for (var i = 0; i < node.fills.length; i++) {
      if (node.fills[i].type === "IMAGE" && node.fills[i].visible !== false) return true;
    }
  } catch(e) {}
  return false;
}

// Collect all text content from a subtree (for truncated nodes summary)
function collectTextContent(node, maxItems) {
  if (!maxItems) maxItems = 10;
  var texts = [];
  function walk(n) {
    if (!n || typeof n !== "object") return;
    if (texts.length >= maxItems) return;
    if (n.type === "TEXT") {
      var t = n.characters;
      if (t && t.trim()) texts.push(t.trim().substring(0, 60));
    }
    if ("children" in n && Array.isArray(n.children)) {
      for (var i = 0; i < n.children.length; i++) walk(n.children[i]);
    }
  }
  walk(node);
  return texts;
}

// Collect icon names from a subtree
function collectIconNames(node, maxItems) {
  if (!maxItems) maxItems = 10;
  var icons = [];
  function walk(n) {
    if (!n || typeof n !== "object") return;
    if (icons.length >= maxItems) return;
    if (isLikelyIcon(n)) icons.push(n.name);
    if ("children" in n && Array.isArray(n.children)) {
      for (var i = 0; i < n.children.length; i++) walk(n.children[i]);
    }
  }
  walk(node);
  return icons;
}

// Resolve the typography of a TEXT node into plain values, unwrapping
// figma.mixed via getStyledTextSegments. Returns null for non-TEXT nodes.
// { content, fontSize, fontFamily, fontWeight, fill, lineHeight (px|"NN%"),
//   letterSpacing, textDecoration, mixed, segments? }
function resolveTextStyle(node, opts) {
  if (!node || node.type !== "TEXT") return null;
  var withSegments = !opts || opts.segments !== false;
  var out = { mixed: false };

  try { out.content = node.characters; } catch(e) {}

  var mixed = isMixed(node.fontSize) || isMixed(node.fontName) ||
              isMixed(node.fills)    || isMixed(node.letterSpacing) ||
              isMixed(node.lineHeight) || isMixed(node.textDecoration);

  if (!mixed) {
    try {
      if (typeof node.fontSize === "number") out.fontSize = node.fontSize;
      if (node.fontName && node.fontName.family) out.fontFamily = node.fontName.family;
      if (node.fontName && node.fontName.style) out.fontWeight = node.fontName.style;
      var hex = getFillHex(node);
      if (hex) out.fill = hex;
      if (node.lineHeight && node.lineHeight.unit === "PERCENT") out.lineHeight = Math.round(node.lineHeight.value) + "%";
      else if (node.lineHeight && node.lineHeight.unit === "PIXELS") out.lineHeight = node.lineHeight.value;
      if (node.letterSpacing && node.letterSpacing.value) out.letterSpacing = node.letterSpacing.value;
      if (node.textDecoration && node.textDecoration !== "NONE") out.textDecoration = node.textDecoration;
    } catch(e) {}
    return out;
  }

  // Mixed styles — read the real per-run values instead of leaking the Symbol.
  out.mixed = true;
  var segs = null;
  try {
    if (typeof node.getStyledTextSegments === "function") {
      segs = node.getStyledTextSegments(["fontSize", "fontName", "fills", "letterSpacing", "lineHeight", "textDecoration"]);
    }
  } catch(e) {}

  if (segs && segs.length) {
    var mapped = segs.map(function(s) {
      var seg = { text: s.characters };
      if (typeof s.fontSize === "number") seg.fontSize = s.fontSize;
      if (s.fontName) {
        seg.fontFamily = s.fontName.family;
        if (s.fontName.style) seg.fontWeight = s.fontName.style;
      }
      var segHex = firstSolidHex(s.fills);
      if (segHex) seg.fill = segHex;
      if (s.lineHeight && s.lineHeight.unit === "PERCENT") seg.lineHeight = Math.round(s.lineHeight.value) + "%";
      else if (s.lineHeight && s.lineHeight.unit === "PIXELS") seg.lineHeight = s.lineHeight.value;
      if (s.letterSpacing && s.letterSpacing.value) seg.letterSpacing = s.letterSpacing.value;
      if (s.textDecoration && s.textDecoration !== "NONE") seg.textDecoration = s.textDecoration;
      return seg;
    });
    if (withSegments) out.segments = mapped;
    // Representative values = first segment, so consumers that only read the
    // flat fields still get real numbers rather than "mixed".
    var head = mapped[0];
    if (head.fontSize !== undefined)     out.fontSize = head.fontSize;
    if (head.fontFamily !== undefined)   out.fontFamily = head.fontFamily;
    if (head.fontWeight !== undefined)   out.fontWeight = head.fontWeight;
    if (head.fill !== undefined)         out.fill = head.fill;
    if (head.lineHeight !== undefined)   out.lineHeight = head.lineHeight;
    if (head.letterSpacing !== undefined) out.letterSpacing = head.letterSpacing;
    if (head.textDecoration !== undefined) out.textDecoration = head.textDecoration;
  } else {
    // No segment API — salvage whatever is not mixed.
    if (typeof node.fontSize === "number") out.fontSize = node.fontSize;
    if (!isMixed(node.fontName) && node.fontName) {
      out.fontFamily = node.fontName.family;
      out.fontWeight = node.fontName.style;
    }
    var fallbackHex = getFillHex(node);
    if (fallbackHex) out.fill = fallbackHex;
  }

  return out;
}

// Instance → main component resolution.
//
// `instance.mainComponent` throws under documentAccess: dynamic-page, so the
// synchronous tree walkers collect { info, node } pairs here and the async
// handler resolves them in batches afterwards. Batching (rather than one
// Promise.all over thousands of instances) keeps a big frame from flooding the
// scene-graph bridge in one go.
var MAIN_COMPONENT_BATCH = 50;
var MAX_RESOLVED_INSTANCES = 400;

function describeMainComponent(main) {
  var out = { componentId: main.id, componentName: main.name };
  try {
    if (main.parent && main.parent.type === "COMPONENT_SET") {
      var setName = main.parent.name;
      out.componentSetName = setName;
      out.variantLabel = main.name.indexOf(setName) === 0
        ? main.name.slice(setName.length).replace(/^[,\s/]+/, "")
        : main.name;
    }
  } catch(e) {}
  try { if (main.key) out.componentKey = main.key; } catch(e) {}
  return out;
}

// entries: [{ info, node }] — `info` is mutated in place with the component
// reference fields. Returns { resolved, truncated }.
async function resolveInstanceComponents(entries, apply) {
  if (!entries || !entries.length) return { resolved: 0, truncated: false };
  var truncated = entries.length > MAX_RESOLVED_INSTANCES;
  var pending = truncated ? entries.slice(0, MAX_RESOLVED_INSTANCES) : entries;
  var resolved = 0;

  for (var i = 0; i < pending.length; i += MAIN_COMPONENT_BATCH) {
    var batch = pending.slice(i, i + MAIN_COMPONENT_BATCH);
    var mains = await Promise.all(batch.map(function(entry) {
      return getMainComponentSafe(entry.node);
    }));
    for (var b = 0; b < batch.length; b++) {
      var main = mains[b];
      if (!main) continue;
      var desc = describeMainComponent(main);
      if (apply) apply(batch[b].info, desc);
      else Object.assign(batch[b].info, desc);
      resolved++;
    }
  }
  return { resolved: resolved, truncated: truncated };
}

function absoluteBox(node) {
  try {
    var box = node.absoluteBoundingBox;
    if (!box) return null;
    return {
      x: Math.round(box.x), y: Math.round(box.y),
      width: Math.round(box.width), height: Math.round(box.height),
    };
  } catch(e) { return null; }
}

// x/y are parent-relative and width/height ignore rotation, so they are not
// enough to place a node that sits under a GROUP (no coordinate system of its
// own) or under any rotation. In those cases emit the absolute box as well.
function needsAbsoluteBox(node) {
  try {
    if (node.rotation) return true;
    var parent = node.parent;
    if (!parent) return false;
    if (parent.type === "GROUP" || parent.type === "BOOLEAN_OPERATION") return true;
    if (parent.rotation) return true;
  } catch(e) {}
  return false;
}

// Collect one item past the cap so the caller can tell a full list from a
// truncated one. Returns { items, truncated }.
function collectCapped(collect, node, maxItems) {
  var items = collect(node, maxItems + 1);
  if (items.length > maxItems) return { items: items.slice(0, maxItems), truncated: true };
  return { items: items, truncated: false };
}

// strokeWeight is figma.mixed when the four sides differ — expand it to the
// per-side weights instead of emitting the Symbol.
function applyStrokeWeight(node, info) {
  try {
    if (!("strokeWeight" in node)) return;
    if (!isMixed(node.strokeWeight)) {
      if (node.strokeWeight) info.strokeWeight = node.strokeWeight;
      return;
    }
    var sides = {
      top:    node.strokeTopWeight,
      right:  node.strokeRightWeight,
      bottom: node.strokeBottomWeight,
      left:   node.strokeLeftWeight,
    };
    var out = {};
    var keys = Object.keys(sides);
    for (var i = 0; i < keys.length; i++) {
      if (typeof sides[keys[i]] === "number") out[keys[i]] = sides[keys[i]];
    }
    if (Object.keys(out).length) info.strokeWeight = out;
  } catch(e) {}
}

// Detail levels: "minimal" | "compact" | "full"
// minimal: id, name, type, position, size, childCount — ~5% token cost
// compact: + fill, stroke, cornerRadius, layout, text content — ~30% token cost
// full:    + effects, segments, gradient details, boundVariables, inline SVG — 100% token cost
// filterInvisible: true (default) = skip nodes with visible:false | false = include all nodes
// instanceCollector: optional array — INSTANCE nodes are pushed as { info, node }
//   for the caller to resolve via resolveInstanceComponents (mainComponent is
//   async-only under documentAccess: dynamic-page).
// walkState: optional { remaining, truncated, absolute } — `remaining` is a node
//   budget so one huge frame can't produce a multi-MB payload; subtrees past the
//   budget are summarized like the depth limit and `truncated` is set.
function extractDesignTree(node, depth, maxDepth, detailLevel, filterInvisible, tokenCollector, instanceCollector, walkState) {
  if (!node || typeof node !== "object") return null;
  if (depth === undefined) depth = 0;
  if (maxDepth === undefined) maxDepth = 15;
  if (!detailLevel) detailLevel = "full";
  if (filterInvisible === undefined) filterInvisible = true;
  if (depth > maxDepth) return null;

  // Skip invisible nodes when filtering is enabled (depth > 0 = non-root nodes)
  if (filterInvisible && depth > 0 && node.visible === false) return null;

  var isMinimal = (detailLevel === "minimal");
  var isCompact = (detailLevel === "compact");
  var isFull    = (detailLevel === "full");

  var info = {
    id:    node.id,
    name:  node.name,
    type:  node.type,
    x:     "x"      in node ? Math.round(node.x)      : undefined,
    y:     "y"      in node ? Math.round(node.y)       : undefined,
    width: "width"  in node ? Math.round(node.width)   : undefined,
    height:"height" in node ? Math.round(node.height)  : undefined,
  };

  if (tokenCollector && info.width) tokenCollector.sizes.add(info.width);
  if (tokenCollector && info.height) tokenCollector.sizes.add(info.height);

  if (walkState && typeof walkState.remaining === "number") walkState.remaining--;

  if (!isMinimal && ((walkState && walkState.absolute) || needsAbsoluteBox(node))) {
    var abox = absoluteBox(node);
    if (abox) info.absoluteBoundingBox = abox;
  }

  // Minimal: only basic info + childCount, skip all style properties
  if (isMinimal) {
    if ("children" in node && node.children.length) {
      info.childCount = node.children.length;
      if (node.type === "TEXT") { try { info.content = node.characters; } catch(e) {} }
      if (walkState && typeof walkState.remaining === "number" && walkState.remaining <= 0) {
        info.childrenTruncated = "nodeBudget";
        walkState.truncated = true;
      } else {
        info.children = node.children
          .map(function(c) { return extractDesignTree(c, depth + 1, maxDepth, detailLevel, filterInvisible, tokenCollector, instanceCollector, walkState); })
          .filter(Boolean);
      }
    }
    return info;
  }

  // ── Fill (multiple fills, gradients, images) ──
  try {
    if ("fills" in node && node.fills && !isMixed(node.fills) && node.fills.length) {
      var fills = node.fills;
      if (fills.length === 1 && fills[0].type === "SOLID") {
        info.fill = rgbToHex(fills[0].color);
        if (tokenCollector && info.fill) tokenCollector.colors.add(info.fill);
        if (fills[0].opacity !== undefined && fills[0].opacity !== 1) {
          info.fillOpacity = Math.round(fills[0].opacity * 100) / 100;
        }
      } else {
        info.fills = [];
        for (var fi = 0; fi < fills.length; fi++) {
          var f = fills[fi];
          var fd = { type: f.type, visible: f.visible !== false };
          if (f.type === "SOLID") {
            fd.color = rgbToHex(f.color);
            if (tokenCollector && fd.color) tokenCollector.colors.add(fd.color);
            if (f.opacity !== undefined && f.opacity !== 1) fd.opacity = Math.round(f.opacity * 100) / 100;
          } else if (f.type === "GRADIENT_LINEAR" || f.type === "GRADIENT_RADIAL" || f.type === "GRADIENT_ANGULAR") {
            fd.gradientStops = f.gradientStops ? f.gradientStops.map(function(gs) {
              var sc = rgbToHex(gs.color);
              if (tokenCollector && sc) tokenCollector.colors.add(sc);
              return { color: sc, position: Math.round(gs.position * 100) / 100 };
            }) : [];
            // Extract gradient angle from gradientTransform matrix
            try {
              if (f.gradientTransform && f.type === "GRADIENT_LINEAR") {
                var gt = f.gradientTransform;
                var angle = Math.round(Math.atan2(gt[1][0], gt[0][0]) * 180 / Math.PI);
                fd.gradientAngle = ((angle % 360) + 360) % 360;
              }
            } catch(e2) {}
          } else if (f.type === "IMAGE") {
            fd.scaleMode = f.scaleMode || "FILL";
            fd.imageHash = f.imageHash || null;
          }
          info.fills.push(fd);
        }
      }
    }
  } catch(e) { /* skip fills */ }

  // ── Stroke (all strokes, not just first solid) ──
  try {
    if ("strokes" in node && node.strokes && !isMixed(node.strokes) && node.strokes.length) {
      var strokes = node.strokes;
      if (strokes.length === 1 && strokes[0].type === "SOLID") {
        info.stroke = rgbToHex(strokes[0].color);
        if (tokenCollector && info.stroke) tokenCollector.colors.add(info.stroke);
        applyStrokeWeight(node, info);
        if (node.strokeAlign) info.strokeAlign = node.strokeAlign;
      } else {
        info.strokes = strokes.map(function(s) {
          var sd = { type: s.type };
          if (s.type === "SOLID") {
            sd.color = rgbToHex(s.color);
            if (tokenCollector && sd.color) tokenCollector.colors.add(sd.color);
          }
          if (s.opacity !== undefined && s.opacity !== 1) sd.opacity = Math.round(s.opacity * 100) / 100;
          return sd;
        });
        applyStrokeWeight(node, info);
        if (node.strokeAlign) info.strokeAlign = node.strokeAlign;
      }
    }
  } catch(e) { /* skip strokes */ }

  // ── Corner radius (per-corner support) ──
  try {
    if ("cornerRadius" in node && node.cornerRadius !== 0) {
      if (typeof node.cornerRadius === "number") {
        info.cornerRadius = node.cornerRadius;
      } else {
        var tl = node.topLeftRadius || 0, tr = node.topRightRadius || 0,
            br = node.bottomRightRadius || 0, bl = node.bottomLeftRadius || 0;
        if (tl === tr && tr === br && br === bl) {
          if (tl !== 0) info.cornerRadius = tl;
        } else {
          info.cornerRadius = { tl: tl, tr: tr, br: br, bl: bl };
        }
      }
    }
  } catch(e) {}

  // ── Rotation ──
  try { if ("rotation" in node && node.rotation !== 0) info.rotation = Math.round(node.rotation * 100) / 100; } catch(e) {}

  // ── Opacity, visibility, blend mode, clip ──
  try { if ("opacity" in node && node.opacity !== 1) info.opacity = Math.round(node.opacity * 100) / 100; } catch(e) {}
  try { if ("visible" in node && !node.visible) info.visible = false; } catch(e) {}
  try { if ("blendMode" in node && node.blendMode !== "NORMAL" && node.blendMode !== "PASS_THROUGH") info.blendMode = node.blendMode; } catch(e) {}
  try { if ("clipsContent" in node && node.clipsContent) info.clipsContent = true; } catch(e) {}

  // ── Bound Variables (Design Tokens) — full only ──
  if (isFull) try {
    if (node.boundVariables) {
      var bv = {};
      var bvKeys = Object.keys(node.boundVariables);
      for (var bvi = 0; bvi < bvKeys.length; bvi++) {
        var bvk = bvKeys[bvi];
        var binding = node.boundVariables[bvk];
        if (binding) {
          if (Array.isArray(binding)) {
            bv[bvk] = binding.map(function(b) { return b ? b.id : null; });
          } else {
            bv[bvk] = binding.id || null;
          }
        }
      }
      if (Object.keys(bv).length > 0) info.boundVariables = bv;
    }
  } catch(e) {}

  // ── Effects (shadows, blurs) — full only ──
  if (isFull) try {
    if ("effects" in node && node.effects && node.effects.length) {
      var effs = [];
      for (var ei = 0; ei < node.effects.length; ei++) {
        var eff = node.effects[ei];
        if (eff.visible === false) continue;
        var ed = { type: eff.type };
        if (eff.color) ed.color = rgbToHex(eff.color);
        if (eff.offset) ed.offset = { x: eff.offset.x, y: eff.offset.y };
        if (eff.radius !== undefined) ed.radius = eff.radius;
        if (eff.spread !== undefined) ed.spread = eff.spread;
        effs.push(ed);
      }
      if (effs.length) info.effects = effs;
    }
  } catch(e) {}

  // ── TEXT node — comprehensive extraction ──
  // Per-segment properties come back as figma.mixed (a Symbol) on multi-style
  // text, so everything typographic goes through resolveTextStyle.
  if (node.type === "TEXT") {
    // Segments are verbose — only worth their tokens at detail "full".
    var text = resolveTextStyle(node, { segments: isFull });
    if (text) {
      info.content = text.content;
      if (text.fill)          info.fill = text.fill;
      if (text.fontSize !== undefined)  info.fontSize = text.fontSize;
      if (text.fontFamily)    info.fontFamily = text.fontFamily;
      if (text.fontWeight && text.fontWeight !== "Regular") info.fontWeight = text.fontWeight;
      if (text.lineHeight !== undefined)    info.lineHeight = text.lineHeight;
      if (text.letterSpacing !== undefined) info.letterSpacing = text.letterSpacing;
      if (text.textDecoration)              info.textDecoration = text.textDecoration;
      if (text.mixed) {
        info.mixedStyles = true;
        if (text.segments) info.segments = text.segments;
      }
      if (tokenCollector && info.fontFamily && info.fontWeight) {
        tokenCollector.fonts.add(info.fontFamily + "/" + info.fontWeight + "/" + (info.fontSize || 14) + "px");
      }
    }
    // Node-level text properties — never mixed.
    try {
      if (node.textAlignHorizontal && node.textAlignHorizontal !== "LEFT") info.textAlign = node.textAlignHorizontal;
      if (node.textAlignVertical && node.textAlignVertical !== "TOP") info.textAlignVertical = node.textAlignVertical;
      if (node.textTruncation && node.textTruncation !== "DISABLED") info.textTruncation = node.textTruncation;
      if (node.textAutoResize && node.textAutoResize !== "NONE") info.textAutoResize = node.textAutoResize;
    } catch(e) {}
  }

  // ── Auto Layout (comprehensive & compact) ──
  try {
    if ("layoutMode" in node && node.layoutMode !== "NONE") {
      var pt = node.paddingTop || 0, pr = node.paddingRight || 0, pb = node.paddingBottom || 0, pl = node.paddingLeft || 0;
      var layoutObj = {
        mode: node.layoutMode,
      };
      if (node.itemSpacing !== undefined && node.itemSpacing !== 0) layoutObj.itemSpacing = node.itemSpacing;
      if (node.primaryAxisAlignItems && node.primaryAxisAlignItems !== "MIN") layoutObj.align = node.primaryAxisAlignItems;
      if (node.counterAxisAlignItems && node.counterAxisAlignItems !== "MIN") layoutObj.crossAlign = node.counterAxisAlignItems;

      if (pt === pr && pr === pb && pb === pl) {
        if (pt !== 0) layoutObj.padding = pt;
      } else if (pt === pb && pl === pr) {
        if (pl !== 0) layoutObj.paddingX = pl;
        if (pt !== 0) layoutObj.paddingY = pt;
      } else {
        if (pt !== 0) layoutObj.paddingTop = pt;
        if (pr !== 0) layoutObj.paddingRight = pr;
        if (pb !== 0) layoutObj.paddingBottom = pb;
        if (pl !== 0) layoutObj.paddingLeft = pl;
      }

      try { if (node.counterAxisSpacing !== undefined && node.counterAxisSpacing !== 0) layoutObj.counterAxisSpacing = node.counterAxisSpacing; } catch(e2) {}
      if (node.primaryAxisSizingMode && node.primaryAxisSizingMode !== "AUTO") layoutObj.primarySizing = node.primaryAxisSizingMode;
      if (node.counterAxisSizingMode && node.counterAxisSizingMode !== "AUTO") layoutObj.counterSizing = node.counterAxisSizingMode;
      if (node.layoutWrap && node.layoutWrap !== "NO_WRAP") layoutObj.wrap = node.layoutWrap;

      info.layout = layoutObj;
    }
  } catch(e) {}

  // ── Child layout properties ──
  try { if ("layoutAlign" in node && node.layoutAlign && node.layoutAlign !== "INHERIT") info.layoutAlign = node.layoutAlign; } catch(e) {}
  try { if ("layoutGrow" in node && node.layoutGrow !== 0) info.layoutGrow = node.layoutGrow; } catch(e) {}
  try { if ("layoutPositioning" in node && node.layoutPositioning === "ABSOLUTE") info.layoutPositioning = "ABSOLUTE"; } catch(e) {}

  // ── Constraints ──
  try {
    if ("constraints" in node && node.constraints) {
      var ch = node.constraints.horizontal, cv = node.constraints.vertical;
      if ((ch && ch !== "MIN") || (cv && cv !== "MIN")) {
        info.constraints = { horizontal: ch, vertical: cv };
      }
    }
  } catch(e) {}

  // ── Applied style references (Issue #3: expose textStyleId / fillStyleId) ──
  try { if (node.textStyleId && typeof node.textStyleId === "string") info.textStyleId = node.textStyleId; } catch(e) {}
  try { if (node.fillStyleId && typeof node.fillStyleId === "string") info.fillStyleId = node.fillStyleId; } catch(e) {}
  try { if (node.strokeStyleId && typeof node.strokeStyleId === "string") info.strokeStyleId = node.strokeStyleId; } catch(e) {}
  try { if (node.effectStyleId && typeof node.effectStyleId === "string") info.effectStyleId = node.effectStyleId; } catch(e) {}
  try { if (node.gridStyleId && typeof node.gridStyleId === "string") info.gridStyleId = node.gridStyleId; } catch(e) {}

  // ── Component-specific info ──
  if (node.type === "COMPONENT" || node.type === "COMPONENT_SET") {
    try { info.description = node.description; } catch(e) {}
    // Expose component property definitions for COMPONENT/COMPONENT_SET
    try {
      if (node.componentPropertyDefinitions) {
        var defs = node.componentPropertyDefinitions;
        var defKeys = Object.keys(defs);
        if (defKeys.length > 0) {
          info.componentPropertyDefinitions = {};
          for (var di = 0; di < defKeys.length; di++) {
            var dk = defKeys[di];
            var d = defs[dk];
            info.componentPropertyDefinitions[dk] = { type: d.type, defaultValue: d.defaultValue };
          }
        }
      }
    } catch(e) {}
  }
  if (node.type === "INSTANCE") {
    // Issue #2: expose source component reference. mainComponent is async-only
    // under documentAccess: dynamic-page, so defer to the caller's collector.
    if (instanceCollector) instanceCollector.push({ info: info, node: node });
    try { if (node.overrides && node.overrides.length) info.overrideCount = node.overrides.length; } catch(e) {}
    // Issue #4: expose explicit component property values on this instance
    try {
      if (node.componentProperties) {
        var props = node.componentProperties;
        var propKeys = Object.keys(props);
        if (propKeys.length > 0) {
          info.componentPropertyValues = {};
          for (var pi = 0; pi < propKeys.length; pi++) {
            var pk = propKeys[pi];
            var pv = props[pk];
            info.componentPropertyValues[pk] = { type: pv.type, value: pv.value };
          }
        }
      }
    } catch(e) {}
  }

  // ── VECTOR / BOOLEAN_OPERATION ──
  if (node.type === "VECTOR" || node.type === "BOOLEAN_OPERATION") {
    try { if (node.vectorPaths) info.pathCount = node.vectorPaths.length; } catch(e) {}
  }

  // ── Image detection — flag nodes with image fills (compact+full) ──
  if ((isCompact || isFull) && hasImageFill(node)) {
    info.hasImage = true;
    info.imageHint = "Use figma_read screenshot with nodeId to extract this image";
  }

  // ── Icon detection — flag small vector/instance nodes (compact+full) ──
  if ((isCompact || isFull) && isLikelyIcon(node)) {
    info.isIcon = true;
    info.iconHint = "Use figma_read export_svg with nodeId to extract SVG markup";
  }

  // ── Children ──
  if (node && typeof node === "object" && "children" in node && Array.isArray(node.children) && node.children.length) {
    var budgetSpent = !!(walkState && typeof walkState.remaining === "number" && walkState.remaining <= 0);
    if (depth >= maxDepth || budgetSpent) {
      // At the depth limit / node budget: summarize instead of truncating to empty []
      info.childCount = node.children.length;
      var texts = collectCapped(collectTextContent, node, 15);
      if (texts.items.length) info.textContent = texts.items;
      var icons = collectCapped(collectIconNames, node, 10);
      if (icons.items.length) info.iconNames = icons.items;
      info.childrenTruncated = budgetSpent ? "nodeBudget" : "maxDepth";
      if (texts.truncated) info.textContentTruncated = true;
      if (icons.truncated) info.iconNamesTruncated = true;
      if (budgetSpent) walkState.truncated = true;
    } else {
      info.children = node.children
        .map(function(c) { return extractDesignTree(c, depth + 1, maxDepth, detailLevel, filterInvisible, tokenCollector, instanceCollector, walkState); })
        .filter(Boolean);
    }
  }

  return info;
}

// Token collection happens inline during extractDesignTree via tokenCollector.
