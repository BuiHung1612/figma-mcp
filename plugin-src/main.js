// ─── PLUGIN ENTRY POINT ───────────────────────────────────────────────────────

// The thin loader already owns the UI. Reopening __html__ here would replace
// its iframe with the loader again while it is fetching the full runtime UI.

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

var currentDocumentId = getOrCreateSessionId();
var currentSessionId = currentDocumentId + ":tab:" + Date.now().toString(36) + "_" + Math.random().toString(36).slice(2, 12);
var currentFileName = figma.root ? figma.root.name : "Untitled";
var currentFileKey = getSavedFileKey();

// Broadcast active file / session metadata on startup
try {
  figma.ui.postMessage({
    type: "session-info",
    sessionId: currentSessionId,
    fileName: currentFileName,
    fileKey: currentFileKey,
    documentId: currentDocumentId,
    runtimeVersion: "{{PLUGIN_VERSION}}",
    protocolVersion: 2,
    operations: Object.keys(handlers)
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
var docChangeGeneration = 0;

function onDocChange(event) {
  try {
    var generation = ++docChangeGeneration;
    variableCache.clear();
    var changes = event && (event.documentChanges || event.nodeChanges);
    var ids = [];
    if (changes) changes.forEach(function(ch) {
      var id = ch && (ch.id || (ch.node && ch.node.id));
      if (id) { ids.push(id); pendingChangedNodeIds.add(id); }
      if (ch && ch.node && ch.node.parent && ch.node.parent.type !== "PAGE") {
        var parentId = ch.node.parent.id;
        ids.push(parentId); pendingChangedNodeIds.add(parentId);
      }
    });
    figma.ui.postMessage({ type: "document-change", changedNodeIds: ids });
    if (!ids.length) return;
    if (docChangeTimer) clearTimeout(docChangeTimer);
    var changedPage = figma.currentPage;
    docChangeTimer = setTimeout(async function() {
      var ids = Array.from(pendingChangedNodeIds);
      pendingChangedNodeIds.clear();
      var nodes = [], deleted = [];
      for (var id of ids) {
        try {
          var node = await findNodeByIdAsync(id);
          if (!node || node.removed) { deleted.push(id); continue; }
          var data = extractDesignTree(node, 0, 0, "compact", false);
          data.parentId = node.parent ? node.parent.id : null;
          data.childIds = "children" in node ? node.children.map(function(c) { return c.id; }) : [];
          nodes.push(data);
        } catch(e) { figma.ui.postMessage({ type: "document-change" }); return; }
      }
      if (figma.currentPage.id !== changedPage.id) return;
      if (generation !== docChangeGeneration) {
        onDocChange({ nodeChanges: ids.map(function(id) { return { id: id }; }) });
        return;
      }
      if (figma.currentPage.id === changedPage.id) {
        figma.ui.postMessage({ type: "node-diff", nodes: nodes, deletedIds: deleted });
      }
    }, 100);
  } catch(e) { figma.ui.postMessage({ type: "document-change" }); }
}

// Listen only to the active page: documentchange requires loading the entire
// file and was making startup expensive even before the index scan began.
var watchedPage = null;
function watchCurrentPage() {
  if (watchedPage && typeof watchedPage.off === "function") {
    watchedPage.off("nodechange", onDocChange);
  }
  watchedPage = figma.currentPage;
  if (watchedPage && typeof watchedPage.on === "function") {
    watchedPage.on("nodechange", onDocChange);
  }
  pendingChangedNodeIds.clear();
  if (docChangeTimer) clearTimeout(docChangeTimer);
}
try {
  watchCurrentPage();
  figma.on("stylechange", onDocChange);
  figma.on("currentpagechange", function() {
    watchCurrentPage();
    onDocChange();
    publishIndex(true).catch(function() {});
  });
} catch(e) {}

async function publishIndex(deferComponents) {
  var scanResult = await handlers.index_scan({ deferComponents: deferComponents });
  // A scan for the previous page must not overwrite the newly active page.
  if (scanResult.pageId !== figma.currentPage.id) return;
}

// Startup indexes the active page and tokens, deferring the file-wide component
// catalogue until a component query or an explicit reindex requests it.
setTimeout(function() {
  publishIndex(true).catch(function() {});
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

var bridgeReplies = new Map();
function sendBridgeReply(reply) {
  reply.sessionId = currentSessionId;
  bridgeReplies.set(reply.id, reply);
  // ponytail: replay cache covers the last 256 requests in this runtime;
  // durable replay after a plugin restart needs a persisted operation journal.
  if (bridgeReplies.size > 256) bridgeReplies.delete(bridgeReplies.keys().next().value);
  figma.ui.postMessage(reply);
}

async function handlePluginRequest(request) {
  if (!request) return;
  if (request.id && bridgeReplies.has(request.id)) {
    figma.ui.postMessage(bridgeReplies.get(request.id));
    return;
  }

  if (request.type === "task-sync") {
    var liveTasks = new Set(request.taskIds || []);
    for (var leaseId of frameTasks.keys()) { if (!liveTasks.has(leaseId)) frameTasks.delete(leaseId); }
    return;
  }

  if (request.type === "runtime-ready") {
    figma.ui.postMessage({ type: "session-info", sessionId: currentSessionId,
      fileName: currentFileName, fileKey: currentFileKey,
    documentId: currentDocumentId,
      runtimeVersion: "{{PLUGIN_VERSION}}", protocolVersion: 2, operations: Object.keys(handlers) });
    return;
  }

  if (request.type === "EVAL_MAIN_CODE" && typeof request.code === "string") {
    try {
      var runner = new Function("figma", "__html__", request.code);
      runner(figma, __html__);
    } catch (err) {
      console.error("[figma-rust-mcp dynamic] Failed to re-evaluate dynamic runtime:", err);
    }
    return;
  }

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
    try { await publishIndex(false); } catch(e) {}
    return;
  }

  const { id, operation, params } = request;
  const handler = typeof resolveOperationHandler === "function"
    ? resolveOperationHandler(operation)
    : handlers[operation];

  if (!handler) {
    sendBridgeReply({
      id, operation, success: false,
      error: `Unsupported operation "${operation}" (request ${id}, runtime {{PLUGIN_VERSION}}, protocol 2). Available: ${Object.keys(handlers).join(", ")}`,
    });
    return;
  }

  const startTime = Date.now();
  try {
    var scoped = await validateTaskOperation(operation, params || {});
    var scopedHandler = resolveOperationHandler(scoped.operation);
    var data = await scopedHandler(scoped.params);
    var durationMs = Date.now() - startTime;
    sendBridgeReply({ id: id, operation: operation, success: true, dataJson: stringifyForBridge(data), durationMs: durationMs });
  } catch (err) {
    var durationMs = Date.now() - startTime;
    var errMsg = "[dispatch:" + operation + "] " + (err && err.message ? err.message : String(err));
    sendBridgeReply({ id: id, operation: operation, success: false, error: errMsg, durationMs: durationMs });
  }
 }

var pluginRequestTail = Promise.resolve();
figma.ui.onmessage = function(request) {
  var result = pluginRequestTail.then(function() { return handlePluginRequest(request); });
  pluginRequestTail = result.catch(function() {});
  return result;
};
