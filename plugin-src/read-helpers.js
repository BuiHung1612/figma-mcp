// ─── READ HELPERS ─────────────────────────────────────────────────────────────

// Helper to aggregate repeating list items and state variants among sibling children
function aggregateRepeatedChildren(children) {
  if (!children || !Array.isArray(children) || children.length <= 2) return children;

  var out = [];
  var i = 0;
  while (i < children.length) {
    var cur = children[i];
    if (!cur || typeof cur !== "object") {
      out.push(cur);
      i++;
      continue;
    }

    var group = [cur];
    var j = i + 1;
    while (j < children.length) {
      var next = children[j];
      if (isSimilarStructure(cur, next)) {
        group.push(next);
        j++;
      } else {
        break;
      }
    }

    if (group.length >= 3) {
      // Aggregate list items
      var rep = Object.assign({}, group[0]);
      var itemTexts = [];
      for (var gi = 0; gi < group.length; gi++) {
        var gNode = group[gi];
        if (gNode.textContent && Array.isArray(gNode.textContent)) {
          itemTexts.push(gNode.textContent.slice(0, 3).join(" | "));
        } else if (gNode.name) {
          itemTexts.push(gNode.name);
        }
      }
      rep._isRepeater = true;
      rep.repeatedCount = group.length;
      rep.itemSamples = itemTexts.slice(0, 5);
      rep._aggregationHint = "Aggregated " + group.length + " similar repeating items to save context tokens. Structure matches representative item.";
      out.push(rep);
      i = j;
    } else {
      out.push(cur);
      i++;
    }
  }

  // Also check if siblings are screen state frames (e.g. Default, Typing, Success, Error)
  return aggregateStateFrames(out);
}

// Group state frames with same dimensions and similar core layouts
function aggregateStateFrames(children) {
  if (!children || !Array.isArray(children) || children.length <= 1) return children;

  // Detect if children are screen variants (e.g. FRAME with width >= 300, height >= 300)
  var frameChildren = [];
  var isAllScreens = true;
  for (var k = 0; k < children.length; k++) {
    var c = children[k];
    if (c && c.type === "FRAME" && c.width && c.height && c.width >= 320 && c.height >= 320) {
      frameChildren.push(c);
    } else {
      isAllScreens = false;
    }
  }

  // If we have multiple screen frames of identical dimensions, group them into Base + State Diffs
  if (frameChildren.length >= 2 && frameChildren.length === children.length) {
    var firstW = frameChildren[0].width;
    var firstH = frameChildren[0].height;
    var allSameDim = frameChildren.every(function(f) {
      return Math.abs(f.width - firstW) <= 10 && Math.abs(f.height - firstH) <= 10;
    });

    if (allSameDim) {
      var baseFrame = frameChildren[0];
      var states = [];
      for (var si = 0; si < frameChildren.length; si++) {
        var sf = frameChildren[si];
        var diffInfo = extractStateDiff(baseFrame, sf, si === 0);
        states.push({
          id: sf.id,
          stateName: sf.name,
          diff: diffInfo
        });
      }

      var aggregatedNode = Object.assign({}, baseFrame);
      aggregatedNode._aggregatedStates = {
        totalStates: frameChildren.length,
        baseStateName: baseFrame.name,
        states: states,
        hint: "All " + frameChildren.length + " screen states share the base layout above. Only diffs (text/fills/components) are listed in states."
      };
      return [aggregatedNode];
    }
  }

  return children;
}

function extractStateDiff(base, current, isBase) {
  if (isBase) return { status: "BASE_TEMPLATE" };
  var diff = {};
  if (base.name !== current.name) diff.name = current.name;

  // Compare direct text content
  var currentTexts = collectNodeTextArray(current);
  var baseTexts = collectNodeTextArray(base);
  var uniqueTexts = currentTexts.filter(function(t) { return baseTexts.indexOf(t) === -1; });
  if (uniqueTexts.length > 0) {
    diff.uniqueTexts = uniqueTexts.slice(0, 10);
  }

  // Check fills diff
  if (current.fill !== base.fill) diff.fill = current.fill;
  if (current.stroke !== base.stroke) diff.stroke = current.stroke;

  return diff;
}

function collectNodeTextArray(node) {
  var res = [];
  if (!node) return res;
  if (node.content) res.push(node.content);
  if (node.textContent && Array.isArray(node.textContent)) res = res.concat(node.textContent);
  if (node.children && Array.isArray(node.children)) {
    for (var i = 0; i < node.children.length; i++) {
      res = res.concat(collectNodeTextArray(node.children[i]));
    }
  }
  return res;
}

function isSimilarStructure(a, b) {
  if (!a || !b || typeof a !== "object" || typeof b !== "object") return false;
  if (a.type !== b.type) return false;
  if (a.role && b.role && a.role === b.role) return true;
  var wDiff = Math.abs((a.width || 0) - (b.width || 0));
  var hDiff = Math.abs((a.height || 0) - (b.height || 0));
  if (wDiff <= 4 && hDiff <= 4 && a.type === b.type) {
    var aChildCount = a.children ? a.children.length : (a.childCount || 0);
    var bChildCount = b.children ? b.children.length : (b.childCount || 0);
    if (aChildCount === bChildCount) return true;
  }
  return false;
}

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
      if (node.textCase && node.textCase !== "ORIGINAL") {
        out.textCase = node.textCase;
        if (node.textCase === "UPPER") {
          out.textTransform = "uppercase";
          if (out.content) out.renderedContent = out.content.toUpperCase();
        } else if (node.textCase === "LOWER") {
          out.textTransform = "lowercase";
          if (out.content) out.renderedContent = out.content.toLowerCase();
        } else if (node.textCase === "TITLE") {
          out.textTransform = "capitalize";
        }
      }
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
      if (s.textCase && s.textCase !== "ORIGINAL") seg.textCase = s.textCase;
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
    if (head.textCase !== undefined) {
      out.textCase = head.textCase;
      if (head.textCase === "UPPER") {
        out.textTransform = "uppercase";
        if (out.content) out.renderedContent = out.content.toUpperCase();
      } else if (head.textCase === "LOWER") {
        out.textTransform = "lowercase";
        if (out.content) out.renderedContent = out.content.toLowerCase();
      } else if (head.textCase === "TITLE") {
        out.textTransform = "capitalize";
      }
    }
  } else {
    // No segment API — salvage whatever is not mixed.
    if (typeof node.fontSize === "number") out.fontSize = node.fontSize;
    if (!isMixed(node.fontName) && node.fontName) {
      out.fontFamily = node.fontName.family;
      out.fontWeight = node.fontName.style;
    }
    var fallbackHex = getFillHex(node);
    if (fallbackHex) out.fill = fallbackHex;
    try {
      if (node.textCase && node.textCase !== "ORIGINAL") {
        out.textCase = node.textCase;
        if (node.textCase === "UPPER") out.textTransform = "uppercase";
        else if (node.textCase === "LOWER") out.textTransform = "lowercase";
        else if (node.textCase === "TITLE") out.textTransform = "capitalize";
      }
    } catch(e) {}
  }

  return out;
}

// ── Design Token / Variable Resolver Map ──
var cachedVariableResolverMap = null;
var cachedVariableResolverMapTime = 0;

async function buildVariableResolverMapAsync(forceRefresh) {
  var now = Date.now();
  if (!forceRefresh && cachedVariableResolverMap && (now - cachedVariableResolverMapTime < 15000)) {
    return cachedVariableResolverMap;
  }
  var map = { byId: {}, byName: {} };
  try {
    if (typeof figma.variables !== "undefined" && typeof figma.variables.getLocalVariablesAsync === "function") {
      var vars = await figma.variables.getLocalVariablesAsync();
      var rawMap = {};
      for (var i = 0; i < vars.length; i++) {
        var v = vars[i];
        if (v && v.id) rawMap[v.id] = v;
      }

      function resolveVarValue(v, depth) {
        if (!v || depth > 6) return null;
        var defaultVal = null;
        if (v.valuesByMode) {
          var modeKeys = Object.keys(v.valuesByMode);
          if (modeKeys.length > 0) defaultVal = v.valuesByMode[modeKeys[0]];
        }
        if (defaultVal && typeof defaultVal === "object" && defaultVal.type === "VARIABLE_ALIAS" && defaultVal.id) {
          var targetVar = rawMap[defaultVal.id];
          if (targetVar) {
            var targetRes = resolveVarValue(targetVar, depth + 1);
            return {
              isAlias: true,
              targetId: targetVar.id,
              targetName: targetVar.name,
              primitiveName: (targetRes && targetRes.primitiveName) ? targetRes.primitiveName : targetVar.name,
              resolvedValue: targetRes ? targetRes.resolvedValue : null,
            };
          }
        }
        if (defaultVal && typeof defaultVal === "object" && "r" in defaultVal && "g" in defaultVal && "b" in defaultVal) {
          return { isAlias: false, resolvedValue: rgbToHex(defaultVal), primitiveName: v.name };
        }
        if (typeof defaultVal === "number" || typeof defaultVal === "string" || typeof defaultVal === "boolean") {
          return { isAlias: false, resolvedValue: defaultVal, primitiveName: v.name };
        }
        return { isAlias: false, resolvedValue: defaultVal, primitiveName: v.name };
      }

      for (var j = 0; j < vars.length; j++) {
        var vr = vars[j];
        if (!vr || !vr.id) continue;
        var res = resolveVarValue(vr, 0);
        var item = {
          id: vr.id,
          name: vr.name,
          resolvedType: vr.resolvedType,
          isAlias: res ? res.isAlias : false,
          targetName: res ? res.targetName : null,
          primitiveName: res ? res.primitiveName : vr.name,
          resolvedValue: res ? res.resolvedValue : null,
        };
        map.byId[vr.id] = item;
        map.byName[vr.name] = item;
      }
    }
  } catch(e) {}
  cachedVariableResolverMap = map;
  cachedVariableResolverMapTime = now;
  return map;
}

// ── Clean Variant & Component Properties Parser ──
function cleanComponentProperties(node) {
  var out = { variant: null, props: null };
  if (!node) return out;

  // 1. Native variantProperties on instance
  if (node.variantProperties && typeof node.variantProperties === "object") {
    var varKeys = Object.keys(node.variantProperties);
    if (varKeys.length > 0) {
      out.variant = {};
      for (var vk = 0; vk < varKeys.length; vk++) {
        out.variant[varKeys[vk]] = node.variantProperties[varKeys[vk]];
      }
    }
  }

  // 2. Component Properties parsing (strip #ID suffix and separate variants vs normal props)
  if (node.componentProperties && typeof node.componentProperties === "object") {
    var rawProps = node.componentProperties;
    var propKeys = Object.keys(rawProps);
    if (propKeys.length > 0) {
      var cleanProps = {};
      var inferredVariants = out.variant || {};
      for (var pk = 0; pk < propKeys.length; pk++) {
        var rawKey = propKeys[pk];
        var propObj = rawProps[rawKey];
        if (!propObj) continue;
        var cleanKey = rawKey.split("#")[0].trim();
        if (propObj.type === "VARIANT") {
          inferredVariants[cleanKey] = propObj.value;
        } else {
          cleanProps[cleanKey] = propObj.value;
        }
      }
      if (Object.keys(cleanProps).length > 0) out.props = cleanProps;
      if (Object.keys(inferredVariants).length > 0) out.variant = inferredVariants;
    }
  }
  return out;
}

// ── Infer Semantic Role ──
function inferSemanticRole(node, info) {
  if (!node) return null;
  var name = (node.name || "").toLowerCase();
  var type = node.type;

  if (isLikelyIcon(node) || name.indexOf("icon") !== -1) return "icon";
  if (name.indexOf("button") !== -1 || name.indexOf("btn") !== -1) return "button";
  if (name.indexOf("input") !== -1 || name.indexOf("textfield") !== -1 || name.indexOf("search") !== -1) return "input";
  if (name.indexOf("badge") !== -1 || name.indexOf("tag") !== -1 || name.indexOf("chip") !== -1 || name.indexOf("pill") !== -1) return "badge";
  if (name.indexOf("avatar") !== -1 || name.indexOf("userpic") !== -1 || name.indexOf("profile-pic") !== -1) return "avatar";
  if (name.indexOf("card") !== -1) return "card";
  if (name.indexOf("modal") !== -1 || name.indexOf("dialog") !== -1 || name.indexOf("popup") !== -1) return "modal";
  if (name.indexOf("checkbox") !== -1) return "checkbox";
  if (name.indexOf("switch") !== -1 || name.indexOf("toggle") !== -1) return "switch";
  if (name.indexOf("divider") !== -1 || name.indexOf("separator") !== -1) return "divider";
  if (name.indexOf("header") !== -1 || name.indexOf("navbar") !== -1 || name.indexOf("appbar") !== -1) return "header";
  if (name.indexOf("tab") !== -1) return "tab";

  if (type === "INSTANCE") {
    if (info && info.componentSetName) {
      var setName = info.componentSetName.toLowerCase();
      if (setName.indexOf("button") !== -1) return "button";
      if (setName.indexOf("input") !== -1) return "input";
      if (setName.indexOf("badge") !== -1) return "badge";
      if (setName.indexOf("avatar") !== -1) return "avatar";
      if (setName.indexOf("icon") !== -1) return "icon";
    }
  }
  return null;
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
      var entryNode = batch[b].node;
      var entryInfo = batch[b].info;
      if (main) {
        var desc = describeMainComponent(main);
        if (apply) apply(entryInfo, desc);
        else Object.assign(entryInfo, desc);
      }
      var parsed = cleanComponentProperties(entryNode);
      if (parsed.variant) entryInfo.variant = parsed.variant;
      if (parsed.props) entryInfo.props = parsed.props;
      var role = inferSemanticRole(entryNode, entryInfo);
      if (role) entryInfo.role = role;
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
        info.fill = rgbToHex(fills[0].color, fills[0].opacity);
        if (tokenCollector && info.fill) tokenCollector.colors.add(info.fill);
        if (fills[0].opacity !== undefined && fills[0].opacity !== 1) {
          info.fillOpacity = Math.round(fills[0].opacity * 1000) / 1000;
        }
      } else {
        info.fills = [];
        for (var fi = 0; fi < fills.length; fi++) {
          var f = fills[fi];
          var fd = { type: f.type, visible: f.visible !== false };
          if (f.type === "SOLID") {
            fd.color = rgbToHex(f.color, f.opacity);
            if (tokenCollector && fd.color) tokenCollector.colors.add(fd.color);
            if (f.opacity !== undefined && f.opacity !== 1) fd.opacity = Math.round(f.opacity * 1000) / 1000;
          } else if (f.type === "GRADIENT_LINEAR" || f.type === "GRADIENT_RADIAL" || f.type === "GRADIENT_ANGULAR") {
            fd.gradientStops = f.gradientStops ? f.gradientStops.map(function(gs) {
              var sc = rgbToHex(gs.color, gs.color ? gs.color.a : 1);
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
        info.stroke = rgbToHex(strokes[0].color, strokes[0].opacity);
        if (tokenCollector && info.stroke) tokenCollector.colors.add(info.stroke);
        applyStrokeWeight(node, info);
        if (strokes[0].opacity !== undefined && strokes[0].opacity !== 1) {
          info.strokeOpacity = Math.round(strokes[0].opacity * 1000) / 1000;
        }
        if (node.strokeAlign) info.strokeAlign = node.strokeAlign;
      } else {
        info.strokes = strokes.map(function(s) {
          var sd = { type: s.type };
          if (s.type === "SOLID") {
            sd.color = rgbToHex(s.color, s.opacity);
            if (tokenCollector && sd.color) tokenCollector.colors.add(sd.color);
          }
          if (s.opacity !== undefined && s.opacity !== 1) sd.opacity = Math.round(s.opacity * 1000) / 1000;
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

  // ── Bound Variables (Design Tokens) — full & compact ──
  if (isFull || isCompact) try {
    if (node.boundVariables) {
      var bv = {};
      var bvKeys = Object.keys(node.boundVariables);
      var varMap = (walkState && walkState.variableMap) ? walkState.variableMap.byId : null;
      for (var bvi = 0; bvi < bvKeys.length; bvi++) {
        var bvk = bvKeys[bvi];
        var binding = node.boundVariables[bvk];
        if (binding) {
          var bindings = Array.isArray(binding) ? binding : [binding];
          var resolvedList = [];
          for (var bi = 0; bi < bindings.length; bi++) {
            var bObj = bindings[bi];
            if (!bObj || !bObj.id) continue;
            var vInfo = varMap && varMap[bObj.id] ? varMap[bObj.id] : null;
            if (vInfo) {
              var entry = { id: bObj.id, name: vInfo.name, type: vInfo.resolvedType };
              if (vInfo.resolvedValue !== null && vInfo.resolvedValue !== undefined) entry.value = vInfo.resolvedValue;
              if (vInfo.isAlias && vInfo.targetName) entry.aliasTarget = vInfo.targetName;
              resolvedList.push(entry);
            } else {
              resolvedList.push({ id: bObj.id });
            }
          }
          if (resolvedList.length > 0) {
            bv[bvk] = Array.isArray(binding) ? resolvedList : resolvedList[0];
            var primaryEntry = resolvedList[0];
            if (primaryEntry && primaryEntry.name) {
              if (bvk === "fills" || bvk === "fill") info.fillToken = primaryEntry.name;
              else if (bvk === "strokes" || bvk === "stroke") info.strokeToken = primaryEntry.name;
              else if (bvk === "itemSpacing") info.gapToken = primaryEntry.name;
              else if (bvk === "paddingTop" || bvk === "paddingBottom" || bvk === "paddingLeft" || bvk === "paddingRight") info.paddingToken = primaryEntry.name;
              else if (bvk === "topLeftRadius" || bvk === "cornerRadius") info.radiusToken = primaryEntry.name;
            }
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
        if (eff.color) ed.color = rgbToHex(eff.color, eff.color.a);
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
      if (info.gapToken) layoutObj.gapToken = info.gapToken;
      if (info.paddingToken) layoutObj.paddingToken = info.paddingToken;
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
    var parsedProps = cleanComponentProperties(node);
    if (parsedProps.variant) info.variant = parsedProps.variant;
    if (parsedProps.props) info.props = parsedProps.props;
    if (instanceCollector) instanceCollector.push({ info: info, node: node });
    try { if (node.overrides && node.overrides.length) info.overrideCount = node.overrides.length; } catch(e) {}
  }

  // ── Semantic Role ──
  var semanticRole = inferSemanticRole(node, info);
  if (semanticRole) info.role = semanticRole;

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
      var rawChildren = node.children
        .map(function(c) { return extractDesignTree(c, depth + 1, maxDepth, detailLevel, filterInvisible, tokenCollector, instanceCollector, walkState); })
        .filter(Boolean);
      info.children = aggregateRepeatedChildren(rawChildren);
    }
  }

  return info;
}

// Token collection happens inline during extractDesignTree via tokenCollector.
