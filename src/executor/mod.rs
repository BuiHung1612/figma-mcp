pub mod assets;

use crate::bridge::BridgeHandle;
use assets::fetch_svg_icon;
use base64::Engine;
use boa_engine::{
    builtins::promise::PromiseState,
    js_string,
    native_function::NativeFunction,
    object::FunctionObjectBuilder,
    object::builtins::JsPromise,
    property::Attribute,
    Context, JsError, JsValue, Source,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, OnceLock};

/// Per-loop iteration cap so a runaway `while (true) {}` errors out early.
pub const LOOP_ITERATION_LIMIT: u64 = 1_000_000;
/// Wall-clock cap for one figma_write script, including bridge round-trips.
pub const SCRIPT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(300);
/// Heap cap for the script worker process (see `crate::CappedAlloc`).
pub const WORKER_MEMORY_BYTES: usize = 1 << 30;
const MAX_LOG_LINES: usize = 200;
const MAX_LOG_BYTES: usize = 16 << 10;

/// A call from the script to the host. Crosses the worker's stdin/stdout as one JSON line.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "lowercase")]
pub enum HostCall {
    Op { operation: String, params: Value },
    Icon { name: String, opts: Value },
    Image { url: String, opts: Value },
    Log { level: String, msg: String },
}

type Host = Box<dyn Fn(HostCall) -> Result<Value, String>>;

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

fn call_host(call: HostCall) -> Result<Value, String> {
    HOST.with(|cell| match cell.borrow().as_ref() {
        Some(host) => host(call),
        None => Err("Host channel closed".to_string()),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub success: bool,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub logs: Vec<String>,
}

type ScriptOutcome = (bool, Option<Value>, Option<String>);

#[derive(Default)]
struct Logs {
    lines: Vec<String>,
    bytes: usize,
    dropped: usize,
}

// Shared HTTP client — reused across all execute_code calls so connection
// pooling is preserved and TLS handshakes are not repeated per call.
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_max_idle_per_host(4)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to build reqwest client")
    })
}

/// Converts a script result through JSON.stringify, which rejects cycles with a TypeError;
/// JsValue::to_json recurses into a cycle until the stack overflows and aborts the daemon.
fn js_to_json(value: &JsValue, context: &mut Context) -> Result<Option<Value>, String> {
    // Boa's native stringify recurses per level too, so a deep (or cyclic) value is
    // measured iteratively first. serde_json rejects more than 128 levels anyway.
    let mut stack = vec![(value.clone(), 0usize)];
    let mut visited = 0usize;
    while let Some((item, depth)) = stack.pop() {
        let Some(object) = item.as_object() else { continue };
        if depth >= 128 { return Err("nested more than 128 levels (or cyclic)".into()); }
        visited += 1;
        if visited > 1_000_000 { return Err("more than 1,000,000 objects".into()); }
        for key in object.own_property_keys(context).map_err(|e| e.to_string())? {
            let child = object.get(key, context).map_err(|e| e.to_string())?;
            if child.is_object() { stack.push((child, depth + 1)); }
        }
    }
    let json = context.global_object().get(js_string!("JSON"), context).map_err(|e| e.to_string())?;
    let stringify = json.as_object()
        .and_then(|object| object.get(js_string!("stringify"), context).ok())
        .and_then(|function| function.as_callable().cloned())
        .ok_or("JSON.stringify is unavailable")?;
    let text = stringify.call(&json, std::slice::from_ref(value), context).map_err(|e| e.to_string())?;
    match text.as_string() {
        Some(text) => serde_json::from_str(&text.to_std_string_escaped()).map(Some).map_err(|e| e.to_string()),
        None => Ok(None), // functions and symbols stringify to undefined
    }
}

/// Runs one host call from a script: bridge ops, icon/image downloads, logs.
async fn handle_host(call: HostCall, bridge: &BridgeHandle, sid: Option<&str>, logs: &Mutex<Logs>) -> Result<Value, String> {
    let http_client = http_client();
    match call {
        HostCall::Op { operation, params } => bridge.send_operation(&operation, params, sid).await,
        HostCall::Icon { name, opts } => {
            let size = opts.get("size").and_then(|v| v.as_f64()).unwrap_or(24.0);
            let fill = opts.get("fill").and_then(|v| v.as_str()).unwrap_or("#1E3150").to_string();
            let svg = fetch_svg_icon(http_client, &name, size, &fill).await?;
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
            bridge.send_operation("create", create_params, sid).await
        }
        HostCall::Image { url, opts } => {
            let bytes = match http_client.get(&url).send().await {
                Ok(r) if r.status().is_success() => assets::read_capped(r, assets::MAX_IMAGE_BYTES).await
                    .map_err(|e| format!("Failed to read image bytes: {}", e))?,
                Ok(r) => return Err(format!("Image download failed with HTTP {}", r.status())),
                Err(e) => return Err(format!("Failed to fetch image: {}", e)),
            };
            let mut create_params = json!({
                "type": "IMAGE",
                "name": opts.get("name").and_then(|v| v.as_str()).unwrap_or("image"),
                "width": opts.get("width").and_then(|v| v.as_f64()).unwrap_or(100.0),
                "height": opts.get("height").and_then(|v| v.as_f64()).unwrap_or(100.0),
                "imageData": base64::engine::general_purpose::STANDARD.encode(&bytes),
                "scaleMode": opts.get("scaleMode").and_then(|v| v.as_str()).unwrap_or("FILL"),
            });
            if let Some(pid) = opts.get("parentId") { create_params["parentId"] = pid.clone(); }
            if let Some(x) = opts.get("x") { create_params["x"] = x.clone(); }
            if let Some(y) = opts.get("y") { create_params["y"] = y.clone(); }
            if let Some(cr) = opts.get("cornerRadius") { create_params["cornerRadius"] = cr.clone(); }
            bridge.send_operation("create", create_params, sid).await
        }
        HostCall::Log { level, msg } => {
            let line = match level.as_str() {
                "error" | "warn" | "info" => format!("[{}] {}", level, msg),
                _ => msg,
            };
            let mut logs = logs.lock().unwrap_or_else(|e| e.into_inner());
            if logs.lines.len() < MAX_LOG_LINES && logs.bytes + line.len() <= MAX_LOG_BYTES {
                logs.bytes += line.len();
                logs.lines.push(line);
            } else {
                logs.dropped += 1;
            }
            Ok(Value::Null)
        }
    }
}

pub async fn execute_code(
    code: &str,
    bridge: BridgeHandle,
    session_id: Option<String>,
) -> ExecResult {
    let logs = Arc::new(Mutex::new(Logs::default()));
    let run = async {
        // Unit tests run in-process: the test binary cannot act as the worker.
        if !cfg!(test) {
            if let Some(child) = spawn_worker() {
                return run_in_worker(child, code, &bridge, session_id.as_deref(), &logs).await;
            }
        }
        run_in_process(code.to_string(), bridge.clone(), session_id.clone(), logs.clone()).await
    };
    // Dropping a timed-out worker kills it (kill_on_drop), so a runaway script stops using CPU.
    let (success, result, error) = tokio::time::timeout(SCRIPT_DEADLINE, run).await
        .unwrap_or_else(|_| (false, None, Some(format!("Script exceeded the {}s deadline", SCRIPT_DEADLINE.as_secs()))));

    let logs = std::mem::take(&mut *logs.lock().unwrap_or_else(|e| e.into_inner()));
    let mut lines = logs.lines;
    if logs.dropped > 0 { lines.push(format!("…{} more lines", logs.dropped)); }
    ExecResult { success, result, error, logs: lines }
}

/// Scripts run in a child process so one that exhausts memory, overflows the stack or
/// outlives the deadline takes down only itself, never the daemon.
fn spawn_worker() -> Option<tokio::process::Child> {
    let exe = std::env::current_exe().ok()?;
    let mut command = tokio::process::Command::new(exe);
    command.arg("--script-worker")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: the daemon has no console to share
    command.spawn().ok()
}

async fn run_in_worker(mut child: tokio::process::Child, code: &str, bridge: &BridgeHandle, sid: Option<&str>, logs: &Mutex<Logs>) -> ScriptOutcome {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    let (Some(mut stdin), Some(stdout), Some(mut stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        return (false, None, Some("Script worker has no stdio".into()));
    };
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let send = |value: Value| format!("{value}\n");
    if stdin.write_all(send(json!({"code": code})).as_bytes()).await.is_err() {
        return (false, None, Some("Script worker did not start".into()));
    }
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(done) = message.get("done") {
            return (done["success"].as_bool().unwrap_or(false), done.get("result").filter(|v| !v.is_null()).cloned(), done["error"].as_str().map(str::to_owned));
        }
        let Ok(call) = serde_json::from_value::<HostCall>(message) else { continue };
        let is_log = matches!(call, HostCall::Log { .. });
        let reply = match handle_host(call, bridge, sid, logs).await {
            Ok(value) => json!({"ok": value}),
            Err(error) => json!({"err": error}),
        };
        if !is_log && stdin.write_all(send(reply).as_bytes()).await.is_err() { break; }
    }
    // The worker ended without a result: it crashed. Its stderr says why.
    let mut detail = String::new();
    let _ = (&mut stderr).take(4096).read_to_string(&mut detail).await;
    let reason = if detail.contains("memory allocation") {
        format!("Script ran out of memory ({} MB limit)", WORKER_MEMORY_BYTES >> 20)
    } else if detail.contains("overflow") {
        "Script overflowed the stack (too deep recursion or nesting)".to_string()
    } else {
        format!("Script worker crashed: {}", detail.trim())
    };
    (false, None, Some(reason))
}

async fn run_in_process(code: String, bridge: BridgeHandle, sid: Option<String>, logs: Arc<Mutex<Logs>>) -> ScriptOutcome {
    type Request = (HostCall, std::sync::mpsc::Sender<Result<Value, String>>);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
    let handler = tokio::spawn(async move {
        while let Some((call, resp)) = rx.recv().await {
            let _ = resp.send(handle_host(call, &bridge, sid.as_deref(), &logs).await);
        }
    });
    let outcome = tokio::task::spawn_blocking(move || {
        run_script(&code, Box::new(move |call| {
            let (resp_tx, resp_rx) = std::sync::mpsc::channel();
            tx.send((call, resp_tx)).map_err(|_| "Host channel closed".to_string())?;
            resp_rx.recv().map_err(|_| "Bridge request cancelled".to_string())?
        }))
    }).await.unwrap_or_else(|e| (false, None, Some(format!("Execution task panicked: {}", e))));
    let _ = handler.await;
    outcome
}

/// Entry point of `figma-rust-mcp --script-worker`: reads `{"code"}` from stdin, answers host calls
/// over stdin/stdout, and prints `{"done": …}`. Nothing else may write to stdout.
pub fn run_worker() {
    fn read_line(input: &mut impl BufRead) -> Option<Value> {
        let mut line = String::new();
        input.read_line(&mut line).ok().filter(|n| *n > 0)?;
        serde_json::from_str(&line).ok()
    }
    fn write_line(value: &Value) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{value}");
        let _ = out.flush();
    }
    let mut input = std::io::stdin().lock();
    let Some(code) = read_line(&mut input).and_then(|v| v["code"].as_str().map(str::to_owned)) else { return };
    let input = RefCell::new(input);
    let outcome = run_script(&code, Box::new(move |call| {
        let is_log = matches!(call, HostCall::Log { .. });
        write_line(&serde_json::to_value(&call).map_err(|e| e.to_string())?);
        if is_log { return Ok(Value::Null); }
        let reply = read_line(&mut *input.borrow_mut()).ok_or("Host closed the channel")?;
        match reply.get("err").and_then(Value::as_str) {
            Some(error) => Err(error.to_string()),
            None => Ok(reply.get("ok").cloned().unwrap_or(Value::Null)),
        }
    }));
    let (success, result, error) = outcome;
    write_line(&json!({"done": {"success": success, "result": result, "error": error}}));
}

fn arg_string(args: &[JsValue], index: usize) -> String {
    args.get(index).and_then(|v| v.as_string()).map(|s| s.to_std_string_escaped()).unwrap_or_default()
}

fn arg_json(args: &[JsValue], index: usize) -> Value {
    args.get(index).and_then(|v| v.as_string()).and_then(|s| serde_json::from_str(&s.to_std_string_escaped()).ok()).unwrap_or_else(|| json!({}))
}

/// Host results reach JS as a JSON string the prelude parses; errors become exceptions.
fn host_result(result: Result<Value, String>) -> Result<JsValue, JsError> {
    match result {
        Ok(value) => Ok(JsValue::String(js_string!(serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string())))),
        Err(error) => Err(JsError::from_opaque(JsValue::String(js_string!(error)))),
    }
}

fn register(context: &mut Context, name: &'static str, function: NativeFunction) {
    let object = FunctionObjectBuilder::new(context.realm(), function).name(js_string!(name)).length(2).build();
    context.register_global_property(js_string!(name), object, Attribute::all()).expect("fresh context has no host globals");
}

/// Runs a script synchronously against `host`; used by the worker and the in-process fallback.
fn run_script(code: &str, host: Host) -> ScriptOutcome {
    HOST.with(|cell| *cell.borrow_mut() = Some(host));
    let outcome = eval_script(code);
    HOST.with(|cell| *cell.borrow_mut() = None);
    outcome
}

fn eval_script(code: &str) -> ScriptOutcome {
    let mut context = Context::default();
    context.runtime_limits_mut().set_loop_iteration_limit(LOOP_ITERATION_LIMIT);

    register(&mut context, "__host_console", NativeFunction::from_copy_closure(|_this, args, _ctx| {
        let _ = call_host(HostCall::Log { level: arg_string(args, 0), msg: arg_string(args, 1) });
        Ok(JsValue::undefined())
    }));
    register(&mut context, "__host_op", NativeFunction::from_copy_closure(|_this, args, _ctx| {
        host_result(call_host(HostCall::Op { operation: arg_string(args, 0), params: arg_json(args, 1) }))
    }));
    register(&mut context, "__host_icon", NativeFunction::from_copy_closure(|_this, args, _ctx| {
        host_result(call_host(HostCall::Icon { name: arg_string(args, 0), opts: arg_json(args, 1) }))
    }));
    register(&mut context, "__host_image", NativeFunction::from_copy_closure(|_this, args, _ctx| {
        host_result(call_host(HostCall::Image { url: arg_string(args, 0), opts: arg_json(args, 1) }))
    }));

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
    "get_node_detail", "read_nodes", "export_image", "search_nodes", "scan_design",
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

    let wrapped_code = format!("(async () => {{\n{}\n}})()", code);
    let eval_res = context.eval(Source::from_bytes(&wrapped_code)).and_then(|value| {
        // Code can close the async wrapper early, leaving a non-promise completion value.
        let Some(object) = value.as_object() else { return Ok(value) };
        let promise = JsPromise::from_object(object.clone())?;
        context.run_jobs();
        match promise.state() {
            PromiseState::Fulfilled(value) => Ok(value),
            PromiseState::Rejected(error) => Err(JsError::from_opaque(error)),
            PromiseState::Pending => Err(JsError::from_opaque(JsValue::String(js_string!("Sandbox promise did not settle")))),
        }
    });

    match eval_res {
        Ok(js_val) => {
            let result_val: Option<Value> = if js_val.is_undefined() || js_val.is_null() {
                None
            } else if let Some(s) = js_val.as_string() {
                Some(Value::String(s.to_std_string_escaped()))
            } else if let Some(b) = js_val.as_boolean() {
                Some(Value::Bool(b))
            } else if let Some(n) = js_val.as_number() {
                Some(json!(n))
            } else {
                // The script's writes already ran, so report success (a failure invites a
                // duplicate re-run) but say why the value is missing instead of returning null.
                match js_to_json(&js_val, &mut context) {
                    Ok(value) => value,
                    Err(e) => Some(json!(format!("[result not JSON-serializable: {e}]"))),
                }
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
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn unserializable_result_is_explained_not_null() {
        let bridge = crate::bridge::BridgeHandle::Direct(crate::bridge::BridgeState::new(0));
        for code in ["const a = {}; a.self = a; return a;", "let a = []; for (let i = 0; i < 100000; i++) a = [a]; return a;"] {
            let res = super::execute_code(code, bridge.clone(), None).await;
            assert!(res.success);
            assert!(res.result.as_ref().and_then(|v| v.as_str()).is_some_and(|s| s.contains("not JSON-serializable")), "{:?}", res.result);
        }
        let res = super::execute_code("return { a: [1, { b: 'x' }], f() {} };", bridge, None).await;
        assert_eq!(res.result, Some(serde_json::json!({"a": [1, {"b": "x"}]})));
    }

    #[tokio::test]
    async fn code_escaping_the_async_wrapper_does_not_panic() {
        let bridge = crate::bridge::BridgeHandle::Direct(crate::bridge::BridgeState::new(0));
        let res = super::execute_code("})(); (() => {", bridge, None).await;
        assert!(res.error.as_deref().is_none_or(|e| !e.contains("panicked")), "{:?}", res.error);
    }

    #[tokio::test]
    async fn runaway_loop_fails_instead_of_hanging() {
        let bridge = crate::bridge::BridgeHandle::Direct(crate::bridge::BridgeState::new(0));
        let res = tokio::time::timeout(std::time::Duration::from_secs(30), super::execute_code("while (true) {}", bridge, None))
            .await
            .expect("sandbox must stop runaway loops");
        assert!(!res.success);
        assert!(res.error.unwrap_or_default().to_lowercase().contains("loop"));
    }

    #[tokio::test]
    async fn sandbox_supports_modern_syntax_documented_in_figma_docs() {
        let bridge = crate::bridge::BridgeHandle::Direct(crate::bridge::BridgeState::new(0));
        let res = super::execute_code("const o = {a: {b: 1}}; const c = {...o, d: 2}; const [x] = [o?.a?.b]; return [x, o.z ?? 5, c.d, `t${1}`, typeof setTimeout];", bridge, None).await;
        assert_eq!(res.result, Some(serde_json::json!([1, 5, 2, "t1", "undefined"])), "{:?}", res.error);
    }

    #[tokio::test]
    async fn returns_objects_keeps_trailing_logs_and_caps_log_volume() {
        let bridge = crate::bridge::BridgeHandle::Direct(crate::bridge::BridgeState::new(0));
        let res = super::execute_code("console.log('last'); return {a: [1, 'x']};", bridge.clone(), None).await;
        assert_eq!(res.result, Some(serde_json::json!({"a": [1, "x"]})));
        assert_eq!(res.logs, ["last"]);
        let res = super::execute_code("for (let i = 0; i < 300; i++) console.log(i);", bridge, None).await;
        assert_eq!(res.logs.len(), super::MAX_LOG_LINES + 1);
        assert_eq!(res.logs.last().unwrap(), "…100 more lines");
    }
}
