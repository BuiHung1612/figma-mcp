// ─── READ DETAIL HANDLERS ─────────────────────────────────────────────────────

// get_node_detail — CSS-like properties for a single node (no tree traversal)
handlers.get_node_detail = async function(params) {
  // Batch support: if nodeIds array is provided, inspect multiple nodes in one call
  if (params && (Array.isArray(params.nodeIds) || Array.isArray(params.ids))) {
    var ids = params.nodeIds || params.ids;
    var results = [];
    for (var bi = 0; bi < ids.length; bi++) {
      try {
        var nd = await handlers.get_node_detail({ id: ids[bi] });
        results.push(nd);
      } catch (e) {
        results.push({ id: ids[bi], error: e.message });
      }
    }
    return { nodes: results, count: results.length };
  }

  // Accept id, nodeId, node_id, targetId, target_id, name, nodeName, node_name — try ID first then name fallback
  var id = params ? (params.id || params.nodeId || params.node_id || params.targetId || params.target_id) : null;
  var nodeName = params ? (params.name || params.nodeName || params.node_name) : null;
  var node = null;
  if (id) node = await findNodeByIdAsync(id);
  if (!node && nodeName) node = findNodeByName(nodeName);
  if (!node) {
    var errMsg = typeof getNodeNotFoundContext === "function"
      ? getNodeNotFoundContext(id, nodeName)
      : ("Node not found: " + (id || nodeName || "no id/name given") + ". Use figma_read get_page_nodes to get current node IDs.");
    throw new Error(errMsg);
  }

  var detail = {
    id: node.id, name: node.name, type: node.type,
    x: Math.round(node.x), y: Math.round(node.y),
    width: Math.round(node.width), height: Math.round(node.height),
  };

  // clipsContent resolved here so get_css does not need a second node fetch
  try { if ("clipsContent" in node && node.clipsContent) detail.clipsContent = true; } catch(e) {}

  // Fill(s)
  try {
    if (node.fills && node.fills.length) {
      detail.fills = [];
      for (var fi = 0; fi < node.fills.length; fi++) {
        var f = node.fills[fi];
        if (f.visible === false) continue;
        var fd = { type: f.type };
        if (f.type === "SOLID") {
          fd.color = rgbToHex(f.color, f.opacity);
          if (f.opacity !== undefined && f.opacity !== 1) fd.opacity = Math.round(f.opacity * 1000) / 1000;
        } else if (f.type === "GRADIENT_LINEAR" || f.type === "GRADIENT_RADIAL" || f.type === "GRADIENT_ANGULAR") {
          fd.gradientStops = f.gradientStops ? f.gradientStops.map(function(gs) {
            return { color: rgbToHex(gs.color, gs.color ? gs.color.a : 1), position: Math.round(gs.position * 100) / 100 };
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
        }

        // Link bound variable token if present
        try {
          if (node.boundVariables && node.boundVariables.fills) {
            var fBindings = Array.isArray(node.boundVariables.fills) ? node.boundVariables.fills : [node.boundVariables.fills];
            if (fBindings[fi] && fBindings[fi].id) {
              var boundVar = await resolveVariableValueAsync(fBindings[fi].id, node, 0);
              if (boundVar) {
                fd.boundToken = {
                  id: fBindings[fi].id,
                  name: boundVar.name,
                  primitive: boundVar.primitiveName,
                  resolvedHex: boundVar.hex || boundVar.resolvedValue,
                };
                if (boundVar.hex) fd.color = boundVar.hex;
              }
            }
          }
        } catch(e3) {}

        detail.fills.push(fd);
      }
    }
  } catch(e) {}

  // Stroke (all strokes)
  try {
    if (node.strokes && node.strokes.length) {
      var dStrokes = node.strokes;
      var strokeBindings = (node.boundVariables && node.boundVariables.strokes) ? (Array.isArray(node.boundVariables.strokes) ? node.boundVariables.strokes : [node.boundVariables.strokes]) : [];

      if (dStrokes.length === 1 && dStrokes[0].type === "SOLID") {
        detail.stroke = rgbToHex(dStrokes[0].color, dStrokes[0].opacity);
        if (dStrokes[0].opacity !== undefined && dStrokes[0].opacity !== 1) {
          detail.strokeOpacity = Math.round(dStrokes[0].opacity * 1000) / 1000;
        }
        try {
          if (strokeBindings[0] && strokeBindings[0].id) {
            var boundStrokeVar = await resolveVariableValueAsync(strokeBindings[0].id, node, 0);
            if (boundStrokeVar) {
              detail.strokeBoundToken = {
                id: strokeBindings[0].id,
                name: boundStrokeVar.name,
                primitive: boundStrokeVar.primitiveName,
                resolvedHex: boundStrokeVar.hex || boundStrokeVar.resolvedValue,
              };
              if (boundStrokeVar.hex) detail.stroke = boundStrokeVar.hex;
            }
          }
        } catch(e4) {}
      } else {
        detail.strokes = [];
        for (var si = 0; si < dStrokes.length; si++) {
          var s = dStrokes[si];
          var sd = { type: s.type };
          if (s.type === "SOLID") sd.color = rgbToHex(s.color, s.opacity);
          if (s.opacity !== undefined && s.opacity !== 1) sd.opacity = Math.round(s.opacity * 1000) / 1000;
          try {
            if (strokeBindings[si] && strokeBindings[si].id) {
              var bsv = await resolveVariableValueAsync(strokeBindings[si].id, node, 0);
              if (bsv) {
                sd.boundToken = {
                  id: strokeBindings[si].id,
                  name: bsv.name,
                  primitive: bsv.primitiveName,
                  resolvedHex: bsv.hex || bsv.resolvedValue,
                };
                if (bsv.hex) sd.color = bsv.hex;
              }
            }
          } catch(e5) {}
          detail.strokes.push(sd);
        }
      }
      applyStrokeWeight(node, detail);
      if (detail.strokeWeight === undefined && typeof node.strokeWeight === "number") {
        detail.strokeWeight = node.strokeWeight;
      }
      detail.strokeAlign = node.strokeAlign;
    }
  } catch(e) {}

  // Corner radius
  try {
    if ("cornerRadius" in node && node.cornerRadius !== 0) {
      if (typeof node.cornerRadius === "number") {
        detail.borderRadius = node.cornerRadius + "px";
      } else {
        detail.borderRadius = (node.topLeftRadius || 0) + "px " + (node.topRightRadius || 0) + "px " + (node.bottomRightRadius || 0) + "px " + (node.bottomLeftRadius || 0) + "px";
      }
    }
  } catch(e) {}

  // Rotation
  try { if ("rotation" in node && node.rotation !== 0) detail.rotation = Math.round(node.rotation * 100) / 100; } catch(e) {}

  // Opacity, blendMode, visible
  try { if (node.opacity !== undefined && node.opacity !== 1) detail.opacity = Math.round(node.opacity * 100) / 100; } catch(e) {}
  try { if (node.blendMode && node.blendMode !== "NORMAL" && node.blendMode !== "PASS_THROUGH") detail.blendMode = node.blendMode; } catch(e) {}
  try { if ("visible" in node && !node.visible) detail.visible = false; } catch(e) {}

  // Effects → CSS boxShadow + filter (blur)
  try {
    if (node.effects && node.effects.length) {
      var shadows = [];
      var blurValues = [];
      for (var ei = 0; ei < node.effects.length; ei++) {
        var eff = node.effects[ei];
        if (eff.visible === false) continue;
        if (eff.type === "DROP_SHADOW" || eff.type === "INNER_SHADOW") {
          var c = eff.color;
          var rgba = "rgba(" + Math.round(c.r * 255) + "," + Math.round(c.g * 255) + "," + Math.round(c.b * 255) + "," + (c.a !== undefined ? Math.round(c.a * 100) / 100 : 1) + ")";
          var prefix = eff.type === "INNER_SHADOW" ? "inset " : "";
          shadows.push(prefix + (eff.offset ? eff.offset.x : 0) + "px " + (eff.offset ? eff.offset.y : 0) + "px " + (eff.radius || 0) + "px " + (eff.spread || 0) + "px " + rgba);
        } else if (eff.type === "LAYER_BLUR") {
          blurValues.push("blur(" + (eff.radius || 0) + "px)");
        } else if (eff.type === "BACKGROUND_BLUR") {
          detail.backdropFilter = "blur(" + (eff.radius || 0) + "px)";
        }
      }
      if (shadows.length) detail.boxShadow = shadows.join(", ");
      if (blurValues.length) detail.filter = blurValues.join(" ");
    }
  } catch(e) {}

  // Layout / padding
  try {
    if (node.layoutMode && node.layoutMode !== "NONE") {
      var alignMap = { "MIN": "flex-start", "CENTER": "center", "MAX": "flex-end", "SPACE_BETWEEN": "space-between" };
      detail.css = {
        display: "flex",
        flexDirection: node.layoutMode === "HORIZONTAL" ? "row" : "column",
        gap: node.itemSpacing + "px",
        alignItems: alignMap[node.counterAxisAlignItems] || node.counterAxisAlignItems,
        justifyContent: alignMap[node.primaryAxisAlignItems] || node.primaryAxisAlignItems,
        padding: node.paddingTop + "px " + node.paddingRight + "px " + node.paddingBottom + "px " + node.paddingLeft + "px",
      };
    }
  } catch(e) {}

  // Text properties — resolveTextStyle unwraps figma.mixed on multi-style text
  if (node.type === "TEXT") {
    var textStyle = resolveTextStyle(node);
    if (textStyle) {
      detail.content = textStyle.content;
      detail.color = textStyle.fill || null;
      detail.fontSize = textStyle.fontSize !== undefined ? textStyle.fontSize + "px" : null;
      detail.fontFamily = textStyle.fontFamily || null;
      detail.fontWeight = textStyle.fontWeight || null;
      if (textStyle.lineHeight !== undefined) {
        detail.lineHeight = typeof textStyle.lineHeight === "number" ? textStyle.lineHeight + "px" : textStyle.lineHeight;
      } else if (!isMixed(node.lineHeight) && node.lineHeight && node.lineHeight.unit === "AUTO") {
        detail.lineHeight = "normal";
      }
      if (textStyle.letterSpacing !== undefined) detail.letterSpacing = textStyle.letterSpacing + "px";
      if (textStyle.textDecoration) detail.textDecoration = textStyle.textDecoration;
      if (textStyle.textCase) detail.textCase = textStyle.textCase;
      if (textStyle.textTransform) detail.textTransform = textStyle.textTransform;
      if (textStyle.renderedContent) detail.renderedContent = textStyle.renderedContent;
      if (textStyle.mixed) {
        detail.mixedStyles = true;
        if (textStyle.segments) detail.segments = textStyle.segments;
      }
    }
    try { detail.textAlign = node.textAlignHorizontal ? node.textAlignHorizontal.toLowerCase() : null; } catch(e) {}
  }

  // P1: Resolve bound variables → name + resolvedType + value (deep recursive resolution)
  try {
    if (node.boundVariables) {
      var bv = {};
      var bvKeys = Object.keys(node.boundVariables);
      for (var bvi = 0; bvi < bvKeys.length; bvi++) {
        var bvk = bvKeys[bvi];
        var binding = node.boundVariables[bvk];
        if (!binding) continue;
        var bindings = Array.isArray(binding) ? binding : [binding];
        var resolved = [];
        for (var bi = 0; bi < bindings.length; bi++) {
          var alias = bindings[bi];
          if (!alias || !alias.id) continue;
          var entry = { id: alias.id };
          try {
            var variable = await getVariableSafeAsync(alias.id);
            if (variable) {
              entry.name = variable.name;
              entry.resolvedType = variable.resolvedType;
              var resolvedToken = await resolveVariableValueAsync(variable, node, 0);
              if (resolvedToken) {
                entry.value = resolvedToken.resolvedValue !== undefined ? resolvedToken.resolvedValue : (resolvedToken.hex || resolvedToken.value);
                if (resolvedToken.type === "ALIAS") {
                  entry.aliasTarget = resolvedToken.targetName;
                  entry.primitiveName = resolvedToken.primitiveName;
                  entry.resolvedValue = resolvedToken.resolvedValue;
                }
              }
            }
          } catch(e2) {}
          resolved.push(entry);
        }
        if (resolved.length > 0) bv[bvk] = Array.isArray(binding) ? resolved : resolved[0];
      }
      if (Object.keys(bv).length > 0) detail.boundVariables = bv;
    }
  } catch(e) {}

  // Attach direct semantic token names on detail
  try {
    if (detail.boundVariables) {
      var bvKeys = Object.keys(detail.boundVariables);
      for (var bvi = 0; bvi < bvKeys.length; bvi++) {
        var bvk = bvKeys[bvi];
        var bEntry = detail.boundVariables[bvk];
        var pName = (bEntry && typeof bEntry === "object") ? (Array.isArray(bEntry) ? (bEntry[0] && bEntry[0].name) : bEntry.name) : null;
        if (pName) {
          if (bvk === "fills" || bvk === "fill") detail.fillToken = pName;
          else if (bvk === "strokes" || bvk === "stroke") detail.strokeToken = pName;
          else if (bvk === "itemSpacing") detail.gapToken = pName;
          else if (bvk === "paddingTop" || bvk === "paddingBottom" || bvk === "paddingLeft" || bvk === "paddingRight") detail.paddingToken = pName;
          else if (bvk === "topLeftRadius" || bvk === "cornerRadius") detail.radiusToken = pName;
        }
      }
    }
  } catch(e) {}

  // P4: Resolve style IDs → name + values (not just opaque IDs)
  try {
    if (node.textStyleId && typeof node.textStyleId === "string") {
      detail.textStyleId = node.textStyleId;
      try {
        var ts = await figma.getStyleByIdAsync(node.textStyleId);
        if (ts) detail.textStyle = { name: ts.name, fontSize: ts.fontSize, fontFamily: ts.fontName ? ts.fontName.family : null, fontWeight: ts.fontName ? ts.fontName.style : null };
      } catch(e2) {}
    }
  } catch(e) {}
  try {
    if (node.fillStyleId && typeof node.fillStyleId === "string") {
      detail.fillStyleId = node.fillStyleId;
      try {
        var fs = await figma.getStyleByIdAsync(node.fillStyleId);
        if (fs) {
          var fsHex = null;
          if (fs.paints && fs.paints.length && fs.paints[0].type === "SOLID") fsHex = rgbToHex(fs.paints[0].color);
          detail.fillStyle = { name: fs.name, hex: fsHex };
        }
      } catch(e2) {}
    }
  } catch(e) {}
  try { if (node.strokeStyleId && typeof node.strokeStyleId === "string") detail.strokeStyleId = node.strokeStyleId; } catch(e) {}
  try { if (node.effectStyleId && typeof node.effectStyleId === "string") detail.effectStyleId = node.effectStyleId; } catch(e) {}

  // Instance: source component reference + property values + overrides + P5 variantLabel + clean variant & props
  if (node.type === "INSTANCE") {
    var instComp = await getMainComponentSafe(node);
    if (instComp) Object.assign(detail, describeMainComponent(instComp));
    var parsed = typeof cleanComponentProperties === "function" ? cleanComponentProperties(node) : null;
    if (parsed) {
      if (parsed.variant) detail.variant = parsed.variant;
      if (parsed.props) detail.props = parsed.props;
    }
    // P3: Full override list instead of just count
    try {
      if (node.overrides && node.overrides.length) {
        detail.overrides = node.overrides.map(function(ov) {
          return { id: ov.id, overriddenFields: ov.overriddenFields || [] };
        });
        detail.overrideCount = node.overrides.length;
      }
    } catch(e) {}
    try {
      if (node.componentProperties) {
        var iProps = node.componentProperties;
        var iPropKeys = Object.keys(iProps);
        if (iPropKeys.length > 0) {
          detail.componentPropertyValues = {};
          for (var ipi = 0; ipi < iPropKeys.length; ipi++) {
            var ipk = iPropKeys[ipi];
            var ipv = iProps[ipk];
            detail.componentPropertyValues[ipk] = { type: ipv.type, value: ipv.value };
          }
        }
      }
    } catch(e) {}
  }

  // Semantic Role
  var semanticRole = typeof inferSemanticRole === "function" ? inferSemanticRole(node, detail) : null;
  if (semanticRole) detail.role = semanticRole;

  // Children count + text content summary
  if (node && typeof node === "object" && "children" in node && Array.isArray(node.children)) {
    detail.childCount = node.children.length;
    var texts = collectTextContent(node, 20);
    if (texts.length) detail.textContent = texts;
    var icons = collectIconNames(node, 10);
    if (icons.length) detail.iconNames = icons;
  }

  return detail;
};

// P2: get_css — return a ready-to-use CSS string for a single node
// Covers: layout (flex), spacing, typography, fills, stroke, radius, shadow, opacity, position
handlers.get_css = async function(params) {
  // Reuse get_node_detail to get all structured data, then format as CSS
  var detail = await handlers.get_node_detail(params);
  var lines = [];

  // Position — only emit absolute positioning for non-auto-layout nodes (flex children use flow)
  if (!detail.css) {
    lines.push("position: absolute;");
    if (detail.x !== undefined) lines.push("left: " + detail.x + "px;");
    if (detail.y !== undefined) lines.push("top: " + detail.y + "px;");
  }
  if (detail.width !== undefined) lines.push("width: " + detail.width + "px;");
  if (detail.height !== undefined) lines.push("height: " + detail.height + "px;");

  // Layout (flex)
  if (detail.css) {
    var c = detail.css;
    if (c.display) lines.push("display: " + c.display + ";");
    if (c.flexDirection) lines.push("flex-direction: " + c.flexDirection + ";");
    if (c.gap) lines.push("gap: " + c.gap + ";");
    if (c.alignItems) lines.push("align-items: " + c.alignItems + ";");
    if (c.justifyContent) lines.push("justify-content: " + c.justifyContent + ";");
    if (c.padding && c.padding !== "0px 0px 0px 0px") lines.push("padding: " + c.padding + ";");
  }

  // Background / fill
  if (detail.fills && detail.fills.length) {
    var f = detail.fills[0];
    if (f.type === "SOLID") {
      var bg = f.color;
      if (f.opacity !== undefined && f.opacity !== 1) {
        // Convert hex + opacity to rgba
        var r2 = parseInt(bg.slice(1, 3), 16);
        var g2 = parseInt(bg.slice(3, 5), 16);
        var b2 = parseInt(bg.slice(5, 7), 16);
        bg = "rgba(" + r2 + ", " + g2 + ", " + b2 + ", " + f.opacity + ")";
      }
      lines.push("background-color: " + bg + ";");
    } else if (f.type === "GRADIENT_LINEAR" && f.gradientStops) {
      var stops = f.gradientStops.map(function(s) { return s.color + " " + Math.round(s.position * 100) + "%"; }).join(", ");
      lines.push("background: linear-gradient(" + (f.gradientAngle || 0) + "deg, " + stops + ");");
    }
  }

  // Stroke / border
  if (detail.stroke) {
    if (typeof detail.strokeWeight === "number") {
      if (detail.strokeWeight > 0) {
        lines.push("border: " + detail.strokeWeight + "px solid " + detail.stroke + ";");
      }
    } else if (detail.strokeWeight && typeof detail.strokeWeight === "object") {
      var sw = detail.strokeWeight;
      if (sw.top) lines.push("border-top: " + sw.top + "px solid " + detail.stroke + ";");
      if (sw.right) lines.push("border-right: " + sw.right + "px solid " + detail.stroke + ";");
      if (sw.bottom) lines.push("border-bottom: " + sw.bottom + "px solid " + detail.stroke + ";");
      if (sw.left) lines.push("border-left: " + sw.left + "px solid " + detail.stroke + ";");
    } else if (detail.strokeWeight !== 0 && !isMixed(detail.strokeWeight)) {
      lines.push("border: 1px solid " + detail.stroke + ";");
    }
  }

  // Border radius
  if (detail.borderRadius) lines.push("border-radius: " + detail.borderRadius + ";");

  // Effects
  if (detail.boxShadow) lines.push("box-shadow: " + detail.boxShadow + ";");
  if (detail.filter) lines.push("filter: " + detail.filter + ";");
  if (detail.backdropFilter) lines.push("backdrop-filter: " + detail.backdropFilter + ";");

  // Opacity / blend
  if (detail.opacity !== undefined) lines.push("opacity: " + detail.opacity + ";");
  if (detail.blendMode) lines.push("mix-blend-mode: " + detail.blendMode.toLowerCase().replace(/_/g, "-") + ";");

  // Typography (TEXT nodes)
  if (detail.color) lines.push("color: " + detail.color + ";");
  if (detail.fontSize) lines.push("font-size: " + detail.fontSize + ";");
  if (detail.fontFamily) lines.push("font-family: \"" + detail.fontFamily + "\", sans-serif;");
  if (detail.fontWeight) {
    var weightMap = { "Thin": 100, "ExtraLight": 200, "Light": 300, "Regular": 400, "Medium": 500, "SemiBold": 600, "Bold": 700, "ExtraBold": 800, "Black": 900 };
    var wNum = weightMap[detail.fontWeight] || detail.fontWeight;
    lines.push("font-weight: " + wNum + ";");
  }
  if (detail.lineHeight) lines.push("line-height: " + detail.lineHeight + ";");
  if (detail.letterSpacing) lines.push("letter-spacing: " + detail.letterSpacing + ";");
  if (detail.textAlign) lines.push("text-align: " + detail.textAlign.toLowerCase() + ";");
  if (detail.textDecoration) lines.push("text-decoration: " + detail.textDecoration.toLowerCase() + ";");
  if (detail.textTransform) lines.push("text-transform: " + detail.textTransform + ";");

  // Rotation
  if (detail.rotation) lines.push("transform: rotate(" + detail.rotation + "deg);");

  // Overflow clip — reuse clipsContent resolved in get_node_detail (no second node fetch)
  if (detail.clipsContent) lines.push("overflow: hidden;");

  return {
    nodeId: detail.id,
    name: detail.name,
    type: detail.type,
    css: lines.join("\n"),
    // Also return structured detail for programmatic use
    detail: detail,
  };
};

// get_design_context — AI-optimized design-to-code payload for a node/selection
// Returns: flex layout semantics, token-resolved colors, typography, component instances
// Much more code-ready than get_design (raw tree) or get_css (single string)
handlers.get_design_context = async function(params) {
  var id = params ? (params.id || params.nodeId || params.node_id || params.targetId || params.target_id) : null;
  var nodeName = params ? (params.name || params.nodeName || params.node_name) : null;
  var node = null;
  if (id) node = await findNodeByIdAsync(id);
  if (!node && nodeName) node = findNodeByName(nodeName);
  if (!node) {
    // Fall back to current selection
    var sel = figma.currentPage.selection;
    node = sel && sel.length > 0 ? sel[0] : null;
  }
  if (!node) {
    var errMsg = (id || nodeName) && typeof getNodeNotFoundContext === "function"
      ? getNodeNotFoundContext(id, nodeName)
      : "No node specified and nothing selected. Pass nodeId or select a node in Figma.";
    throw new Error(errMsg);
  }

  // Build variable name lookup: variableId → name
  // Use getLocalVariablesAsync (single call) instead of per-ID fetches — avoids O(n²) await loop
  var varNameMap = {};
  try {
    var localVars = await figma.variables.getLocalVariablesAsync();
    for (var lvi = 0; lvi < localVars.length; lvi++) {
      var lv = localVars[lvi];
      if (lv) varNameMap[lv.id] = lv.name;
    }
  } catch(e) {}

  // Build style name lookup: styleId → name
  var styleNameMap = {};
  try {
    var pStyles = await figma.getLocalPaintStylesAsync();
    pStyles.forEach(function(s) { styleNameMap[s.id] = s.name; });
    var tStyles = await figma.getLocalTextStylesAsync();
    tStyles.forEach(function(s) { styleNameMap[s.id] = s.name; });
  } catch(e) {}

  // Resolve fill to token name or hex
  function resolveFill(nd) {
    if (!nd) return null;
    try {
      if (nd.fillStyleId && styleNameMap[nd.fillStyleId]) return "var(--" + styleNameMap[nd.fillStyleId].replace(/\//g, "-") + ")";
      if (nd.boundVariables && nd.boundVariables.fills) {
        var bvf = Array.isArray(nd.boundVariables.fills) ? nd.boundVariables.fills[0] : nd.boundVariables.fills;
        if (bvf && bvf.id && varNameMap[bvf.id]) return "var(--" + varNameMap[bvf.id].replace(/\//g, "-") + ")";
      }
      return getFillHex(nd);
    } catch(e) { return null; }
  }

  var ctxInstances = [];

  // Resolve a single node to its code-ready context shape
  function nodeContext(nd, depth) {
    if (!nd || nd.visible === false) return null;
    var ctx = { id: nd.id, name: nd.name, type: nd.type };

    // Layout
    try {
      if (nd.layoutMode && nd.layoutMode !== "NONE") {
        var alignMap = { "MIN": "flex-start", "CENTER": "center", "MAX": "flex-end", "SPACE_BETWEEN": "space-between" };
        ctx.layout = {
          display: "flex",
          flexDirection: nd.layoutMode === "HORIZONTAL" ? "row" : "column",
          gap: nd.itemSpacing + "px",
          alignItems: alignMap[nd.counterAxisAlignItems] || nd.counterAxisAlignItems,
          justifyContent: alignMap[nd.primaryAxisAlignItems] || nd.primaryAxisAlignItems,
          padding: nd.paddingTop + "px " + nd.paddingRight + "px " + nd.paddingBottom + "px " + nd.paddingLeft + "px",
          wrap: nd.layoutWrap === "WRAP" ? "wrap" : "nowrap",
        };
      }
    } catch(e) {}

    // Size
    try { ctx.size = { width: Math.round(nd.width), height: Math.round(nd.height) }; } catch(e) {}

    // Fill (token-resolved)
    var fillVal = resolveFill(nd);
    if (fillVal) ctx.fill = fillVal;
    if (nd.fills && !isMixed(nd.fills) && nd.fills.length && nd.fills[0].type === "SOLID" && nd.fills[0].opacity !== undefined && nd.fills[0].opacity !== 1) {
      ctx.fillOpacity = Math.round(nd.fills[0].opacity * 1000) / 1000;
    }

    // Direct token bindings
    try {
      if (nd.boundVariables) {
        var bvKeys = Object.keys(nd.boundVariables);
        for (var bvi = 0; bvi < bvKeys.length; bvi++) {
          var bvk = bvKeys[bvi];
          var binding = nd.boundVariables[bvk];
          var bindId = binding ? (Array.isArray(binding) ? (binding[0] ? binding[0].id : null) : binding.id) : null;
          if (bindId && varMap[bindId]) {
            var vName = varMap[bindId].name;
            if (bvk === "fills" || bvk === "fill") ctx.fillToken = vName;
            else if (bvk === "strokes" || bvk === "stroke") ctx.strokeToken = vName;
            else if (bvk === "itemSpacing") ctx.gapToken = vName;
            else if (bvk === "paddingTop" || bvk === "paddingBottom" || bvk === "paddingLeft" || bvk === "paddingRight") ctx.paddingToken = vName;
            else if (bvk === "topLeftRadius" || bvk === "cornerRadius") ctx.radiusToken = vName;
          }
        }
      }
    } catch(e) {}

    // Stroke
    try {
      if (nd.strokes && nd.strokes.length && nd.strokes[0].type === "SOLID") {
        var swWeight = nd.strokeWeight;
        if (isMixed(swWeight)) {
          var swSides = {};
          if (typeof nd.strokeTopWeight === "number") swSides.top = nd.strokeTopWeight;
          if (typeof nd.strokeRightWeight === "number") swSides.right = nd.strokeRightWeight;
          if (typeof nd.strokeBottomWeight === "number") swSides.bottom = nd.strokeBottomWeight;
          if (typeof nd.strokeLeftWeight === "number") swSides.left = nd.strokeLeftWeight;
          swWeight = Object.keys(swSides).length ? swSides : "mixed";
        }
        ctx.stroke = { color: rgbToHex(nd.strokes[0].color, nd.strokes[0].opacity), weight: swWeight };
        if (nd.strokes[0].opacity !== undefined && nd.strokes[0].opacity !== 1) {
          ctx.stroke.opacity = Math.round(nd.strokes[0].opacity * 1000) / 1000;
        }
      }
    } catch(e) {}

    // Corner radius
    try {
      if ("cornerRadius" in nd && nd.cornerRadius) ctx.borderRadius = nd.cornerRadius + "px";
    } catch(e) {}

    // Opacity / effects
    try { if (nd.opacity !== undefined && nd.opacity !== 1) ctx.opacity = nd.opacity; } catch(e) {}
    try {
      if (nd.effects && nd.effects.length) {
        ctx.effects = nd.effects.filter(function(e) { return e.visible !== false; }).map(function(e) {
          var ed = { type: e.type };
          if (e.color) ed.color = "rgba(" + Math.round(e.color.r*255) + "," + Math.round(e.color.g*255) + "," + Math.round(e.color.b*255) + "," + Math.round((e.color.a||1)*100)/100 + ")";
          if (e.offset) { ed.offsetX = e.offset.x; ed.offsetY = e.offset.y; }
          if (e.radius) ed.radius = e.radius;
          return ed;
        });
      }
    } catch(e) {}

    // Text — resolveTextStyle unwraps figma.mixed on multi-style text
    if (nd.type === "TEXT") {
      var ndText = resolveTextStyle(nd);
      if (ndText) {
        ctx.text = { content: ndText.content };
        if (nd.textStyleId && styleNameMap[nd.textStyleId]) ctx.text.style = styleNameMap[nd.textStyleId];
        if (ndText.fontSize !== undefined) ctx.text.fontSize = ndText.fontSize;
        ctx.text.fontFamily = ndText.fontFamily || null;
        ctx.text.fontWeight = ndText.fontWeight || null;
        if (ndText.lineHeight !== undefined) {
          ctx.text.lineHeight = typeof ndText.lineHeight === "number" ? ndText.lineHeight + "px" : ndText.lineHeight;
        }
        // Token-resolved colour wins over the raw hex from the segments.
        ctx.text.color = resolveFill(nd) || ndText.fill || null;
        if (ndText.textCase) ctx.text.textCase = ndText.textCase;
        if (ndText.textTransform) ctx.text.textTransform = ndText.textTransform;
        if (ndText.renderedContent) ctx.text.renderedContent = ndText.renderedContent;
        if (ndText.textDecoration) ctx.text.textDecoration = ndText.textDecoration;
        if (ndText.mixed) {
          ctx.text.mixedStyles = true;
          if (ndText.segments) ctx.text.segments = ndText.segments;
        }
        try { ctx.text.align = nd.textAlignHorizontal ? nd.textAlignHorizontal.toLowerCase() : null; } catch(e) {}
      }
    }

    // Component instance — mainComponent is async-only under
    // documentAccess: dynamic-page, so collect now and resolve after the walk.
    if (nd.type === "INSTANCE") {
      ctxInstances.push({ info: ctx, node: nd });
      var parsedProps = typeof cleanComponentProperties === "function" ? cleanComponentProperties(nd) : null;
      if (parsedProps) {
        if (parsedProps.variant) ctx.variant = parsedProps.variant;
        if (parsedProps.props) ctx.props = parsedProps.props;
      }
    }

    // Role
    var sRole = typeof inferSemanticRole === "function" ? inferSemanticRole(nd, ctx) : null;
    if (sRole) ctx.role = sRole;

    // Children (limited depth to avoid token overflow)
    if (depth < 4 && nd.children && nd.children.length) {
      var rawCtxChildren = [];
      for (var i = 0; i < nd.children.length; i++) {
        var child = nodeContext(nd.children[i], depth + 1);
        if (child) rawCtxChildren.push(child);
      }
      ctx.children = typeof aggregateRepeatedChildren === "function" ? aggregateRepeatedChildren(rawCtxChildren) : rawCtxChildren;
    } else if (nd.children && nd.children.length) {
      ctx.childCount = nd.children.length;
    }

    return ctx;
  }

  var context = nodeContext(node, 0);
  // { name, set, variant } shape — kept distinct from the tree walkers' fields.
  await resolveInstanceComponents(ctxInstances, function(target, desc) {
    var comp = target.component || {};
    comp.name = desc.componentName;
    if (desc.componentSetName) comp.set = desc.componentSetName;
    if (desc.variantLabel) comp.variant = desc.variantLabel;
    target.component = comp;
  });

  // Summary tokens used in this subtree (for code scaffolding)
  // Use plain objects instead of Set — Figma sandbox ES5 compatibility
  var usedColors = {}, usedTextStyles = {}, usedComponents = {};
  function collectUsed(nd) {
    if (!nd) return;
    if (nd.fill && nd.fill.indexOf("var(--") === 0) usedColors[nd.fill] = 1;
    if (nd.text && nd.text.style) usedTextStyles[nd.text.style] = 1;
    if (nd.component && (nd.component.set || nd.component.name)) {
      usedComponents[nd.component.set || nd.component.name] = 1;
    }
    if (nd.children) {
      for (var cui = 0; cui < nd.children.length; cui++) collectUsed(nd.children[cui]);
    }
  }
  collectUsed(context);

  return {
    nodeId: node.id,
    name: node.name,
    type: node.type,
    context: context,
    summary: {
      tokensUsed: Object.keys(usedColors),
      textStylesUsed: Object.keys(usedTextStyles),
      componentsUsed: Object.keys(usedComponents),
    },
    hint: "Use context.layout for flex CSS, context.fill for token-resolved colors, context.component for React component mapping, context.text.style for typography class names.",
  };
};

// Aliases for developer convenience & MCP tool naming compatibility
handlers.inspect_node = handlers.get_design_context;
handlers.inspectNode = handlers.get_design_context;
handlers.inspect = handlers.get_design_context;
handlers.getDesignContext = handlers.get_design_context;
handlers.designContext = handlers.get_design_context;
handlers.getCss = handlers.get_css;
handlers.css = handlers.get_css;
handlers.get_node = handlers.get_node_detail;
handlers.getNode = handlers.get_node_detail;
handlers.get_node_detail = handlers.get_node_detail;
handlers.getNodeDetail = handlers.get_node_detail;
handlers.get_node_info = handlers.get_node_detail;
handlers.getNodeInfo = handlers.get_node_detail;
handlers.node_detail = handlers.get_node_detail;
handlers.nodeDetail = handlers.get_node_detail;
handlers.node_info = handlers.get_node_detail;
handlers.nodeInfo = handlers.get_node_detail;


// get_component_map — list every component instance in a frame with variant properties
// Use this to map Figma components to their code equivalents
handlers.get_component_map = async function(params) {
  var id = params ? (params.id || params.nodeId) : null;
  var nodeName = params ? (params.name || params.nodeName) : null;
  var node = null;
  if (id) node = await findNodeByIdAsync(id);
  if (!node && nodeName) node = findNodeByName(nodeName);
  if (!node) {
    var sel = figma.currentPage.selection;
    node = sel && sel.length > 0 ? sel[0] : figma.currentPage;
  }

  // Find all INSTANCE nodes in subtree
  var instances = [];
  var mapInstances = [];
  function walkInstances(nd) {
    if (!nd) return;
    if (nd.type === "INSTANCE") {
      var entry = { id: nd.id, name: nd.name, x: Math.round(nd.x), y: Math.round(nd.y), width: Math.round(nd.width), height: Math.round(nd.height) };
      mapInstances.push({ info: entry, node: nd });
      try {
        if (nd.componentProperties) {
          var props = {};
          var pks = Object.keys(nd.componentProperties);
          for (var pi = 0; pi < pks.length; pi++) props[pks[pi]] = nd.componentProperties[pks[pi]].value;
          if (Object.keys(props).length > 0) entry.properties = props;
        }
      } catch(e) {}
      instances.push(entry);
    }
    if (nd.children) nd.children.forEach(walkInstances);
  }
  walkInstances(node);
  var mapInfo = await resolveInstanceComponents(mapInstances);

  // Suggested import path based on component name convention — computed after
  // the async component resolution, otherwise there is no name to derive from.
  instances.forEach(function(entry) {
    var cname = entry.componentSetName || entry.componentName || "";
    var cnameLast = cname ? cname.split("/").slice(-1)[0].replace(/[^a-zA-Z0-9]/g, "") : "";
    entry.suggestedImport = cname ? "import { " + cnameLast + " } from '@/components/" + cnameLast + "'" : null;
  });

  // Deduplicate by componentName for summary
  var uniqueComponents = {};
  instances.forEach(function(inst) {
    var key = inst.componentSetName || inst.componentName || inst.name;
    if (key && !uniqueComponents[key]) uniqueComponents[key] = { name: key, count: 0, suggestedImport: inst.suggestedImport };
    if (key) uniqueComponents[key].count++;
  });

  return {
    frameId: node.id,
    frameName: node.name,
    totalInstances: instances.length,
    resolvedInstances: mapInfo.resolved,
    instancesTruncated: mapInfo.truncated || undefined,
    instances: instances,
    uniqueComponents: Object.values(uniqueComponents),
    hint: "suggestedImport is a best-guess based on component name. Adjust the import path to match your actual codebase structure.",
  };
};

// get_unmapped_components — find component instances with no description (likely no code mapping)
// Helps AI ask: "do you have a Button component in your codebase?"
handlers.get_unmapped_components = async function(params) {
  var mapResult = await handlers.get_component_map(params);
  var allLocalComps = await handlers.get_local_components();

  // Build set of components that have descriptions (likely mapped)
  var described = new Set();
  allLocalComps.components.forEach(function(c) {
    if (!c.description || !c.description.trim()) return;
    described.add(c.name);
    // get_component_map keys variants by their set name — map that too.
    if (c.setName) described.add(c.setName);
  });
  allLocalComps.componentSets.forEach(function(s) { if (s.description && s.description.trim()) described.add(s.name); });

  // Filter to only unique components that are used in frame but have no description
  var unmapped = mapResult.uniqueComponents.filter(function(uc) { return !described.has(uc.name); });
  var mapped = mapResult.uniqueComponents.filter(function(uc) { return described.has(uc.name); });

  return {
    frameId: mapResult.frameId,
    frameName: mapResult.frameName,
    totalUsed: mapResult.uniqueComponents.length,
    unmapped: unmapped.map(function(u) { return { name: u.name, count: u.count, suggestedImport: u.suggestedImport }; }),
    mapped: mapped.map(function(u) { return { name: u.name, count: u.count }; }),
    hint: unmapped.length > 0
      ? "These " + unmapped.length + " components have no description. Add a code import path to each component's description in Figma (e.g. '@/components/Button') so get_component_map can suggest accurate imports."
      : "All components in this frame have descriptions — fully mapped.",
  };
};

// get_styles — read all local paint, text, effect, and grid styles
handlers.get_styles = async function() {
  var paintStyles = await figma.getLocalPaintStylesAsync();
  var textStyles = await figma.getLocalTextStylesAsync();
  var effectStyles = await figma.getLocalEffectStylesAsync();
  var gridStyles = await figma.getLocalGridStylesAsync();

  return {
    paintStyles: paintStyles.map(function(s) {
      var paints = s.paints || [];
      var hex = null;
      if (paints.length > 0 && paints[0].type === "SOLID") {
        hex = rgbToHex(paints[0].color, paints[0].opacity);
      }
      return { id: s.id, name: s.name, hex: hex, type: "PAINT" };
    }),
    textStyles: textStyles.map(function(s) {
      return {
        id: s.id, name: s.name, type: "TEXT",
        fontSize: s.fontSize,
        fontFamily: s.fontName ? s.fontName.family : null,
        fontWeight: s.fontName ? s.fontName.style : null,
        lineHeight: s.lineHeight ? s.lineHeight.value : null,
        letterSpacing: s.letterSpacing ? s.letterSpacing.value : null,
      };
    }),
    effectStyles: effectStyles.map(function(s) {
      return { id: s.id, name: s.name, type: "EFFECT", effects: s.effects.length };
    }),
    gridStyles: gridStyles.map(function(s) {
      return { id: s.id, name: s.name, type: "GRID" };
    }),
  };
};

// get_local_components — enhanced component listing with descriptions and properties
handlers.get_local_components = async function() {
  if (typeof figma.loadAllPagesAsync === "function") {
    try { await figma.loadAllPagesAsync(); } catch (e) {}
  }
  var comps = figma.root.findAllWithCriteria({ types: ["COMPONENT"] });
  if (typeof yieldToUI === "function") await yieldToUI(0);
  var sets = figma.root.findAllWithCriteria({ types: ["COMPONENT_SET"] });
  if (typeof yieldToUI === "function") await yieldToUI(0);

  var componentList = [];
  var COMP_BATCH_SIZE = 50;
  for (var ci = 0; ci < comps.length; ci++) {
    var c = comps[ci];
    var info = {
      id: c.id, name: c.name, key: c.key || null,
      description: c.description || "",
      setName: (c.parent && c.parent.type === "COMPONENT_SET") ? c.parent.name : null,
      width: Math.round(c.width), height: Math.round(c.height),
      page: c.parent ? (function findPage(n) {
        while (n && n.type !== "PAGE") n = n.parent;
        return n ? n.name : null;
      })(c) : null,
    };
    // Component properties (variant props)
    try {
      if (c.componentPropertyDefinitions) {
        var defs = c.componentPropertyDefinitions;
        var props = {};
        for (var key in defs) {
          if (Object.prototype.hasOwnProperty.call(defs, key)) {
            props[key] = { type: defs[key].type, defaultValue: defs[key].defaultValue };
            if (defs[key].variantOptions) props[key].options = defs[key].variantOptions;
          }
        }
        info.properties = props;
      }
    } catch(e) { /* skip properties */ }
    componentList.push(info);

    if (ci > 0 && ci % COMP_BATCH_SIZE === 0 && typeof yieldToUI === "function") {
      await yieldToUI(0);
    }
  }

  var componentSets = [];
  for (var si = 0; si < sets.length; si++) {
    var s = sets[si];
    componentSets.push({
      id: s.id, name: s.name, key: s.key || null,
      description: s.description || "",
      variantCount: s.children ? s.children.length : 0,
    });
    if (si > 0 && si % COMP_BATCH_SIZE === 0 && typeof yieldToUI === "function") {
      await yieldToUI(0);
    }
  }

  return {
    components: componentList,
    componentSets: componentSets,
    total: componentList.length + componentSets.length,
  };
};

// get_viewport — current viewport position and zoom
handlers.get_viewport = async function() {
  var vp = figma.viewport;
  return {
    center: { x: Math.round(vp.center.x), y: Math.round(vp.center.y) },
    zoom: vp.zoom,
    bounds: vp.bounds ? {
      x: Math.round(vp.bounds.x), y: Math.round(vp.bounds.y),
      width: Math.round(vp.bounds.width), height: Math.round(vp.bounds.height),
    } : null,
  };
};

// set_viewport — navigate to specific area
handlers.set_viewport = async function(params) {
  if (params.nodeId || params.nodeName) {
    // Zoom to fit a specific node
    var node = params.nodeId ? (await findNodeByIdAsync(params.nodeId)) : findNodeByName(params.nodeName);
    if (!node) throw new Error("Node not found for viewport navigation");
    figma.viewport.scrollAndZoomIntoView([node]);
    return { scrolledTo: node.id, name: node.name };
  }
  if (params.center) {
    figma.viewport.center = { x: params.center.x, y: params.center.y };
  }
  if (params.zoom !== undefined) {
    figma.viewport.zoom = params.zoom;
  }
  return {
    center: { x: Math.round(figma.viewport.center.x), y: Math.round(figma.viewport.center.y) },
    zoom: figma.viewport.zoom,
  };
};

// get_variables — read Figma local variables (Design Tokens)
handlers.get_variables = async function() {
  var collections = [];
  try {
    var localCollections = await figma.variables.getLocalVariableCollectionsAsync();

    // One bulk fetch instead of an await per variable id — a design system with
    // a few hundred tokens otherwise pays that many sequential round-trips.
    var varsById = {};
    var bulkLoaded = false;
    try {
      if (typeof figma.variables.getLocalVariablesAsync === "function") {
        var allVars = await figma.variables.getLocalVariablesAsync();
        for (var avi = 0; avi < allVars.length; avi++) {
          if (allVars[avi]) varsById[allVars[avi].id] = allVars[avi];
        }
        bulkLoaded = true;
      }
    } catch(e) { /* fall back to per-id lookups */ }

    var resolvedTokensMap = {};

    for (var ci = 0; ci < localCollections.length; ci++) {
      var col = localCollections[ci];
      var variables = [];
      for (var vi = 0; vi < col.variableIds.length; vi++) {
        var vid = col.variableIds[vi];
        var v = bulkLoaded ? varsById[vid] : await getVariableSafeAsync(vid);
        if (!v) continue;
        var values = {};
        for (var modeId in v.valuesByMode) {
          if (Object.prototype.hasOwnProperty.call(v.valuesByMode, modeId)) {
            var val = v.valuesByMode[modeId];
            if (val && typeof val === "object" && "r" in val && "g" in val && "b" in val) {
              var hexVal = rgbToHex(val, val.a);
              values[modeId] = hexVal;
              resolvedTokensMap[v.name] = hexVal;
            } else if (val && typeof val === "object" && val.type === "VARIABLE_ALIAS" && val.id) {
              var resolvedAlias = await resolveVariableValueAsync(val.id, modeId, 1);
              var finalHexOrVal = resolvedAlias ? (resolvedAlias.resolvedValue || resolvedAlias.hex || resolvedAlias.value) : null;
              values[modeId] = {
                type: "VARIABLE_ALIAS",
                aliasId: val.id,
                aliasName: resolvedAlias ? resolvedAlias.name : null,
                primitiveName: resolvedAlias ? resolvedAlias.primitiveName : null,
                resolvedValue: finalHexOrVal,
              };
              if (finalHexOrVal) resolvedTokensMap[v.name] = finalHexOrVal;
            } else {
              values[modeId] = val;
              if (val !== undefined && val !== null) resolvedTokensMap[v.name] = val;
            }
          }
        }
        variables.push({
          id: v.id, name: v.name,
          resolvedType: v.resolvedType,
          values: values,
          description: v.description || "",
        });

        if (vi > 0 && vi % 50 === 0 && typeof yieldToUI === "function") {
          await yieldToUI(0);
        }
      }
      collections.push({
        id: col.id, name: col.name,
        modes: col.modes.map(function(m) { return { id: m.modeId, name: m.name }; }),
        variables: variables,
      });
      if (typeof yieldToUI === "function") await yieldToUI(0);
    }
  } catch(e) {
    return { error: "Variables API not available: " + e.message, collections: [] };
  }
  return { collections: collections, resolvedTokens: resolvedTokensMap };
};

handlers.get_variable_tokens = handlers.get_variables;
handlers.getVariableTokens = handlers.get_variables;
handlers.get_tokens = handlers.get_variables;
handlers.getTokens = handlers.get_variables;
handlers.tokens = handlers.get_variables;
handlers.variables = handlers.get_variables;
handlers.getVariables = handlers.get_variables;
handlers.getStyles = handlers.get_styles;
handlers.styles = handlers.get_styles;
handlers.getLocalComponents = handlers.get_local_components;
handlers.localComponents = handlers.get_local_components;
handlers.getViewport = handlers.get_viewport;
handlers.setViewport = handlers.set_viewport;
handlers.getComponentMap = handlers.get_component_map;
handlers.getUnmappedComponents = handlers.get_unmapped_components;


// ─── EXPORT ASSETS (Batch Icon & Image Extractor) ────────────────────────────
handlers.export_assets = async function(params) {
  var id = params ? (params.id || params.nodeId) : null;
  var targetNode = null;
  if (id) targetNode = await findNodeByIdAsync(id);
  if (!targetNode) {
    var sel = figma.currentPage.selection;
    targetNode = sel && sel.length > 0 ? sel[0] : figma.currentPage;
  }
  if (!targetNode) throw new Error("No node found to export assets from");

  var icons = [];
  var images = [];
  var seenNames = {};

  function sanitizeAssetName(rawName, fallback) {
    var name = (rawName || fallback)
      .toLowerCase()
      .replace(/^icon[s\/_\-\s]+/, '')
      .replace(/^ic[_\-\s]+/, '')
      .replace(/[\/\s_\.]+/g, '-')
      .replace(/[^a-z0-9\-]/g, '')
      .replace(/^-+|-+$/g, '');
    if (!name) name = fallback;
    if (seenNames[name]) {
      seenNames[name]++;
      return name + "-" + seenNames[name];
    }
    seenNames[name] = 1;
    return name;
  }

  function isVectorIcon(nd) {
    if (!nd || nd.visible === false) return false;
    var t = nd.type;
    if (t === "VECTOR" || t === "BOOLEAN_OPERATION" || t === "STAR" || t === "POLYGON") return true;
    if ((t === "FRAME" || t === "COMPONENT" || t === "INSTANCE" || t === "GROUP") && nd.width <= 96 && nd.height <= 96) {
      var n = nd.name.toLowerCase();
      if (n.indexOf("icon") >= 0 || n.indexOf("ic_") >= 0 || n.indexOf("ic-") >= 0) return true;
      if (nd.children && nd.children.length > 0 && nd.children.length <= 8) {
        var allVectors = nd.children.every(function(c) {
          return c.type === "VECTOR" || c.type === "BOOLEAN_OPERATION" || c.type === "LINE" || c.type === "ELLIPSE" || c.type === "RECTANGLE";
        });
        if (allVectors) return true;
      }
    }
    return false;
  }

  function hasImageFill(nd) {
    if (!nd || !nd.fills || !Array.isArray(nd.fills)) return false;
    return nd.fills.some(function(f) { return f && f.type === "IMAGE" && f.visible !== false; });
  }

  var nodesToInspect = [];
  function collectAssetNodes(nd, depth) {
    if (!nd || nd.visible === false || depth > 20) return;
    if (isVectorIcon(nd)) {
      nodesToInspect.push({ node: nd, kind: "icon" });
      return;
    }
    if (hasImageFill(nd)) {
      nodesToInspect.push({ node: nd, kind: "image" });
    }
    if (nd.children && Array.isArray(nd.children)) {
      for (var ci = 0; ci < nd.children.length; ci++) {
        collectAssetNodes(nd.children[ci], depth + 1);
      }
    }
  }

  if (targetNode === figma.currentPage) {
    for (var pi = 0; pi < targetNode.children.length; pi++) {
      collectAssetNodes(targetNode.children[pi], 0);
    }
  } else {
    collectAssetNodes(targetNode, 0);
  }

  var exportLimit = Math.min(nodesToInspect.length, 60);
  for (var i = 0; i < exportLimit; i++) {
    var item = nodesToInspect[i];
    var nd = item.node;
    var baseName = sanitizeAssetName(nd.name, "asset-" + (i + 1));

    if (item.kind === "icon") {
      try {
        var svgBytes = await nd.exportAsync({ format: "SVG" });
        var svgStr = String.fromCharCode.apply(null, Array.from(svgBytes));
        icons.push({
          id: nd.id,
          name: baseName,
          fileName: baseName + ".svg",
          width: Math.round(nd.width),
          height: Math.round(nd.height),
          svg: svgStr
        });
      } catch (e) {}
    } else if (item.kind === "image") {
      try {
        var pngBytes = await nd.exportAsync({ format: "PNG", constraint: { type: "SCALE", value: 2 } });
        var b64 = uint8ArrayToBase64(pngBytes);
        images.push({
          id: nd.id,
          name: baseName,
          fileName: baseName + ".png",
          width: Math.round(nd.width),
          height: Math.round(nd.height),
          dataUrl: "data:image/png;base64," + b64
        });
      } catch (e) {}
    }
  }

  return {
    success: true,
    sourceNodeId: targetNode.id,
    sourceNodeName: targetNode.name,
    totalIcons: icons.length,
    totalImages: images.length,
    icons: icons,
    images: images
  };
};

handlers.exportAssets = handlers.export_assets;

