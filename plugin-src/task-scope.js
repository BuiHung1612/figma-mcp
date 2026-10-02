// Task leases live in one plugin runtime and bind writes to independent frames.
var frameTasks = new Map();

function operationKey(operation) {
  return String(operation).replace(/[^a-z0-9]/gi, "").toLowerCase();
}

function taskReadOperation(operation) {
  return ["status", "query", "listpages", "listcomponents", "getselection", "getdesign",
    "getpagenodes", "getnodedetail", "getdesigncontext", "getcss", "getcomponentmap", "getunmappedcomponents",
    "getstyles", "getvariables", "getvariabletokens", "gettokens", "getlocalcomponents", "getviewport",
    "screenshot", "exportsvg", "exportimage", "exportassets", "scandesign", "searchnodes", "indexscan",
    "getcomponentproperties", "getreactions"].indexOf(operationKey(operation)) !== -1;
}

handlers.task_start = async function(params) {
  if (!params.taskId || !params.frameId) throw new Error("taskId and frameId are required");
  if (frameTasks.has(params.taskId)) throw new Error("Task already exists");
  var frame = await findNodeByIdAsync(params.frameId);
  if (!frame || frame.type !== "FRAME" || !frame.parent ||
      (frame.parent.type !== "PAGE" && frame.parent.type !== "SECTION")) {
    throw new Error("Task frame must be an independent FRAME directly under a PAGE or SECTION");
  }
  for (var owned of frameTasks.values()) {
    if (owned === frame.id) throw new Error("Frame is already reserved by another task");
  }
  // Definitions can affect instances elsewhere; tasks may only edit local UI.
  var pending = [frame], visited = 0;
  while (pending.length) {
    var node = pending.pop();
    if (node.type === "COMPONENT" || node.type === "COMPONENT_SET") {
      throw new Error("Task frames cannot contain shared component definitions");
    }
    if (node.type !== "INSTANCE" && node.children) {
      for (var i = 0; i < node.children.length; i++) pending.push(node.children[i]);
    }
    if (++visited % 100 === 0) await yieldToUI(0);
  }
  frameTasks.set(params.taskId, frame.id);
  return { taskId: params.taskId, frameId: frame.id, frameName: frame.name };
};

handlers.task_end = async function(params) {
  if (!frameTasks.delete(params.taskId)) throw new Error("Unknown taskId");
  return { taskId: params.taskId, released: true };
};

async function taskNode(id, root, allowRoot) {
  if (typeof id !== "string" || !id) throw new Error("Scoped writes require explicit node IDs");
  var node = await findNodeByIdAsync(id);
  if (!node || node.removed) throw new Error("Task target no longer exists: " + id);
  if (!allowRoot && node.id === root.id) throw new Error("Cannot remove, move, clone or flatten the task root");
  var cursor = node;
  while (cursor) {
    if (cursor.type === "COMPONENT" || cursor.type === "COMPONENT_SET") throw new Error("Shared component definitions are outside task scope");
    if (cursor.id === root.id) return node;
    cursor = cursor.parent;
  }
  throw new Error("Node is outside task frame: " + id);
}

async function validateTaskOperation(operation, params, taskId, depth) {
  var key = operationKey(operation);
  taskId = taskId || params._taskId;
  if (!taskId) {
    if (frameTasks.size && !taskReadOperation(operation) && key !== "taskstart" && key !== "taskend") {
      throw new Error("Active frame tasks require taskId for writes");
    }
    return { operation: operation, params: params };
  }
  var frameId = frameTasks.get(taskId);
  if (!frameId) throw new Error("Unknown taskId; restart the task after plugin reload");
  var root = await findNodeByIdAsync(frameId);
  if (!root || root.removed || root.type !== "FRAME" || !root.parent ||
      (root.parent.type !== "PAGE" && root.parent.type !== "SECTION")) throw new Error("Task frame was deleted or moved; restart the task");
  params = Object.assign({}, params);
  if (taskReadOperation(operation)) {
    if (key === "getselection") { operation = "get_design"; key = "getdesign"; }
    if (["getdesign", "getnodedetail", "getdesigncontext", "getcss", "getcomponentmap", "getunmappedcomponents",
         "screenshot", "exportsvg", "exportimage", "exportassets", "scandesign", "searchnodes"].indexOf(key) !== -1 &&
        !params.id && !params.nodeId && !params.name && !params.nodeName) params.id = frameId;
    if (key === "screenshot" || key === "exportimage") params.keepViewport = true;
    return { operation: operation, params: params };
  }
  if (key === "batch") {
    if ((depth || 0) >= 8) throw new Error("Task batch nesting limit exceeded");
    var ops = params.operations || [];
    if (!Array.isArray(ops) || ops.length > 50) throw new Error("Invalid task batch");
    var checked = [];
    // Validate the entire batch before executing any mutation.
    for (var bi = 0; bi < ops.length; bi++) checked.push(await validateTaskOperation(ops[bi].operation, ops[bi].params || {}, taskId, (depth || 0) + 1));
    params.operations = checked;
  } else if (key === "create") {
    if (["FRAME", "GROUP", "RECTANGLE", "ELLIPSE", "LINE", "TEXT", "SVG", "VECTOR", "IMAGE"].indexOf(params.type) === -1) {
      throw new Error("Task create type is not supported");
    }
    params.parentId = params.parentId || frameId;
    await taskNode(params.parentId, root, true);
  } else if (key === "append") {
    await taskNode(params.parentId, root, true);
    await taskNode(params.childId, root, false);
    delete params.parentName; delete params.childName;
  } else if (key === "group") {
    if (!Array.isArray(params.nodeIds) || !params.nodeIds.length) throw new Error("Task group requires nodeIds");
    for (var gi = 0; gi < params.nodeIds.length; gi++) await taskNode(params.nodeIds[gi], root, false);
  } else if (["modify", "resize", "clone", "delete", "remove", "ungroup", "flatten", "setcomponentproperties", "swapcomponent"].indexOf(key) !== -1) {
    var ids = params.ids || [params.id || params.nodeId || params.targetId || params.target_id];
    if (!Array.isArray(ids) || !ids.length) throw new Error("Invalid task node IDs");
    for (var ni = 0; ni < ids.length; ni++) await taskNode(ids[ni], root, key === "modify" || key === "resize" || key === "setcomponentproperties" || key === "swapcomponent");
    // Canonicalize targets so a handler cannot fall back to a name elsewhere.
    if (!params.ids) params.id = ids[0];
    delete params.nodeName;
    if (key !== "modify" && key !== "clone") delete params.name;
    if (params.parentId) await taskNode(params.parentId, root, true);
    if (key === "delete" || key === "remove") params.force = false;
  } else {
    throw new Error("Operation '" + operation + "' is not allowed in a frame task (global styles, variables, selection and page changes are shared)");
  }
  return { operation: operation, params: params };
}
