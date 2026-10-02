// Canonical numeric data is retained alongside legacy CSS fields. Never re-parse
// a serialized color to recover alpha, paint geometry, or effect parameters.
function parseColorValue(value) {
  if (value && typeof value === "object") {
    var components = [value.r, value.g, value.b, value.a === undefined ? 1 : value.a];
    if (components.some(function(v) { return typeof v !== "number" || !Number.isFinite(v) || v < 0 || v > 1; })) {
      throw new Error("Invalid normalized RGBA color");
    }
    return { r: components[0], g: components[1], b: components[2], a: components[3] };
  }
  var s = String(value || "").trim().toLowerCase();
  if (s === "none" || s === "transparent") return { r: 0, g: 0, b: 0, a: 0 };
  if (CSS_COLOR_MAP[s]) s = CSS_COLOR_MAP[s].toLowerCase();
  if (/^(?:[a-f0-9]{3}|[a-f0-9]{4}|[a-f0-9]{6}|[a-f0-9]{8})$/.test(s)) s = "#" + s;
  // Figma's parser supports CSS color syntax and the full named-color table.
  if (typeof figma !== "undefined" && figma.util && typeof figma.util.solidPaint === "function") {
    var paint = figma.util.solidPaint(s);
    return parseColorValue({ r: paint.color.r, g: paint.color.g, b: paint.color.b, a: paint.opacity === undefined ? 1 : paint.opacity });
  }
  var h = s.replace(/^#/, "");
  if (/^(?:[a-f0-9]{3}|[a-f0-9]{4}|[a-f0-9]{6}|[a-f0-9]{8})$/.test(h)) {
    if (h.length <= 4) h = h.split("").map(function(c) { return c + c; }).join("");
    return { r: parseInt(h.slice(0, 2), 16) / 255, g: parseInt(h.slice(2, 4), 16) / 255,
      b: parseInt(h.slice(4, 6), 16) / 255, a: h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1 };
  }
  var match = s.match(/^(rgba?|hsla?)\(([^)]+)\)$/);
  if (!match) throw new Error("Unsupported color format: " + value);
  var parts = match[2].trim().split(/[\s,\/]+/);
  if (parts.length !== 3 && parts.length !== 4) throw new Error("Invalid color: " + value);
  function channel(v, scale) {
    if (!/^[+-]?(?:\d*\.)?\d+%?$/.test(v)) throw new Error("Invalid color component: " + v);
    return Math.max(0, Math.min(1, parseFloat(v) / (v.endsWith("%") ? 100 : scale)));
  }
  var alpha = parts.length === 4 ? channel(parts[3], 1) : 1;
  if (match[1].startsWith("rgb")) return { r: channel(parts[0], 255), g: channel(parts[1], 255), b: channel(parts[2], 255), a: alpha };
  var hue = parts[0];
  if (!/^[+-]?(?:\d*\.)?\d+(?:deg|rad|turn|grad)?$/.test(hue) || !parts[1].endsWith("%") || !parts[2].endsWith("%")) throw new Error("Invalid HSL color");
  var degrees = parseFloat(hue) * (hue.endsWith("turn") ? 360 : hue.endsWith("grad") ? 0.9 : hue.endsWith("rad") ? 180 / Math.PI : 1);
  var h6 = ((degrees % 360) + 360) % 360 / 60;
  var sat = channel(parts[1], 1), light = channel(parts[2], 1);
  var chroma = (1 - Math.abs(2 * light - 1)) * sat, x = chroma * (1 - Math.abs(h6 % 2 - 1));
  var rgb = h6 < 1 ? [chroma, x, 0] : h6 < 2 ? [x, chroma, 0] : h6 < 3 ? [0, chroma, x] : h6 < 4 ? [0, x, chroma] : h6 < 5 ? [x, 0, chroma] : [chroma, 0, x];
  var m = light - chroma / 2;
  return { r: rgb[0] + m, g: rgb[1] + m, b: rgb[2] + m, a: alpha };
}

function serializePaint(paint) {
  var data = JSON.parse(JSON.stringify(paint));
  if (paint.type === "SOLID") {
    data.rgba = parseColorValue(paint.color);
    data.color = colorToCss(data.rgba, data.rgba.a * (paint.opacity === undefined ? 1 : paint.opacity));
    data.alphaIncluded = true;
  } else if (paint.gradientStops) {
    data.gradientStops = paint.gradientStops.map(function(stop) {
      var rgba = parseColorValue(stop.color);
      return { position: stop.position, rgba: rgba, color: colorToCss(rgba), boundVariables: stop.boundVariables };
    });
  }
  return data;
}

function paintToCss(paint, width, height) {
  if (paint.visible === false) return null;
  if (paint.blendMode && paint.blendMode !== "NORMAL") throw new Error("Paint blend mode requires compositing: " + paint.blendMode);
  var opacity = paint.opacity === undefined ? 1 : paint.opacity;
  if (paint.type === "SOLID") {
    var c = paint.rgba ? parseColorValue(paint.rgba) : parseColorValue(paint.color);
    return colorToCss(c, c.a * (paint.alphaIncluded && !paint.rgba ? 1 : opacity));
  }
  if (paint.type !== "GRADIENT_LINEAR") throw new Error("Export SVG for paint " + paint.type + "; CSS conversion is not exact");
  var gt = paint.gradientTransform;
  if (!gt || !width || !height || !paint.gradientStops || !paint.gradientStops.length) throw new Error("Linear gradient requires transform, stops and target dimensions");
  var a = gt[0][0], b = gt[0][1], c0 = gt[0][2];
  var qx = a / width, qy = b / height, len = Math.hypot(qx, qy);
  if (!Number.isFinite(len) || len === 0) throw new Error("Invalid gradient transform");
  var nx = qx / len, ny = qy / len;
  var span = Math.abs(nx) * width + Math.abs(ny) * height;
  var min = c0 + Math.min(0, a) + Math.min(0, b);
  var angle = ((Math.atan2(nx, -ny) * 180 / Math.PI) % 360 + 360) % 360;
  var stops = paint.gradientStops.map(function(stop) {
    var color = parseColorValue(stop.rgba || stop.color);
    return colorToCss(color, color.a * opacity) + " " + ((stop.position - min) / (len * span) * 100) + "%";
  });
  return "linear-gradient(" + angle + "deg, " + stops.join(", ") + ")";
}

function paintsToCss(paints, width, height) {
  var visible = (paints || []).filter(function(p) { return p.visible !== false; });
  if (visible.length === 1 && visible[0].type === "SOLID") return { property: "background-color", value: paintToCss(visible[0], width, height) };
  var layers = visible.map(function(p) {
    var css = paintToCss(p, width, height);
    return p.type === "SOLID" ? "linear-gradient(" + css + ", " + css + ")" : css;
  });
  // Both paint arrays and CSS background images list the top layer first.
  return layers.length ? { property: "background", value: layers.join(", ") } : null;
}

function effectsToCss(effects) {
  var shadows = [], filters = [], backdrops = [];
  (effects || []).forEach(function(e) {
    if (e.visible === false) return;
    if (e.blendMode && e.blendMode !== "NORMAL") throw new Error("Effect blend mode requires SVG: " + e.blendMode);
    if (e.type === "DROP_SHADOW" || e.type === "INNER_SHADOW") {
      if (!e.color || !e.offset || typeof e.radius !== "number") throw new Error("Incomplete shadow data");
      shadows.push((e.type === "INNER_SHADOW" ? "inset " : "") + e.offset.x + "px " + e.offset.y + "px " + e.radius + "px " + (e.spread || 0) + "px " + colorToCss(parseColorValue(e.color)));
    } else if (e.type === "LAYER_BLUR" || e.type === "BACKGROUND_BLUR") {
      if (e.blurType && e.blurType !== "NORMAL") throw new Error("Progressive blur requires SVG");
      (e.type === "LAYER_BLUR" ? filters : backdrops).push("blur(" + e.radius + "px)");
    } else throw new Error("Unsupported effect: " + e.type);
  });
  return { boxShadow: shadows.join(", "), filter: filters.join(" "), backdropFilter: backdrops.join(" ") };
}
