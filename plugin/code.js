// ─── FIGMA MCP BRIDGE — DYNAMIC THIN LOADER (v3.0.0) ───────────────────────────
// This file is a permanent micro-stub. You never need to reinstall or edit this file.
// The actual runtime is loaded dynamically from the local figma-mcp Rust engine.

figma.showUI(__html__, { width: 340, height: 480, title: "Figma MCP Bridge", themeColors: true });

// Restore window size if saved
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

// Initial stub message listener
figma.ui.onmessage = async function(msg) {
  if (!msg) return;

  if (msg.type === "EVAL_MAIN_CODE" && typeof msg.code === "string") {
    try {
      // Execute dynamic main runtime with figma context
      var runner = new Function("figma", "__html__", msg.code);
      runner(figma, __html__);
    } catch (err) {
      console.error("[figma-mcp stub] Failed to evaluate dynamic runtime:", err);
      try {
        figma.notify("MCP Runtime Error: " + (err && err.message ? err.message : String(err)), { error: true });
      } catch(e) {}
    }
    return;
  }

  if (msg.type === "resize") {
    var newW = Math.max(260, Math.min(1000, Math.round(msg.width)));
    var newH = Math.max(200, Math.min(1200, Math.round(msg.height)));
    try { figma.ui.resize(newW, newH); } catch(e) {}
    return;
  }

  if (msg.type === "save-window-size") {
    var sw = Math.max(260, Math.min(1000, Math.round(msg.width)));
    var sh = Math.max(200, Math.min(1200, Math.round(msg.height)));
    try {
      figma.clientStorage.setAsync("mcp_window_size", { width: sw, height: sh }).catch(function() {});
    } catch(e) {}
    return;
  }
};
