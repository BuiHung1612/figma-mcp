// ─── PLUGIN ENTRY POINT ───────────────────────────────────────────────────────

figma.showUI(__html__, { width: 320, height: 420, title: "Figma MCP Bridge" });

// Restore saved window size if user previously resized
figma.clientStorage.getAsync("mcp_window_size").then(function(saved) {
  if (saved && saved.width && saved.height) {
    try {
      figma.ui.resize(
        Math.max(260, Math.min(1000, saved.width)),
        Math.max(200, Math.min(1200, saved.height))
      );
    } catch(e) {}
  }
}).catch(function() {});

// Broadcast active file / session metadata on startup
try {
  figma.ui.postMessage({
    type: "session-info",
    sessionId: figma.root.id,
    fileName: figma.root.name
  });
} catch (e) {}

// Broadcast selection changes live to UI
figma.on("selectionchange", function() {
  try {
    var sel = figma.currentPage.selection;
    var summary = [];
    for (var i = 0; i < Math.min(sel.length, 5); i++) {
      summary.push({ id: sel[i].id, name: sel[i].name, type: sel[i].type });
    }
    figma.ui.postMessage({
      type: "selection-change",
      count: sel.length,
      selection: summary
    });
  } catch (e) {}
});

// ─── DISPATCHER ───────────────────────────────────────────────────────────────

// Sanitize data before postMessage — remove Symbol values (e.g. figma.mixed)
// that cannot be serialized via structured clone / JSON.
// Uses fast native JSON serialization with replacer instead of slow recursive JS allocations.
function sanitizeForPostMessage(obj) {
  if (obj === null || obj === undefined) return obj;
  var t = typeof obj;
  if (t === "string" || t === "number" || t === "boolean") return obj;
  if (t === "symbol") return "mixed";
  try {
    return JSON.parse(JSON.stringify(obj, function(_k, v) {
      if (typeof v === "symbol") return "mixed";
      return v;
    }));
  } catch(e) {
    return obj;
  }
}

figma.ui.onmessage = async (request) => {
  if (!request) return;

  // Handle window resizing from UI drag handle
  if (request.type === "resize") {
    var newW = Math.max(260, Math.min(1000, Math.round(request.width)));
    var newH = Math.max(200, Math.min(1200, Math.round(request.height)));
    try {
      figma.ui.resize(newW, newH);
      figma.clientStorage.setAsync("mcp_window_size", { width: newW, height: newH }).catch(function() {});
    } catch(e) {}
    return;
  }

  const { id, operation, params } = request;
  const handler = handlers[operation];

  if (!handler) {
    figma.ui.postMessage({
      id, operation, success: false,
      error: `Unknown operation "${operation}". Available: ${Object.keys(handlers).join(", ")}`,
    });
    return;
  }

  try {
    var data = await handler(params || {});
    figma.ui.postMessage({ id: id, operation: operation, success: true, data: sanitizeForPostMessage(data) });
  } catch (err) {
    var errMsg = "[dispatch:" + operation + "] " + (err && err.message ? err.message : String(err));
    figma.ui.postMessage({ id: id, operation: operation, success: false, error: errMsg });
  }
};

