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

// ─── UNIQUE SESSION IDENTIFIER (Multi-tab Support) ───────────────────────────
// In Figma, figma.root.id is always "0:0" across all files. To support running
// multiple plugins concurrently across multiple tabs/files, generate and persist
// a unique document session ID in document pluginData.
function getSavedFileKey() {
  if (figma.fileKey) return figma.fileKey;
  try {
    var saved = figma.root.getPluginData("figma_file_key");
    if (saved && saved.length > 0) return saved;
  } catch(e) {}
  return null;
}

function getOrCreateSessionId() {
  var fk = getSavedFileKey();
  if (fk) {
    return fk;
  }
  try {
    var stored = figma.root.getPluginData("mcp_session_id");
    if (stored && stored.length > 0) {
      return stored;
    }
  } catch(e) {}

  var rand = Math.random().toString(36).substring(2, 10);
  var ts = Date.now().toString(36);
  var newId = "doc_" + rand + "_" + ts;
  try {
    figma.root.setPluginData("mcp_session_id", newId);
  } catch(e) {}
  return newId;
}

var currentSessionId = getOrCreateSessionId();
var currentFileName = figma.root ? figma.root.name : "Untitled";
var currentFileKey = getSavedFileKey();

// Broadcast active file / session metadata on startup
try {
  figma.ui.postMessage({
    type: "session-info",
    sessionId: currentSessionId,
    fileName: currentFileName,
    fileKey: currentFileKey
  });
} catch (e) {}

// Broadcast selection changes live to UI
figma.on("selectionchange", function() {
  try {
    var sel = figma.currentPage.selection;
    var summary = [];
    var fullNode = null;
    for (var i = 0; i < Math.min(sel.length, 5); i++) {
      var n = sel[i];
      // If node is an internal instance sub-layer (starts with I...), find its main component instance or top frame
      var mainId = n.id;
      if (mainId.startsWith("I") && mainId.indexOf(";") !== -1) {
        var parts = mainId.split(";");
        mainId = parts[parts.length - 1]; // Use real canonical node id
      }
      summary.push({
        id: n.id,
        mainId: mainId,
        name: n.name,
        type: n.type,
        width: typeof n.width === "number" ? Math.round(n.width) : undefined,
        height: typeof n.height === "number" ? Math.round(n.height) : undefined
      });
    }
    if (sel.length === 1 && typeof nodeToInfo === "function") {
      try { fullNode = nodeToInfo(sel[0]); } catch(e0) {}
    }
    figma.ui.postMessage({
      type: "selection-change",
      count: sel.length,
      pageName: figma.currentPage ? figma.currentPage.name : undefined,
      selection: summary,
      fullNode: fullNode
    });
  } catch (e) {}
});

// Broadcast granular document changes to invalidate or incrementally update index cache
var docChangeTimer = null;
var pendingChangedNodeIds = new Set();

function onDocChange(event) {
  try {
    if (event && event.documentChanges) {
      for (var i = 0; i < event.documentChanges.length; i++) {
        var ch = event.documentChanges[i];
        if (ch && ch.id) pendingChangedNodeIds.add(ch.id);
      }
    }
    if (docChangeTimer) clearTimeout(docChangeTimer);
    docChangeTimer = setTimeout(async function() {
      var ids = Array.from(pendingChangedNodeIds);
      pendingChangedNodeIds.clear();

      if (ids.length === 1) {
        try {
          var singleNode = await findNodeByIdAsync(ids[0]);
          if (singleNode) {
            figma.ui.postMessage({
              type: "delta-diff",
              id: singleNode.id,
              diff: {
                name: singleNode.name,
                characters: "characters" in singleNode ? singleNode.characters : undefined,
                visible: singleNode.visible,
                width: singleNode.width,
                height: singleNode.height,
                x: singleNode.x,
                y: singleNode.y
              }
            });
            return;
          }
        } catch(e1) {}
      }

      if (ids.length > 0 && ids.length <= 15) {
        // Incremental micro-diff update for fast real-time editing
        var diffNodes = [];
        for (var j = 0; j < ids.length; j++) {
          try {
            var n = await findNodeByIdAsync(ids[j]);
            if (n && typeof nodeToInfo === "function") {
              diffNodes.push(nodeToInfo(n));
            }
          } catch(e2) {}
        }
        if (diffNodes.length > 0) {
          figma.ui.postMessage({ type: "node-diff", nodes: diffNodes });
          return;
        }
      }

      // Fallback for large batch changes: mark dirty or trigger background sync
      figma.ui.postMessage({ type: "document-change" });
    }, 500);
  } catch (e) {}
}

// In dynamic-page mode, Figma requires figma.loadAllPagesAsync() before registering documentchange.
// Delay loading all pages slightly so plugin UI opens and paints immediately without UI freeze.
setTimeout(function() {
  if (typeof figma.loadAllPagesAsync === "function") {
    figma.loadAllPagesAsync().then(function() {
      try {
        figma.on("documentchange", onDocChange);
      } catch (e) {}
    }).catch(function() {});
  } else {
    try {
      figma.on("documentchange", onDocChange);
    } catch (e) {}
  }
}, 800);

// Background initial scan after 1.8s startup delay (ensures UI is fully rendered and WebSocket connected)
setTimeout(async function() {
  try {
    if (handlers.index_scan) {
      var startMs = Date.now();
      var scanResult = await handlers.index_scan();
      figma.ui.postMessage({
        type: "index-update",
        data: scanResult,
        fileName: currentFileName,
        sessionId: currentSessionId,
        startMs: startMs
      });
    }
  } catch (e) {}
}, 1800);

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
    } catch(e) {}
    return;
  }

  if (request.type === "save-window-size") {
    var sw = Math.max(260, Math.min(1000, Math.round(request.width)));
    var sh = Math.max(200, Math.min(1200, Math.round(request.height)));
    try {
      figma.clientStorage.setAsync("mcp_window_size", { width: sw, height: sh }).catch(function() {});
    } catch(e) {}
    return;
  }

  // Set and persist Figma File Key
  if (request.type === "set-file-key") {
    try {
      var newFileKey = (request.fileKey || "").trim();
      if (newFileKey) {
        figma.root.setPluginData("figma_file_key", newFileKey);
        currentFileKey = newFileKey;
        figma.ui.postMessage({
          type: "session-info",
          sessionId: currentSessionId,
          fileName: currentFileName,
          fileKey: newFileKey
        });
        figma.notify("Figma File Key saved!", { timeout: 1500 });
      }
    } catch(e) {}
    return;
  }

  // Toast notification from UI
  if (request.type === "notify") {
    try {
      if (request.message) {
        figma.notify(request.message, { timeout: request.timeout || 1500, error: request.error || false });
      }
    } catch(e) {}
    return;
  }

  // Zoom to selection when clicked on selection bar
  if (request.type === "zoom-to-selection") {
    try {
      if (figma.currentPage.selection.length > 0) {
        figma.viewport.scrollAndZoomIntoView(figma.currentPage.selection);
        figma.notify("Focused on selection", { timeout: 1000 });
      }
    } catch(e) {}
    return;
  }

  // Export selection image as PNG for UI clipboard/download
  if (request.type === "export-selection-image") {
    try {
      var sel = figma.currentPage.selection;
      if (!sel || sel.length === 0) {
        figma.notify("Please select a layer to capture", { timeout: 1500 });
        return;
      }
      var targetNode = sel[0];
      var scale = request.scale || 2;
      var bytes = await targetNode.exportAsync({
        format: "PNG",
        constraint: { type: "SCALE", value: scale }
      });
      figma.ui.postMessage({
        type: "selection-image-exported",
        nodeId: targetNode.id,
        nodeName: targetNode.name,
        bytes: Array.from(bytes)
      });
    } catch (err) {
      figma.notify("Export failed: " + (err && err.message ? err.message : String(err)), { error: true });
    }
    return;
  }

  // Handle manual reindex request from UI
  if (request.type === "manual-reindex") {
    try {
      if (handlers.index_scan) {
        var startMs = Date.now();
        var scanResult = await handlers.index_scan();
        figma.ui.postMessage({
          type: "index-update",
          data: scanResult,
          fileName: currentFileName,
          sessionId: currentSessionId,
          startMs: startMs
        });
      }
    } catch(e) {}
    return;
  }

  const { id, operation, params } = request;
  const handler = typeof resolveOperationHandler === "function"
    ? resolveOperationHandler(operation)
    : handlers[operation];

  if (!handler) {
    figma.ui.postMessage({
      id, operation, success: false,
      error: `Unknown operation "${operation}". Available: ${Object.keys(handlers).join(", ")}`,
    });
    return;
  }

  const startTime = Date.now();
  try {
    var data = await handler(params || {});
    var durationMs = Date.now() - startTime;
    figma.ui.postMessage({ id: id, operation: operation, success: true, dataJson: stringifyForBridge(data), durationMs: durationMs });
  } catch (err) {
    var durationMs = Date.now() - startTime;
    var errMsg = "[dispatch:" + operation + "] " + (err && err.message ? err.message : String(err));
    figma.ui.postMessage({ id: id, operation: operation, success: false, error: errMsg, durationMs: durationMs });
  }
};

