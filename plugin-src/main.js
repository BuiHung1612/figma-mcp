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

// Serialize the handler result ONCE, here in the main thread, and ship the
// JSON string to the UI. The UI forwards that string straight into the HTTP /
// WebSocket body, so a big design tree is stringified a single time instead of
// stringify → parse (sanitize) → structured clone → stringify (transport).
// Symbol values (figma.mixed) can't be cloned or serialized — replace them.
function bridgeReplacer(_key, value) {
  return typeof value === "symbol" ? "mixed" : value;
}

function stringifyForBridge(data) {
  if (data === undefined) return "null";
  try {
    var json = JSON.stringify(data, bridgeReplacer);
    return json === undefined ? "null" : json;
  } catch (e) {
    return JSON.stringify({
      error: "Result could not be serialized: " + (e && e.message ? e.message : String(e)),
    });
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
    figma.ui.postMessage({ id: id, operation: operation, success: true, dataJson: stringifyForBridge(data) });
  } catch (err) {
    var errMsg = "[dispatch:" + operation + "] " + (err && err.message ? err.message : String(err));
    figma.ui.postMessage({ id: id, operation: operation, success: false, error: errMsg });
  }
};

