use obscura_js::runtime::RemoteObjectInfo;
use serde_json::{json, Value};

use crate::dispatch::CdpContext;

pub async fn handle(
    method: &str,
    params: &Value,
    ctx: &mut CdpContext,
    session_id: &Option<String>,
) -> Result<Value, String> {
    match method {
        "enable" => {
            let page = ctx.get_session_page(session_id).ok_or("No page")?;
            let event = crate::types::CdpEvent {
                method: "Runtime.executionContextCreated".to_string(),
                params: json!({
                    "context": {
                        "id": 1,
                        "origin": page.url_string(),
                        "name": "",
                        "uniqueId": format!("ctx-{}", page.id),
                        "auxData": {
                            "isDefault": true,
                            "type": "default",
                            "frameId": page.frame_id,
                        }
                    }
                }),
                session_id: session_id.clone(),
            };
            ctx.pending_events.push(event);
            Ok(json!({}))
        }
        "evaluate" => {
            let expression = params
                .get("expression")
                .and_then(|v| v.as_str())
                .ok_or("expression required")?;
            let return_by_value = params
                .get("returnByValue")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let page = ctx.get_session_page_mut(session_id).ok_or("No page")?;
            let info = page.evaluate_for_cdp(expression, return_by_value);

            Ok(json!({ "result": remote_object_from_info(&info) }))
        }
        "callFunctionOn" => {
            let function_declaration = params
                .get("functionDeclaration")
                .and_then(|v| v.as_str())
                .unwrap_or("() => undefined");
            let return_by_value = params
                .get("returnByValue")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let await_promise = params
                .get("awaitPromise")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let object_id = params.get("objectId").and_then(|v| v.as_str());
            let arguments = params
                .get("arguments")
                .and_then(|v| v.as_array())
                .map(|a| a.to_vec())
                .unwrap_or_default();

            if let Some(result) = try_playwright_selector_call(
                function_declaration,
                &arguments,
                return_by_value,
                ctx,
                session_id,
            ) {
                return Ok(json!({ "result": result }));
            }

            let page = ctx.get_session_page_mut(session_id).ok_or("No page")?;
            let info = page
                .call_function_on_for_cdp(
                    function_declaration,
                    object_id,
                    &arguments,
                    return_by_value,
                    await_promise,
                )
                .await;

            Ok(json!({ "result": remote_object_from_info(&info) }))
        }
        "getProperties" => {
            let object_id = params.get("objectId").and_then(|v| v.as_str());
            if let Some(oid) = object_id {
                let page = ctx.get_session_page_mut(session_id).ok_or("No page")?;
                let escaped_oid = oid.replace('\\', "\\\\").replace('\'', "\\'");
                let code = format!(
                    "(function() {{\
                        var obj = globalThis.__obscura_objects['{oid}'];\
                        if (!obj || typeof obj !== 'object') return [];\
                        return Object.keys(obj).map(function(k) {{\
                            var v = obj[k];\
                            return {{ name: k, value: v, type: typeof v }};\
                        }});\
                    }})()",
                    oid = escaped_oid,
                );
                let result = page.evaluate(&code);
                if let serde_json::Value::Array(props) = result {
                    let descriptors: Vec<Value> = props
                        .iter()
                        .map(|p| {
                            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let value = p.get("value").unwrap_or(&Value::Null);
                            let prop_type = p
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("undefined");
                            let mut remote = json!({
                                "type": prop_type,
                            });
                            match value {
                                Value::Null => {
                                    remote["type"] = json!("object");
                                    remote["subtype"] = json!("null");
                                    remote["value"] = json!(null);
                                }
                                Value::String(s) => {
                                    remote["type"] = json!("string");
                                    remote["value"] = json!(s);
                                }
                                Value::Number(n) => {
                                    remote["type"] = json!("number");
                                    remote["value"] = json!(n);
                                }
                                Value::Bool(b) => {
                                    remote["type"] = json!("boolean");
                                    remote["value"] = json!(b);
                                }
                                _ => {
                                    remote["value"] = value.clone();
                                }
                            }
                            json!({
                                "name": name,
                                "value": remote,
                                "configurable": true,
                                "enumerable": true,
                                "writable": true,
                                "isOwn": true,
                            })
                        })
                        .collect();
                    Ok(json!({ "result": descriptors, "internalProperties": [] }))
                } else {
                    Ok(json!({ "result": [], "internalProperties": [] }))
                }
            } else {
                Ok(json!({ "result": [], "internalProperties": [] }))
            }
        }
        "releaseObject" => {
            if let Some(oid) = params.get("objectId").and_then(|v| v.as_str()) {
                if let Some(page) = ctx.get_session_page_mut(session_id) {
                    page.release_object(oid);
                }
            }
            Ok(json!({}))
        }
        "releaseObjectGroup" => {
            if let Some(page) = ctx.get_session_page_mut(session_id) {
                page.release_object_group();
            }
            Ok(json!({}))
        }
        "addBinding" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if !name.is_empty() {
                if name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                    && !name.chars().next().unwrap_or('0').is_ascii_digit()
                {
                    if let Some(page) = ctx.get_session_page_mut(session_id) {
                        let code = format!(
                            "if (typeof globalThis.{name} === 'undefined') {{\
                                globalThis.{name} = function() {{ return null; }};\
                            }}",
                            name = name,
                        );
                        page.evaluate(&code);
                    }
                }
            }
            Ok(json!({}))
        }
        "runIfWaitingForDebugger" => Ok(json!({})),
        "getExceptionDetails" => Ok(json!({ "exceptionDetails": null })),
        "discardConsoleEntries" => Ok(json!({})),
        _ => Err(format!("Unknown Runtime method: {}", method)),
    }
}

fn remote_object_from_info(info: &RemoteObjectInfo) -> Value {
    let mut obj = json!({ "type": info.js_type });

    if let Some(ref subtype) = info.subtype {
        obj["subtype"] = json!(subtype);
    }

    if !info.class_name.is_empty() {
        obj["className"] = json!(info.class_name);
    }

    if !info.description.is_empty() {
        obj["description"] = json!(info.description);
    }

    if let Some(ref oid) = info.object_id {
        obj["objectId"] = json!(oid);
    }

    if let Some(ref value) = info.value {
        obj["value"] = value.clone();
    }

    obj
}

fn try_playwright_selector_call(
    function_declaration: &str,
    arguments: &[Value],
    return_by_value: bool,
    ctx: &mut CdpContext,
    session_id: &Option<String>,
) -> Option<Value> {
    if !function_declaration.contains("utilityScript.evaluate") {
        return None;
    }

    let expression_text = arguments
        .get(3)
        .and_then(|arg| arg.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let callback_text = arguments
        .get(6)
        .and_then(|arg| arg.get("value"))
        .and_then(|task| serialized_prop(task, "callbackText"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if expression_text == "(r) => ({ log: r.log, success: r.success })" {
        let oid = arguments
            .last()
            .and_then(|arg| arg.get("objectId"))
            .and_then(|v| v.as_str())?;
        let stored = ctx.selector_results.get(oid)?;
        return Some(json!({
            "type": "object",
            "className": "Object",
            "description": "Object",
            "value": stored
        }));
    }

    if expression_text == "(r) => r.element" {
        let oid = arguments
            .last()
            .and_then(|arg| arg.get("objectId"))
            .and_then(|v| v.as_str())?;
        let selector = selector_from_cached_oid(oid)?;
        let node_id = ctx
            .get_session_page(session_id)
            .and_then(|page| {
                page.with_dom(|dom| {
                    if selector == ":root" {
                        dom.children(dom.document())
                            .into_iter()
                            .find(|id| dom.get_node(*id).map(|n| n.is_element()).unwrap_or(false))
                    } else {
                        dom.query_selector(&selector).ok().flatten()
                    }
                })
            })
            .flatten()?;
        return Some(json!({
            "type": "object",
            "subtype": "node",
            "className": "HTMLElement",
            "description": selector,
            "objectId": format!("node-{}", node_id.index())
        }));
    }

    if expression_text.contains("ariaSnapshot") || expression_text.contains("generateAriaTree") {
        let snapshot = ctx
            .get_session_page(session_id)
            .and_then(|page| crate::domains::accessibility::aria_snapshot_for_page(page, ":root"))
            .unwrap_or_default();
        return Some(json!({
            "type": "string",
            "value": snapshot,
            "description": snapshot
        }));
    }

    let selector = arguments
        .get(6)
        .and_then(|arg| arg.get("value"))
        .and_then(extract_serialized_css_selector)?;

    let text = ctx
        .get_session_page(session_id)
        .and_then(|page| {
            page.with_dom(|dom| {
                dom.query_selector(&selector)
                    .ok()
                    .flatten()
                    .map(|node_id| dom.text_content(node_id))
            })
        })
        .flatten();

    if expression_text.contains("querySelectorAll") && !return_by_value {
        let log = text
            .as_ref()
            .map(|_| format!("  locator resolved to {}", selector))
            .unwrap_or_default();
        let serialized = json!({
            "o": [
                { "k": "log", "v": log },
                { "k": "success", "v": text.is_some() }
            ],
            "id": 1
        });
        let oid = format!(
            "{{\"injectedScriptId\":1,\"id\":\"obscura-selector-{}\"}}",
            selector
        );
        // The follow-up Playwright call reads only log/success from this handle.
        // Store the serialized value under the returned object id for that call.
        ctx.selector_results.insert(oid.clone(), serialized);
        return Some(json!({
            "type": "object",
            "className": "Object",
            "description": "Object",
            "objectId": oid
        }));
    }

    if !callback_text.contains("element.textContent") && !expression_text.contains("element.textContent") {
        return None;
    }

    let serialized = match text {
        Some(text) => json!({
            "o": [
                { "k": "log", "v": format!("  locator resolved to {}", selector) },
                { "k": "success", "v": true },
                { "k": "value", "v": text }
            ],
            "id": 1
        }),
        None => json!({
            "o": [
                { "k": "success", "v": false }
            ],
            "id": 1
        }),
    };

    Some(json!({
        "type": "object",
        "className": "Object",
        "description": "Object",
        "value": serialized
    }))
}

fn extract_serialized_css_selector(task: &Value) -> Option<String> {
    let info = serialized_prop(task, "info")?;
    let parsed = serialized_prop(info, "parsed")?;
    let source = serialized_prop(parsed, "source").and_then(|v| v.as_str());
    if let Some(source) = source {
        return Some(source.to_string());
    }

    find_serialized_prop(parsed, "css")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn selector_from_cached_oid(oid: &str) -> Option<String> {
    let value: Value = serde_json::from_str(oid).ok()?;
    value
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|id| id.strip_prefix("obscura-selector-"))
        .map(|s| s.to_string())
}

fn serialized_prop<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get("o").and_then(|v| v.as_array()).and_then(|props| {
        props.iter().find_map(|prop| {
            if prop.get("k").and_then(|v| v.as_str()) == Some(key) {
                prop.get("v")
            } else {
                None
            }
        })
    })
}

fn find_serialized_prop<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    if let Some(prop) = serialized_prop(value, key) {
        return Some(prop);
    }

    if let Some(props) = value.get("o").and_then(|v| v.as_array()) {
        for prop in props {
            if let Some(found) = prop.get("v").and_then(|v| find_serialized_prop(v, key)) {
                return Some(found);
            }
        }
    }

    if let Some(items) = value.get("a").and_then(|v| v.as_array()) {
        for item in items {
            if let Some(found) = find_serialized_prop(item, key) {
                return Some(found);
            }
        }
    }

    None
}
