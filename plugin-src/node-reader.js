// Protocol 3: projected, flat node reads. Cursors retain traversal position,
// not serialized trees, and expire whenever the active page changes.
var nodeRevision = 0;
var nodeReadCursors = new Map();
var nodeReadSequence = 0;
var NODE_FIELDS = ["geometry", "content", "text", "style", "layout", "tokens", "component"];

function copyNodeFields(node, target, keys) {
  keys.forEach(function(key) {
    if (key in node) {
      var value = node[key];
      target[key] = typeof value === "symbol" ? "mixed" : value;
    }
  });
}

function readNodeRecord(node, children, parentId, fields, opaque) {
  var info = { id: node.id, name: node.name, type: node.type, parentId: parentId,
    indexDetail: "minimal", opaque: opaque };
  if (!opaque) {
    info.childIds = children.map(function(child) { return child.id; });
    info.childCount = children.length;
  }
  if (fields.indexOf("geometry") !== -1) {
    copyNodeFields(node, info, ["x", "y", "width", "height", "rotation", "opacity", "visible", "clipsContent"]);
  }
  if (fields.indexOf("text") !== -1 && node.type === "TEXT") {
    Object.assign(info, resolveTextStyle(node, { segments: true }) || {});
    copyNodeFields(node, info, ["textAlignHorizontal", "textAlignVertical", "textAutoResize", "textTruncation"]);
  }
  if (fields.indexOf("content") !== -1 && node.type === "TEXT") info.content = node.characters;
  if (fields.indexOf("style") !== -1) {
    ["fills", "strokes"].forEach(function(key) {
      if (key in node) {
        var paints = node[key];
        info[key === "fills" ? "paintData" : "strokes"] = isMixed(paints) ? "mixed" : paints.map(serializePaint);
      }
    });
    copyNodeFields(node, info, ["effects", "cornerRadius", "topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius", "strokeAlign", "blendMode"]);
    applyStrokeWeight(node, info);
  }
  if (fields.indexOf("layout") !== -1) {
    copyNodeFields(node, info, ["layoutMode", "itemSpacing", "counterAxisSpacing", "layoutWrap",
      "paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "primaryAxisAlignItems",
      "counterAxisAlignItems", "primaryAxisSizingMode", "counterAxisSizingMode", "layoutAlign",
      "layoutGrow", "layoutPositioning", "layoutSizingHorizontal", "layoutSizingVertical", "constraints"]);
  }
  if (fields.indexOf("tokens") !== -1) {
    copyNodeFields(node, info, ["boundVariables", "fillStyleId", "strokeStyleId", "textStyleId", "effectStyleId"]);
  }
  if (fields.indexOf("component") !== -1) {
    var props = cleanComponentProperties(node);
    if (props.variant) info.variant = props.variant;
    if (props.props) info.props = props.props;
    copyNodeFields(node, info, ["description", "componentPropertyDefinitions"]);
  }
  return info;
}

handlers.read_nodes = async function(params) {
  var p = params || {};
  for (var target of ["id", "name"]) {
    if (p[target] !== undefined && (typeof p[target] !== "string" || !p[target])) throw new Error(target + " must be a non-empty string");
  }
  var limit = p.limit === undefined ? 200 : p.limit;
  if (!Number.isInteger(limit) || limit < 1 || limit > 500) throw new Error("limit must be an integer from 1 to 500");
  var state;
  if (p.cursor !== undefined) {
    if (typeof p.cursor !== "string" || !nodeReadCursors.has(p.cursor)) throw new Error("Cursor expired; start a new read_nodes request");
    state = nodeReadCursors.get(p.cursor);
    for (var key of ["id", "depth", "fields", "includeHidden", "expandInstances"]) {
      if (p[key] !== undefined && JSON.stringify(p[key]) !== JSON.stringify(state.options[key])) {
        throw new Error("Cursor options cannot change; start a new read_nodes request");
      }
    }
    nodeReadCursors.delete(p.cursor);
  } else {
    var fields = p.fields === undefined ? ["geometry", "content"] : p.fields;
    if (!Array.isArray(fields) || !fields.length || fields.some(function(field) { return NODE_FIELDS.indexOf(field) === -1; })) {
      throw new Error("fields must contain geometry, content, text, style, layout, tokens or component");
    }
    for (var flag of ["includeHidden", "expandInstances"]) {
      if (p[flag] !== undefined && typeof p[flag] !== "boolean") throw new Error(flag + " must be boolean");
    }
    var page = figma.currentPage;
    var root = p.id === page.id ? page : p.id ? await findNodeByIdAsync(p.id) : p.name ? findNodeByName(p.name) : page.selection && page.selection[0] || page;
    if (!root || root.removed) throw new Error("Node not found");
    var ancestor = root;
    while (ancestor && ancestor.type !== "PAGE") ancestor = ancestor.parent;
    if (root !== page && ancestor !== page) throw new Error("read_nodes only indexes the active page; switch pages first");
    var depth = p.depth === undefined ? (root === page ? 0 : 256) : p.depth === "full" ? 256 : p.depth;
    if (!Number.isInteger(depth) || depth < 0 || depth > 256) throw new Error("depth must be full or an integer from 0 to 256");
    var options = { id: root.id, depth: depth, fields: fields, includeHidden: p.includeHidden === true, expandInstances: p.expandInstances === true };
    var roots = withInstanceVisibility(options.includeHidden, function() { return root === page ? page.children : [root]; });
    state = { options: options, pageId: page.id, revision: nodeRevision, visited: 0,
      stack: [{ children: roots, next: 0, depth: 0, parentId: root === page || !root.parent || root.parent.type === "PAGE" ? null : root.parent.id }] };
  }
  if (state.pageId !== figma.currentPage.id || state.revision !== nodeRevision) throw new Error("Cursor expired after a page or document change");
  var nodes = [], instances = [];
  while (state.stack.length && nodes.length < limit && state.visited < 50000) {
    var started = Date.now();
    withInstanceVisibility(state.options.includeHidden, function() {
      while (state.stack.length && nodes.length < limit && state.visited < 50000) {
        if (Date.now() - started >= 8) break;
        var cursor = state.stack[state.stack.length - 1];
        if (cursor.next >= cursor.children.length) { state.stack.pop(); continue; }
        var node = cursor.children[cursor.next++];
        if (!node || node.removed || (!state.options.includeHidden && node.visible === false)) continue;
        var opaque = node.type === "INSTANCE" && !state.options.expandInstances;
        var children = !opaque && "children" in node ? node.children : [];
        var info = readNodeRecord(node, children, cursor.parentId, state.options.fields, opaque);
        info.childrenLoaded = !opaque && (!children.length || cursor.depth < state.options.depth);
        nodes.push(info);
        state.visited++;
        if (node.type === "INSTANCE" && state.options.fields.indexOf("component") !== -1) instances.push({ info: info, node: node });
        if (children.length && cursor.depth < state.options.depth) {
          state.stack.push({ children: children, next: 0, depth: cursor.depth + 1, parentId: node.id });
        }
        if (Date.now() - started >= 8) break;
      }
    });
    if (state.stack.length && nodes.length < limit) await yieldToUI(0);
    if (state.pageId !== figma.currentPage.id || state.revision !== nodeRevision) throw new Error("Document changed during read; retry read_nodes");
  }
  // Remove exhausted ancestors so the final page does not need an empty follow-up.
  while (state.stack.length && state.stack[state.stack.length - 1].next >= state.stack[state.stack.length - 1].children.length) state.stack.pop();
  var componentResolution = instances.length ? await resolveInstanceComponents(instances) : null;
  if (state.pageId !== figma.currentPage.id || state.revision !== nodeRevision) throw new Error("Document changed during read; retry read_nodes");
  var nextCursor = null;
  if (state.stack.length && state.visited < 50000) {
    nextCursor = "nodes:" + (++nodeReadSequence);
    nodeReadCursors.set(nextCursor, state);
    // ponytail: keep 16 in-flight traversals; clients restart older cursors if evicted.
    if (nodeReadCursors.size > 16) nodeReadCursors.delete(nodeReadCursors.keys().next().value);
  }
  return { schemaVersion: 4, pageId: state.pageId, revision: state.revision, scope: state.options,
    nodes: nodes, nextCursor: nextCursor, complete: !state.stack.length, totalRead: state.visited,
    budgetReached: state.stack.length > 0 && state.visited >= 50000, componentResolution: componentResolution };
};
