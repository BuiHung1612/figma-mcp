pub mod assets;

use crate::bridge::BridgeHandle;
use assets::fetch_svg_icon;
use base64::Engine;
use boa_engine::{
    js_string,
    native_function::NativeFunction,
    object::FunctionObjectBuilder,
    property::Attribute,
    Context, JsError, JsValue, Source,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::sync::mpsc::{channel as sync_channel, Sender as SyncSender};
use std::sync::Arc;

pub const TIMEOUT_MS: u64 = 30_000;

pub enum HostRequest {
    Op {
        operation: String,
        params: Value,
        resp: SyncSender<Result<Value, String>>,
    },
    Icon {
        name: String,
        opts: Value,
        resp: SyncSender<Result<Value, String>>,
    },
    Image {
        url: String,
        opts: Value,
        resp: SyncSender<Result<Value, String>>,
    },
    Log {
        level: String,
        msg: String,
    },
}

thread_local! {
    static HOST_TX: RefCell<Option<tokio::sync::mpsc::UnboundedSender<HostRequest>>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub success: bool,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub logs: Vec<String>,
}

pub async fn execute_code(
    code: &str,
    bridge: BridgeHandle,
    session_id: Option<String>,
) -> ExecResult {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<HostRequest>();
    let code_owned = code.to_string();

    let logs = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let logs_collector = logs.clone();

    // Spawn async handler for host requests
    let bridge_clone = bridge.clone();
    let sid_clone = session_id.clone();
    let http_client = reqwest::Client::new();

    let host_handler = tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            match req {
                HostRequest::Op { operation, params, resp } => {
                    let res = bridge_clone.send_operation(&operation, params, sid_clone.as_deref()).await;
                    let _ = resp.send(res);
                }
                HostRequest::Icon { name, opts, resp } => {
                    let size = opts.get("size").and_then(|v| v.as_f64()).unwrap_or(24.0);
                    let fill = opts.get("fill").and_then(|v| v.as_str()).unwrap_or("#1E3150").to_string();

                    let res = match fetch_svg_icon(&http_client, &name, size, &fill).await {
                        Ok(svg) => {
                            let mut create_params = json!({
                                "type": "SVG",
                                "name": opts.get("name").and_then(|v| v.as_str()).unwrap_or(&format!("icon/{}", name)),
                                "width": size,
                                "height": size,
                                "svg": svg,
                                "fill": fill,
                            });
                            if let Some(pid) = opts.get("parentId") { create_params["parentId"] = pid.clone(); }
                            if let Some(x) = opts.get("x") { create_params["x"] = x.clone(); }
                            if let Some(y) = opts.get("y") { create_params["y"] = y.clone(); }
                            bridge_clone.send_operation("create", create_params, sid_clone.as_deref()).await
                        }
                        Err(e) => Err(e),
                    };
                    let _ = resp.send(res);
                }
                HostRequest::Image { url, opts, resp } => {
                    let res = match http_client.get(&url).send().await {
                        Ok(r) if r.status().is_success() => match r.bytes().await {
                            Ok(bytes) => {
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                                let mut create_params = json!({
                                    "type": "IMAGE",
                                    "name": opts.get("name").and_then(|v| v.as_str()).unwrap_or("image"),
                                    "width": opts.get("width").and_then(|v| v.as_f64()).unwrap_or(100.0),
                                    "height": opts.get("height").and_then(|v| v.as_f64()).unwrap_or(100.0),
                                    "imageData": b64,
                                    "scaleMode": opts.get("scaleMode").and_then(|v| v.as_str()).unwrap_or("FILL"),
                                });
                                if let Some(pid) = opts.get("parentId") { create_params["parentId"] = pid.clone(); }
                                if let Some(x) = opts.get("x") { create_params["x"] = x.clone(); }
                                if let Some(y) = opts.get("y") { create_params["y"] = y.clone(); }
                                if let Some(cr) = opts.get("cornerRadius") { create_params["cornerRadius"] = cr.clone(); }
                                bridge_clone.send_operation("create", create_params, sid_clone.as_deref()).await
                            }
                            Err(e) => Err(format!("Failed to read image bytes: {}", e)),
                        },
                        Ok(r) => Err(format!("Image download failed with HTTP {}", r.status())),
                        Err(e) => Err(format!("Failed to fetch image: {}", e)),
                    };
                    let _ = resp.send(res);
                }
                HostRequest::Log { level, msg } => {
                    let line = match level.as_str() {
                        "error" => format!("[error] {}", msg),
                        "warn" => format!("[warn] {}", msg),
                        "info" => format!("[info] {}", msg),
                        _ => msg,
                    };
                    if let Ok(mut l) = logs_collector.lock() {
                        l.push(line);
                    }
                }
            }
        }
    });

    let eval_task = tokio::task::spawn_blocking(move || {
        HOST_TX.with(|cell| {
            *cell.borrow_mut() = Some(tx);
        });

        let mut context = Context::default();

        // Register __host_console
        let console_fn = NativeFunction::from_copy_closure(|_this, args, _ctx| {
            let level = args.first().and_then(|v| v.as_string()).map(|s| s.to_std_string_escaped()).unwrap_or_default();
            let msg = args.get(1).and_then(|v| v.as_string()).map(|s| s.to_std_string_escaped()).unwrap_or_default();
            HOST_TX.with(|cell| {
                if let Some(tx) = cell.borrow().as_ref() {
                    let _ = tx.send(HostRequest::Log { level, msg });
                }
            });
            Ok(JsValue::undefined())
        });
        let console_obj = FunctionObjectBuilder::new(context.realm(), console_fn)
            .name(js_string!("__host_console"))
            .length(2)
            .build();
        context.register_global_property(js_string!("__host_console"), console_obj, Attribute::all()).unwrap();

        // Register __host_op
        let host_op_fn = NativeFunction::from_copy_closure(|_this, args, _ctx| {
            let op = args.first().and_then(|v| v.as_string()).map(|s| s.to_std_string_escaped()).unwrap_or_default();
            let params_json = args.get(1).and_then(|v| v.as_string()).map(|s| s.to_std_string_escaped()).unwrap_or_else(|| "{}".to_string());
            let params: Value = serde_json::from_str(&params_json).unwrap_or(json!({}));

            let (resp_tx, resp_rx) = sync_channel();
            let sent = HOST_TX.with(|cell| {
                if let Some(tx) = cell.borrow().as_ref() {
                    tx.send(HostRequest::Op { operation: op, params, resp: resp_tx }).is_ok()
                } else {
                    false
                }
            });

            if !sent {
                return Err(JsError::from_opaque(JsValue::String(js_string!("Host channel closed"))));
            }

            match resp_rx.recv() {
                Ok(Ok(val)) => {
                    let out_json = serde_json::to_string(&val).unwrap_or_else(|_| "null".to_string());
                    Ok(JsValue::String(js_string!(out_json)))
                }
                Ok(Err(e)) => Err(JsError::from_opaque(JsValue::String(js_string!(e)))),
                Err(_) => Err(JsError::from_opaque(JsValue::String(js_string!("Bridge request cancelled")))),
            }
        });
        let host_op_obj = FunctionObjectBuilder::new(context.realm(), host_op_fn)
            .name(js_string!("__host_op"))
            .length(2)
            .build();
        context.register_global_property(js_string!("__host_op"), host_op_obj, Attribute::all()).unwrap();

        // Register __host_icon
        let host_icon_fn = NativeFunction::from_copy_closure(|_this, args, _ctx| {
            let name = args.first().and_then(|v| v.as_string()).map(|s| s.to_std_string_escaped()).unwrap_or_default();
            let opts_json = args.get(1).and_then(|v| v.as_string()).map(|s| s.to_std_string_escaped()).unwrap_or_else(|| "{}".to_string());
            let opts: Value = serde_json::from_str(&opts_json).unwrap_or(json!({}));

            let (resp_tx, resp_rx) = sync_channel();
            let sent = HOST_TX.with(|cell| {
                if let Some(tx) = cell.borrow().as_ref() {
                    tx.send(HostRequest::Icon { name, opts, resp: resp_tx }).is_ok()
                } else {
                    false
                }
            });

            if !sent {
                return Err(JsError::from_opaque(JsValue::String(js_string!("Host channel closed"))));
            }

            match resp_rx.recv() {
                Ok(Ok(val)) => {
                    let out_json = serde_json::to_string(&val).unwrap_or_else(|_| "null".to_string());
                    Ok(JsValue::String(js_string!(out_json)))
                }
                Ok(Err(e)) => Err(JsError::from_opaque(JsValue::String(js_string!(e)))),
                Err(_) => Err(JsError::from_opaque(JsValue::String(js_string!("Bridge icon request cancelled")))),
            }
        });
        let host_icon_obj = FunctionObjectBuilder::new(context.realm(), host_icon_fn)
            .name(js_string!("__host_icon"))
            .length(2)
            .build();
        context.register_global_property(js_string!("__host_icon"), host_icon_obj, Attribute::all()).unwrap();

        // Register __host_image
        let host_img_fn = NativeFunction::from_copy_closure(|_this, args, _ctx| {
            let url = args.first().and_then(|v| v.as_string()).map(|s| s.to_std_string_escaped()).unwrap_or_default();
            let opts_json = args.get(1).and_then(|v| v.as_string()).map(|s| s.to_std_string_escaped()).unwrap_or_else(|| "{}".to_string());
            let opts: Value = serde_json::from_str(&opts_json).unwrap_or(json!({}));

            let (resp_tx, resp_rx) = sync_channel();
            let sent = HOST_TX.with(|cell| {
                if let Some(tx) = cell.borrow().as_ref() {
                    tx.send(HostRequest::Image { url, opts, resp: resp_tx }).is_ok()
                } else {
                    false
                }
            });

            if !sent {
                return Err(JsError::from_opaque(JsValue::String(js_string!("Host channel closed"))));
            }

            match resp_rx.recv() {
                Ok(Ok(val)) => {
                    let out_json = serde_json::to_string(&val).unwrap_or_else(|_| "null".to_string());
                    Ok(JsValue::String(js_string!(out_json)))
                }
                Ok(Err(e)) => Err(JsError::from_opaque(JsValue::String(js_string!(e)))),
                Err(_) => Err(JsError::from_opaque(JsValue::String(js_string!("Bridge image request cancelled")))),
            }
        });
        let host_img_obj = FunctionObjectBuilder::new(context.realm(), host_img_fn)
            .name(js_string!("__host_image"))
            .length(2)
            .build();
        context.register_global_property(js_string!("__host_image"), host_img_obj, Attribute::all()).unwrap();

        let prelude = r##"
const console = {
    log: (...args) => __host_console("log", args.map(x => typeof x === "object" ? JSON.stringify(x, null, 2) : String(x)).join(" ")),
    error: (...args) => __host_console("error", args.map(x => typeof x === "object" ? JSON.stringify(x, null, 2) : String(x)).join(" ")),
    warn: (...args) => __host_console("warn", args.map(x => typeof x === "object" ? JSON.stringify(x, null, 2) : String(x)).join(" ")),
    info: (...args) => __host_console("info", args.map(x => typeof x === "object" ? JSON.stringify(x, null, 2) : String(x)).join(" ")),
};

function __callHost(op, params = {}) {
    try {
        const raw = __host_op(op, JSON.stringify(params));
        return JSON.parse(raw);
    } catch (e) {
        throw new Error(String(e));
    }
}

const ALL_OPS = [
    "status", "listPages", "setPage", "createPage",
    "query", "create", "modify", "delete", "append",
    "listComponents", "instantiate",
    "ensure_library", "get_library_tokens",
    "createVariableCollection", "createVariable", "applyVariable",
    "modifyVariable", "setupDesignTokens",
    "addVariableMode", "renameVariableMode", "removeVariableMode", "setVariableValue",
    "setFrameVariableMode", "clearFrameVariableMode",
    "createPaintStyle", "createTextStyle", "createComponent",
    "applyTextStyle",
    "clone", "group", "ungroup", "flatten", "resize",
    "set_selection", "set_viewport", "batch",
    "setReactions", "removeReactions",
    "setScrollBehavior",
    "setComponentProperties", "swapComponent",
    "addComponentProperty", "bindComponentPropertyToText", "removeComponentProperty",
    "bindComponentProperty", "unbindComponentProperty",
    "loadAllPagesAsync",
    "get_selection", "get_design", "get_page_nodes",
    "screenshot", "export_svg",
    "get_styles", "get_local_components", "get_viewport", "get_variables",
    "get_node_detail", "export_image", "search_nodes", "scan_design",
    "getReactions", "getComponentProperties"
];

const figma = {
    notify: (msg) => Promise.resolve(msg),
};

for (const op of ALL_OPS) {
    figma[op] = async (params = {}) => __callHost(op, params);
}

figma.getNodeById = async (id) => __callHost("get_node_detail", { id });
figma.getNode = async (id) => __callHost("get_node_detail", { id });
figma.getChildren = async (nodeId) => {
    const detail = __callHost("get_node_detail", { id: nodeId, depth: 1 });
    return (detail && Array.isArray(detail.children)) ? detail.children : [];
};
figma.zoom_to_fit = async (opts = {}) => {
    const nodeIds = opts.nodeIds || (opts.nodeId ? [opts.nodeId] : []);
    return __callHost("set_viewport", { nodeId: nodeIds[0] || null });
};
figma.getCurrentPage = async () => __callHost("status", {});
figma.get_page_nodes = async () => {
    const raw = __callHost("get_page_nodes", {});
    const arr = Array.isArray(raw) ? raw : (raw && Array.isArray(raw.nodes) ? raw.nodes : []);
    return arr;
};

figma.loadImage = async (url, opts = {}) => {
    try {
        const raw = __host_image(url, JSON.stringify(opts));
        return JSON.parse(raw);
    } catch (e) {
        throw new Error(String(e));
    }
};

figma.loadIcon = async (iconName, opts = {}) => {
    try {
        const raw = __host_icon(iconName, JSON.stringify(opts));
        return JSON.parse(raw);
    } catch (e) {
        throw new Error(String(e));
    }
};

figma.loadIconIn = async (iconName, opts = {}) => {
    const cSize = opts.containerSize || 40;
    const fill = opts.fill || "#6C5CE7";
    const bgOpacity = opts.bgOpacity !== undefined ? opts.bgOpacity : 0.1;
    const iSize = opts.iconSize || Math.floor(cSize / 2);

    if (opts.noContainer) {
        await figma.loadIcon(iconName, {
            parentId: opts.parentId,
            size: iSize,
            fill,
        });
        return { id: opts.parentId };
    }

    const createParams = {
        type: "FRAME",
        name: opts.name || ("icon-" + iconName + "-wrap"),
        parentId: opts.parentId,
        x: opts.x !== undefined ? opts.x : 0,
        y: opts.y !== undefined ? opts.y : 0,
        width: cSize,
        height: cSize,
        fill,
        fillOpacity: bgOpacity,
        cornerRadius: Math.floor(cSize / 2),
        layoutMode: "HORIZONTAL",
        primaryAxisAlignItems: "CENTER",
        counterAxisAlignItems: "CENTER",
    };
    if (opts.layoutAlign !== undefined) createParams.layoutAlign = opts.layoutAlign;
    if (opts.layoutGrow !== undefined) createParams.layoutGrow = opts.layoutGrow;

    const container = await figma.create(createParams);
    await figma.loadIcon(iconName, {
        parentId: container.id,
        size: iSize,
        fill,
    });
    return container;
};
"##;

        if let Err(e) = context.eval(Source::from_bytes(prelude)) {
            return (false, None, Some(format!("Sandbox initialization failed: {}", e)));
        }

        let wrapped_code = format!("(async () => {{\n{}\n}})()", code_owned);
        let eval_res = context.eval(Source::from_bytes(&wrapped_code));

        match eval_res {
            Ok(js_val) => {
                let result_val: Option<Value> = if js_val.is_undefined() || js_val.is_null() {
                    None
                } else if let Some(s) = js_val.as_string() {
                    Some(Value::String(s.to_std_string_escaped()))
                } else if let Some(b) = js_val.as_boolean() {
                    Some(Value::Bool(b))
                } else {
                    js_val.as_number().map(|n| json!(n))
                };

                (true, result_val, None)
            }
            Err(e) => {
                let mut err_msg = e.to_string();
                if err_msg.contains("ReferenceError") {
                    err_msg.push_str("\nNote: Each figma_write call runs in an isolated sandbox — variables from previous calls are not available. Re-query node IDs with figma.get_page_nodes() or figma.query() at the start of each call.");
                }
                (false, None, Some(err_msg))
            }
        }
    });

    let (success, result, error) = eval_task.await.unwrap_or_else(|e| (false, None, Some(format!("Execution task panicked: {}", e))));
    host_handler.abort();

    let captured_logs = logs.lock().unwrap().clone();

    ExecResult {
        success,
        result,
        error,
        logs: captured_logs,
    }
}
