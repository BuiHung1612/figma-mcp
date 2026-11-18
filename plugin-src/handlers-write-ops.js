// ─── NEW WRITE OPERATIONS ────────────────────────────────────────────────────

// clone — duplicate a node
var CLONE_KEYS = new Set(["id", "nodeId", "x", "y", "name", "key", "parentId", "insertIndex"]);

handlers.clone = async function(params) {
  assertKnownKeys("clone", params, CLONE_KEYS);
  var node = await findNodeByIdAsync(params.id || params.nodeId);
  if (!node) throw new Error("clone: node not found: " + (params.id || params.nodeId));
  var parent = params.parentId ? await findNodeByIdAsync(params.parentId) : null;
  if (params.parentId && !parent) throw new Error("parentId not found: " + params.parentId);
  var clone = node.clone();
  if (parent || params.insertIndex !== undefined) {
    parent = parent || clone.parent;
    parent.insertChild(params.insertIndex === undefined ? parent.children.length : Math.max(0, Math.min(params.insertIndex, parent.children.length)), clone);
  }
  if (params.x !== undefined) clone.x = params.x;
  if (params.y !== undefined) clone.y = params.y;
  if (params.name) clone.name = params.name;
  // Keys must stay unique: the copy keeps none unless one is given.
  clearKeys(clone);
  if (params.key !== undefined) clone.setPluginData(NODE_KEY_DATA, String(params.key));
  journalCreated(params, clone);
  var info = nodeToInfo(clone);
  info.idMap = cloneIdMap(node, clone);
  return info;
};

function clearKeys(root) {
  var stack = [root];
  while (stack.length) {
    var n = stack.pop();
    try { if (n.getPluginData(NODE_KEY_DATA)) n.setPluginData(NODE_KEY_DATA, ""); } catch (e) {}
    if (n.type !== "INSTANCE" && n.children) for (var i = 0; i < n.children.length; i++) stack.push(n.children[i]);
  }
}

// clone() mirrors the source tree, so walking both in step maps every source id
// to its copy. Instance internals are skipped: their ids derive from the
// instance id ("I<instance>;<path>") and are addressable that way.
var CLONE_ID_MAP_LIMIT = 2000;
function cloneIdMap(source, copy) {
  var map = {}, count = 0, stack = [[source, copy]];
  while (stack.length && count < CLONE_ID_MAP_LIMIT) {
    var pair = stack.pop(), a = pair[0], b = pair[1];
    map[a.id] = b.id;
    count++;
    if (a.type === "INSTANCE" || !a.children || !b.children) continue;
    for (var i = a.children.length - 1; i >= 0; i--) {
      if (b.children[i]) stack.push([a.children[i], b.children[i]]);
    }
  }
  if (stack.length) map._truncated = true;
  return map;
}

// One figma_write script = one undo step. Figma folds plugin edits into a
// single history entry until commitUndo, so without this one Ctrl+Z reverts
// everything the agent drew in the session.
handlers.commit_undo = async function(params) {
  if (typeof figma.commitUndo === "function") figma.commitUndo();
  journalFor(params).length = 0;
  return { committed: true };
};

// Removes nodes created since the last commit_undo (a failed figma_write).
handlers.rollback = async function(params) {
  return { removed: rollbackJournal(params, 0) };
};

// figma.byKey(key): the one node created with that key, or null.
handlers.find_by_key = async function(params) {
  if (!params || !params.key) throw new Error("byKey: key is required");
  var scope = params.parentId ? await findNodeByIdAsync(params.parentId) : figma.currentPage;
  if (!scope) throw new Error("parentId not found: " + params.parentId);
  var key = String(params.key);
  var found = scope.findAll(function(n) { try { return n.getPluginData(NODE_KEY_DATA) === key; } catch (e) { return false; } });
  if (found.length > 1) throw new Error("byKey: " + found.length + " nodes share key \"" + key + "\": " + found.map(function(n) { return n.id; }).join(", ") + ". Pass parentId to narrow it.");
  return found.length ? nodeToInfo(found[0]) : null;
};

// detach: turn an instance into a plain frame so its layers can be edited freely.
handlers.detach = async function(params) {
  assertKnownKeys("detach", params, new Set(["id", "nodeId"]));
  var node = await findNodeByIdAsync(params.id || params.nodeId);
  if (!node || node.type !== "INSTANCE") throw new Error("detach: " + (params.id || params.nodeId) + " is not an instance");
  return nodeToInfo(node.detachInstance());
};

// render: build a whole tree in one call. Each node is a create() spec plus
// optional children; `key` names a node so the result maps keys to ids.
var RENDER_DEPTH_LIMIT = 32, RENDER_NODE_LIMIT = 500;
handlers.render = async function(params) {
  assertKnownKeys("render", params, new Set(["tree", "parentId", "insertIndex"]));
  if (!params.tree || typeof params.tree !== "object") throw new Error("render: tree is required");
  var count = 0, ids = {};
  async function build(spec, parentId, insertIndex, depth) {
    if (depth > RENDER_DEPTH_LIMIT) throw new Error("render: tree deeper than " + RENDER_DEPTH_LIMIT);
    if (++count > RENDER_NODE_LIMIT) throw new Error("render: more than " + RENDER_NODE_LIMIT + " nodes; split the tree");
    var children = spec.children || [];
    if (!Array.isArray(children)) throw new Error("render: children must be an array");
    var createParams = Object.assign({}, spec);
    delete createParams.children;
    if (parentId) createParams.parentId = parentId;
    if (insertIndex !== undefined) createParams.insertIndex = insertIndex;
    createParams._taskId = params._taskId;
    var info = await handlers.create(createParams);
    if (spec.key !== undefined) {
      if (ids[spec.key]) throw new Error("render: duplicate key \"" + spec.key + "\"");
      ids[spec.key] = info.id;
    }
    for (var i = 0; i < children.length; i++) await build(children[i], info.id, undefined, depth + 1);
    return info;
  }
  var mark = journalFor(params).length;
  try {
    var root = await build(params.tree, params.parentId, params.insertIndex, 0);
  } catch (e) {
    rollbackJournal(params, mark);
    throw e;
  }
  // Children were journaled one by one; removing the root removes them all.
  journalFor(params).length = mark;
  journalCreated(params, await findNodeByIdAsync(root.id));
  return { root: nodeToInfo(await findNodeByIdAsync(root.id)), ids: ids };
};

handlers.list_available_fonts = async function(params) {
  var q = (params && (params.query || params.family) || "").toLowerCase();
  var fonts = await figma.listAvailableFontsAsync();
  var families = {};
  for (var i = 0; i < fonts.length; i++) {
    var f = fonts[i].fontName;
    if (q && f.family.toLowerCase().indexOf(q) === -1) continue;
    (families[f.family] = families[f.family] || []).push(f.style);
  }
  var limit = (params && params.limit) || 50;
  return Object.keys(families).slice(0, limit).map(function(family) { return { family: family, styles: families[family] }; });
};

// Unlike ensureFontLoaded this never falls back: it tells the caller whether
// the exact font is usable before it edits text in that font.
handlers.load_font = async function(params) {
  if (!params || !params.family) throw new Error("family is required");
  var style = params.style || "Regular";
  try {
    await figma.loadFontAsync({ family: params.family, style: style });
    return { family: params.family, style: style, loaded: true };
  } catch (e) {
    throw new Error("Font \"" + params.family + " " + style + "\" is not available in this Figma session. " +
      "Call figma.listAvailableFonts({ query }) to see installed families.");
  }
};

// group — group selected or specified nodes
handlers.group = async function(params) {
  var nodeIds = params.nodeIds || [];
  var nodes = [];
  for (var i = 0; i < nodeIds.length; i++) {
    var n = await findNodeByIdAsync(nodeIds[i]);
    if (n) nodes.push(n);
  }
  if (nodes.length < 1) throw new Error("Need at least 1 node to group");
  var parent = nodes[0].parent || figma.currentPage;
  var group = figma.group(nodes, parent);
  if (params.name) group.name = params.name;
  return nodeToInfo(group);
};

// ungroup — ungroup a group node
handlers.ungroup = async function(params) {
  var node = await resolveNode(params);
  if (!node) throw new Error("Node not found for ungrouping");
  if (node.removed) throw new Error("Node was already deleted");
  if (node.type !== "GROUP" && node.type !== "FRAME") throw new Error("Node must be GROUP or FRAME to ungroup");
  if (!node.children || node.children.length === 0) {
    node.remove();
    return { ungrouped: [] };
  }
  var children = [];
  var parent = node.parent || figma.currentPage;
  if (parent.removed) parent = figma.currentPage;
  var nodeChildren = [].concat(node.children);
  for (var i = 0; i < nodeChildren.length; i++) {
    parent.appendChild(nodeChildren[i]);
    children.push(nodeToInfo(nodeChildren[i]));
  }
  node.remove();
  return { ungrouped: children };
};

// flatten — flatten a node (merge vectors)
handlers.flatten = async function(params) {
  var node = await resolveNode(params);
  if (!node) throw new Error("Node not found for flatten");
  var flat = figma.flatten([node]);
  return nodeToInfo(flat);
};

// resize — resize a node with constraints
handlers.resize = async function(params) {
  assertKnownKeys("resize", params, new Set(["id", "nodeId", "width", "height"]));
  var node = await findNodeByIdAsync(params.id || params.nodeId);
  if (!node) throw new Error("Node not found for resize");
  if (!("resize" in node)) throw new Error("Node type does not support resize");
  var w = params.width !== undefined ? params.width : node.width;
  var h = params.height !== undefined ? params.height : node.height;
  node.resize(w, h);
  return nodeToInfo(node);
};

// set_selection — programmatically select nodes
handlers.set_selection = async function(params) {
  var nodeIds = params.nodeIds || [];
  var nodes = [];
  for (var i = 0; i < nodeIds.length; i++) {
    var n = await findNodeByIdAsync(nodeIds[i]);
    if (n) nodes.push(n);
  }
  // BUG-08: nodes may belong to a different page than currentPage.
  // Find the page of the first node and switch to it before setting selection.
  if (nodes.length > 0) {
    var targetPage = null;
    var candidate = nodes[0];
    while (candidate && candidate.type !== "PAGE") candidate = candidate.parent;
    if (candidate && candidate.type === "PAGE" && candidate !== figma.currentPage) {
      await figma.setCurrentPageAsync(candidate);
    }
  }
  figma.currentPage.selection = nodes;
  return { selected: nodes.map(nodeToInfo) };
};

handlers.setSelection = handlers.set_selection;

// batch — execute multiple operations in one call
// Supported operations: create, modify, delete (single or batch ids), append, clone
// All or nothing: stops at the first failing op, removes the nodes the batch
// created and throws. Returns each op's result in order.
handlers.batch = async function(params) {
  var operations = Array.isArray(params) ? params : (params.operations || []);
  if (!operations.length) throw new Error("No operations provided");
  if (operations.length > 50) throw new Error("Max 50 operations per batch");
  var mark = journalFor(params).length;
  var results = [];
  for (var i = 0; i < operations.length; i++) {
    var op = operations[i];
    var handler = typeof resolveOperationHandler === "function" ? resolveOperationHandler(op.operation) : handlers[op.operation];
    try {
      if (!handler) throw new Error("unknown operation");
      results.push(await handler(Object.assign({}, op.params || {}, { _taskId: params._taskId })));
    } catch (e) {
      var removed = rollbackJournal(params, mark);
      throw new Error("batch op " + i + " (" + op.operation + ") failed: " + e.message +
        (removed ? " — removed the " + removed + " node(s) this batch had created" : ""));
    }
  }
  return results;
};

