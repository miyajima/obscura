use serde_json::{json, Value};

use crate::dispatch::CdpContext;
use crate::types::CdpEvent;

pub async fn handle(method: &str, params: &Value, ctx: &mut CdpContext) -> Result<Value, String> {
    match method {
        "setDiscoverTargets" => {
            ctx.pending_events.push(CdpEvent::new(
                "Target.targetCreated",
                json!({
                    "targetInfo": {
                        "targetId": "browser",
                        "type": "browser",
                        "title": "",
                        "url": "",
                        "attached": true,
                        "browserContextId": "",
                    }
                }),
            ));
            for page in &ctx.pages {
                ctx.pending_events.push(CdpEvent::new(
                    "Target.targetCreated",
                    json!({
                        "targetInfo": {
                            "targetId": page.id,
                            "type": "page",
                            "title": page.title,
                            "url": page.url_string(),
                            "attached": false,
                            "browserContextId": page.context.id,
                        }
                    }),
                ));
            }
            Ok(json!({}))
        }
        "getTargets" => {
            let targets: Vec<Value> = ctx
                .pages
                .iter()
                .map(|page| {
                    json!({
                        "targetId": page.id,
                        "type": "page",
                        "title": page.title,
                        "url": page.url_string(),
                        "attached": true,
                        "browserContextId": page.context.id,
                    })
                })
                .collect();
            Ok(json!({ "targetInfos": targets }))
        }
        "createTarget" => {
            let url = params
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("about:blank");
            let page_id = ctx.create_page();
            let session_id = format!("{}-session", page_id);

            if let Some(page) = ctx.get_page_mut(&page_id) {
                if url == "about:blank" || url.is_empty() {
                    page.navigate_blank();
                } else {
                    let _ = page.navigate(url).await;
                }
            }

            if let Some(page) = ctx.get_page(&page_id) {
                ctx.pending_events.push(CdpEvent::new(
                    "Target.targetCreated",
                    json!({
                        "targetInfo": {
                            "targetId": page_id,
                            "type": "page",
                            "title": page.title,
                            "url": page.url_string(),
                            "attached": false,
                            "browserContextId": page.context.id,
                        }
                    }),
                ));
            }

            ctx.sessions.insert(session_id.clone(), page_id.clone());

            if let Some(page) = ctx.get_page(&page_id) {
                ctx.pending_events.push(CdpEvent::new(
                    "Target.attachedToTarget",
                    json!({
                        "sessionId": session_id,
                        "targetInfo": {
                            "targetId": page_id,
                            "type": "page",
                            "title": page.title,
                            "url": page.url_string(),
                            "attached": true,
                            "browserContextId": page.context.id,
                        },
                        "waitingForDebugger": false,
                    }),
                ));
            }

            Ok(json!({ "targetId": page_id }))
        }
        "attachToTarget" => {
            let target_id = params
                .get("targetId")
                .and_then(|v| v.as_str())
                .ok_or("targetId required")?;
            let existing_session_id = ctx.sessions.iter().find_map(|(session_id, page_id)| {
                if page_id == target_id {
                    Some(session_id.clone())
                } else {
                    None
                }
            });
            let session_id =
                existing_session_id.unwrap_or_else(|| format!("{}-session", target_id));
            let already_attached = ctx.sessions.contains_key(&session_id);
            ctx.sessions
                .insert(session_id.clone(), target_id.to_string());

            if !already_attached {
                if let Some(page) = ctx.get_page(target_id) {
                    ctx.pending_events.push(CdpEvent::new(
                        "Target.attachedToTarget",
                        json!({
                            "sessionId": session_id,
                            "targetInfo": {
                                "targetId": target_id,
                                "type": "page",
                                "title": page.title,
                                "url": page.url_string(),
                                "attached": true,
                                "browserContextId": page.context.id,
                            },
                            "waitingForDebugger": false,
                        }),
                    ));
                }
            }

            Ok(json!({ "sessionId": session_id }))
        }
        "attachToBrowserTarget" => {
            let session_id = "browser-session".to_string();
            let already_attached = ctx.sessions.contains_key(&session_id);
            ctx.sessions
                .insert(session_id.clone(), "browser".to_string());
            if !already_attached {
                ctx.pending_events.push(CdpEvent::new(
                    "Target.attachedToTarget",
                    json!({
                        "sessionId": session_id,
                        "targetInfo": {
                            "targetId": "browser",
                            "type": "browser",
                            "title": "",
                            "url": "",
                            "attached": true,
                            "browserContextId": "",
                        },
                        "waitingForDebugger": false,
                    }),
                ));
            }

            Ok(json!({ "sessionId": session_id }))
        }
        "closeTarget" => {
            let target_id = params
                .get("targetId")
                .and_then(|v| v.as_str())
                .ok_or("targetId required")?;
            let session_id = format!("{}-session", target_id);

            ctx.pending_events.push(CdpEvent::new(
                "Target.detachedFromTarget",
                json!({
                    "sessionId": session_id,
                    "targetId": target_id,
                }),
            ));
            ctx.pending_events.push(CdpEvent::new(
                "Target.targetDestroyed",
                json!({ "targetId": target_id }),
            ));

            ctx.remove_page(target_id);
            Ok(json!({ "success": true }))
        }
        "setAutoAttach" => {
            if ctx.pages.is_empty() {
                ctx.create_page();
            }

            let unattached_pages: Vec<_> = ctx
                .pages
                .iter()
                .filter(|page| !ctx.sessions.values().any(|page_id| page_id == &page.id))
                .map(|page| {
                    (
                        page.id.clone(),
                        page.title.clone(),
                        page.url_string(),
                        page.context.id.clone(),
                    )
                })
                .collect();

            for (page_id, title, url, context_id) in unattached_pages {
                let session_id = format!("{}-session", page_id);
                ctx.sessions.insert(session_id.clone(), page_id.clone());
                ctx.pending_events.push(CdpEvent::new(
                    "Target.targetCreated",
                    json!({
                        "targetInfo": {
                            "targetId": page_id,
                            "type": "page",
                            "title": title,
                            "url": url,
                            "attached": false,
                            "browserContextId": context_id,
                        }
                    }),
                ));
                ctx.pending_events.push(CdpEvent::new(
                    "Target.attachedToTarget",
                    json!({
                        "sessionId": session_id,
                        "targetInfo": {
                            "targetId": page_id,
                            "type": "page",
                            "title": title,
                            "url": url,
                            "attached": true,
                            "browserContextId": context_id,
                        },
                        "waitingForDebugger": false,
                    }),
                ));
            }

            Ok(json!({}))
        }
        "getBrowserContexts" => Ok(json!({ "browserContextIds": [ctx.default_context.id] })),
        "createBrowserContext" => {
            ctx.default_context.cookie_jar.clear();
            Ok(json!({ "browserContextId": ctx.default_context.id }))
        }
        "disposeBrowserContext" => {
            ctx.default_context.cookie_jar.clear();
            Ok(json!({}))
        }
        "getTargetInfo" => {
            let target_id = params.get("targetId").and_then(|v| v.as_str());
            match target_id {
                Some(id) => {
                    let page = ctx.get_page(id).ok_or("Target not found")?;
                    Ok(json!({
                        "targetInfo": {
                            "targetId": id,
                            "type": "page",
                            "title": page.title,
                            "url": page.url_string(),
                            "attached": true,
                            "browserContextId": page.context.id,
                        }
                    }))
                }
                None => Ok(json!({
                    "targetInfo": {
                        "targetId": "browser",
                        "type": "browser",
                        "title": "",
                        "url": "",
                        "attached": true,
                    }
                })),
            }
        }
        _ => Err(format!("Unknown Target method: {}", method)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn attach_to_browser_target_returns_session_id_and_event() {
        let mut ctx = CdpContext::new();
        let result = handle("attachToBrowserTarget", &json!({}), &mut ctx)
            .await
            .expect("attachToBrowserTarget should succeed");

        assert_eq!(result["sessionId"], "browser-session");
        assert_eq!(
            ctx.sessions.get("browser-session").map(String::as_str),
            Some("browser")
        );
        let event = ctx
            .pending_events
            .iter()
            .find(|event| event.method == "Target.attachedToTarget")
            .expect("attachedToTarget event must be emitted");
        assert_eq!(event.params["targetInfo"]["type"], "browser");
    }

    #[tokio::test]
    async fn set_auto_attach_creates_and_attaches_page() {
        let mut ctx = CdpContext::new();
        let result = handle("setAutoAttach", &json!({}), &mut ctx)
            .await
            .expect("setAutoAttach should succeed");
        assert_eq!(result, json!({}));
        assert!(!ctx.pages.is_empty());
        assert!(ctx
            .pending_events
            .iter()
            .any(|event| event.method == "Target.attachedToTarget"));
    }

    #[tokio::test]
    async fn unknown_target_method_still_errors() {
        let mut ctx = CdpContext::new();
        let err = handle("notARealMethod", &json!({}), &mut ctx)
            .await
            .expect_err("unknown methods must surface as errors");
        assert!(err.contains("Unknown Target method"));
    }
}
