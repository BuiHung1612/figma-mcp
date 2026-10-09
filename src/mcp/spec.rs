//! Bounded specs over the canonical reader; raw records remain available by ID.
use crate::bridge::BridgeHandle;
use super::protocol::ToolResult;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub(super) fn coverage(data: &Value) -> Value {
    let nodes = data["nodes"].as_array().map(Vec::as_slice).unwrap_or_default();
    let opaque = nodes.iter().filter(|n| n["opaque"] == true).count();
    let unexpanded = nodes.iter().filter(|n| n["childrenLoaded"] == false && n["opaque"] != true).count();
    json!({"nodesReturned":nodes.len(), "textsReturned":nodes.iter().filter(|n| n["type"] == "TEXT").count(),
        "traversalComplete":data["complete"], "subtreeComplete":data["complete"] == true && opaque == 0 && unexpanded == 0,
        "opaqueInstances":opaque,"unexpandedNodes":unexpanded,"budgetReached":data["budgetReached"],
        "scope":data["scope"],"revision":data["revision"],"componentResolution":data["componentResolution"]})
}

// Exact repeated property bundles only: identity, topology and copy stay inline.
pub(super) fn compact(data: &Value) -> Value {
    if data.get("templates").is_some() || data.get("decode").is_some() { return data.clone(); }
    let mut result = super::semantic_optimizer::compress_tree(data, true);
    let Some(nodes) = result["nodes"].as_array() else { return result };
    if nodes.iter().any(|n| n.get("templateRef").is_some()) { return result; }
    let bundle = |n: &Value| -> Value {
        Value::Object(n.as_object().into_iter().flatten().filter(|(k,_)|
            !["id","parentId","childIds","name","content","x","y","width","height","templateRef"].contains(&k.as_str()))
            .map(|(k,v)|(k.clone(),v.clone())).collect())
    };
    let mut counts = BTreeMap::<String,usize>::new();
    let mut components = BTreeMap::<String,(Map<String,Value>,usize)>::new();
    for n in nodes {
        let properties = bundle(n);
        *counts.entry(properties.to_string()).or_default() += 1;
        if let Some(component) = n["componentId"].as_str() {
            let properties = properties.as_object().expect("bundle");
            let entry = components.entry(component.into()).or_insert((properties.clone(),0));
            entry.0.retain(|k,v| properties.get(k) == Some(v));
            entry.1 += 1;
        }
    }
    let mut refs = BTreeMap::new();
    let mut component_refs = BTreeMap::new();
    let mut templates = Map::new();
    for (key,count) in counts {
        let id = format!("t{}",templates.len());
        if count > 1 && (count-1)*key.len() > count*(id.len()+20)+id.len()+8 {
            templates.insert(id.clone(),serde_json::from_str(&key).expect("serialized template"));
            refs.insert(key,id);
        }
    }
    for (component,(properties,count)) in components {
        let size = Value::Object(properties.clone()).to_string().len();
        let id = format!("t{}",templates.len());
        if count > 1 && (count-1)*size > count*(id.len()+20)+id.len()+8 {
            templates.insert(id.clone(),Value::Object(properties));
            component_refs.insert(component,id);
        }
    }
    if templates.is_empty() { return result; }
    let original = result.clone();
    for n in result["nodes"].as_array_mut().expect("node array") {
        let properties = bundle(n);
        let reference = refs.get(&properties.to_string()).cloned().or_else(||
            n["componentId"].as_str().and_then(|id| component_refs.get(id)).cloned());
        if let Some(id) = reference {
            let obj = n.as_object_mut().expect("node object");
            for key in templates[&id].as_object().expect("template").keys() { obj.remove(key); }
            obj.insert("templateRef".into(),json!(id));
        }
    }
    let used: std::collections::BTreeSet<_> = result["nodes"].as_array().expect("nodes").iter().filter_map(|n| n["templateRef"].as_str().map(str::to_owned)).collect();
    templates.retain(|id,_| used.contains(id));
    result["templates"] = Value::Object(templates);
    result["decode"] = json!("Merge templates[node.templateRef] then inline node fields; remove templateRef. Then recursively merge _compression.styles[object.styleRef] on every referenced object and remove styleRef. Geometry, identity, copy and mixed text are exact.");
    if result.to_string().len() < original.to_string().len() { result } else { original }
}

pub(super) async fn read_all(bridge: &BridgeHandle, mut params: Value, session: Option<&str>) -> Result<Value,String> {
    let mut nodes = Vec::new();
    let mut resolved = 0;
    let mut components_truncated = false;
    loop {
        let mut page = bridge.send_operation("read_nodes",params,session).await?;
        resolved += page["componentResolution"]["resolved"].as_u64().unwrap_or(0);
        components_truncated |= page["componentResolution"]["truncated"] == true;
        nodes.extend(page["nodes"].as_array().ok_or("Reader returned no nodes")?.iter().cloned());
        if let Some(cursor) = page["nextCursor"].as_str() {
            params = json!({"cursor":cursor,"limit":500});
        } else {
            page["nodes"] = json!(nodes);
            page["componentResolution"] = json!({"resolved":resolved,"truncated":components_truncated});
            return Ok(page);
        }
    }
}

pub(super) async fn get_spec(bridge: BridgeHandle, args: Value) -> ToolResult {
    let mode = args["mode"].as_str().unwrap_or("overview");
    if !["overview","spec","detail"].contains(&mode) { return ToolResult::error("mode must be overview, spec or detail"); }
    let session = args["sessionId"].as_str();
    let mut params = json!({});
    if let Some(obj) = args.as_object() {
        for (k,v) in obj {
            match k.as_str() {
                "nodeId" => params["id"] = v.clone(),
                "nodeName" => params["name"] = v.clone(),
                "cursor"|"depth"|"fields"|"includeHidden"|"expandInstances"|"limit"|"maxBytes" => params[k] = v.clone(),
                _ => {}
            }
        }
    }
    if params.get("cursor").is_none() {
        if params.get("depth").is_none() { params["depth"] = json!(if mode == "overview" { 2 } else if mode == "detail" { 0 } else { 256 }); }
        if params.get("fields").is_none() { params["fields"] = if mode == "overview" { json!(["geometry","content","layout","component"]) } else { json!(["geometry","content","text","style","layout","tokens","component"]) }; }
        if params.get("expandInstances").is_none() { params["expandInstances"] = json!(true); }
    }
    if params.get("maxBytes").is_none() { params["maxBytes"] = json!(24000); }
    let data = if args.get("outputPath").is_some() {
        if params.get("cursor").is_some() { return ToolResult::error("outputPath requires a new read, not a cursor"); }
        read_all(&bridge,params,session).await
    } else { bridge.send_operation("read_nodes",params,session).await };
    let data = match data { Ok(data) => data, Err(e) => return ToolResult::error(e) };
    let coverage = coverage(&data);
    if let Some(path) = args["outputPath"].as_str() {
        let path = std::path::Path::new(path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            if let Err(e) = tokio::fs::create_dir_all(parent).await { return ToolResult::error(e.to_string()); }
        }
        if let Err(e) = tokio::fs::write(path,data.to_string()).await { return ToolResult::error(e.to_string()); }
        return ToolResult::text(json!({"savedTo":std::fs::canonicalize(path).unwrap_or(path.to_path_buf()),"coverage":coverage,
            "hint":"Raw records saved outside context. Read a section with nodeId + mode:spec, or one node with mode:detail."}).to_string());
    }
    let raw_bytes = data.to_string().len();
    let mut out = compact(&data);
    out["coverage"] = coverage;
    out["rawBytes"] = json!(raw_bytes);
    out["hint"] = json!("Use overview node IDs to read sections individually with mode:spec; continue nextCursor with the same sessionId. maxBytes targets raw node bytes; one oversized node, metadata and component resolution may exceed it.");
    ToolResult::text(out.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn restore(mut out: Value) -> Value {
        let templates = out.as_object_mut().unwrap().remove("templates").unwrap_or(json!({}));
        let styles = out.as_object_mut().unwrap().remove("_compression").unwrap_or(json!({}));
        out.as_object_mut().unwrap().remove("decode");
        for n in out["nodes"].as_array_mut().unwrap() {
            if let Some(id) = n.as_object_mut().unwrap().remove("templateRef") {
                let mut properties = templates[id.as_str().unwrap()].as_object().unwrap().clone();
                properties.extend(n.as_object().unwrap().clone()); *n = Value::Object(properties);
            }
        }
        fn restore_styles(value: &mut Value, styles: &Value) {
            match value {
                Value::Object(obj) => {
                    if let Some(id) = obj.remove("styleRef") {
                        obj.extend(styles["styles"][id.as_str().unwrap()].as_object().unwrap().clone());
                    }
                    for child in obj.values_mut() { restore_styles(child,styles); }
                }
                Value::Array(items) => for item in items { restore_styles(item,styles); },
                _ => {}
            }
        }
        restore_styles(&mut out,&styles);
        out
    }

    #[tokio::test]
    async fn spec_export_collects_pages_and_returns_only_coverage() {
        let state = crate::bridge::server::BridgeState::new(0);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut session = crate::bridge::session::Session::new("spec-session".into(),None);
        session.ws_tx = Some(tx);
        let params = json!({"id":"root","depth":256,"fields":["geometry","content","text","style","layout","tokens","component"],"expandInstances":true,"maxBytes":24000});
        state.inner.lock().await.sessions.insert("spec-session".into(),session);
        let responder_state = state.clone();
        let responder = tokio::spawn(async move {
            for (expected,page) in [
                (params,json!({"nodes":[{"id":"root","type":"FRAME","childrenLoaded":true}],"nextCursor":"page2","complete":false})),
                (json!({"cursor":"page2","limit":500}),json!({"nodes":[{"id":"text","type":"TEXT","content":"Override","fontSize":15.5}],"nextCursor":null,"complete":true}))
            ] {
                let message = tokio::time::timeout(std::time::Duration::from_secs(2),rx.recv()).await.unwrap().unwrap();
                let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                assert_eq!(request["params"],expected);
                let mut inner = responder_state.inner.lock().await;
                inner.sessions.get_mut("spec-session").unwrap().pending.remove(request["id"].as_str().unwrap()).unwrap().sender.send(Ok(page)).unwrap();
            }
        });
        let path = std::env::temp_dir().join(format!("figma-spec-test-{}.json",std::process::id()));
        let result = get_spec(BridgeHandle::Direct(state),json!({"nodeId":"root","mode":"spec","sessionId":"spec-session","outputPath":path})).await;
        let serialized = serde_json::to_value(result).unwrap();
        let payload: Value = serde_json::from_str(serialized["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(payload["coverage"]["nodesReturned"],2);
        assert_eq!(payload["coverage"]["subtreeComplete"],true);
        assert!(payload.get("nodes").is_none());
        let saved: Value = serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        assert_eq!(saved["nodes"][1]["fontSize"],15.5);
        responder.await.unwrap();
        tokio::fs::remove_file(path).await.unwrap();
    }

    #[test]
    fn templates_roundtrip_mixed_text_geometry_and_overrides() {
        let raw = json!({"nodes":(0..80).map(|i|json!({"id":format!("i:{i}"),"parentId":"root","childIds":[],"name":"Label","type":"TEXT",
            "content":format!("Override {i}"),"x":i,"width":15.5,"fontFamily":"Lato","fontSize":15.5,"fontWeight":"Regular",
            "segments":[{"start":0,"end":3,"fontSize":22.25,"fontWeight":"Demi"}],"componentId":"c:1","variant":{"State":if i == 0 {"Disabled"} else {"Active"}},"props":{"Text":format!("Instance {i}")}})).collect::<Vec<_>>()});
        let out = compact(&raw);
        assert!(out.to_string().len() < raw.to_string().len());
        assert_eq!(restore(out),raw);
        if let Ok(path) = std::env::var("FIGMA_SPEC_BENCH") {
            let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let encoded = compact(&fixture);
            eprintln!("MICELLE spec: {} -> {} bytes",fixture.to_string().len(),encoded.to_string().len());
            assert!(restore(encoded) == fixture, "fixture must reconstruct exactly");
        }
        assert_eq!(coverage(&json!({"complete":true,"nodes":[{"opaque":true}]}))["subtreeComplete"],false);
        assert_eq!(coverage(&json!({"complete":true,"nodes":[{"childrenLoaded":false}]}))["subtreeComplete"],false);
    }
}
