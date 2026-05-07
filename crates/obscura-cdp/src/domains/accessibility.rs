use obscura_browser::Page;
use obscura_dom::{DomTree, Node, NodeData, NodeId};
use serde_json::{json, Value};

use crate::dispatch::CdpContext;

pub async fn handle(
    method: &str,
    params: &Value,
    ctx: &mut CdpContext,
    session_id: &Option<String>,
) -> Result<Value, String> {
    match method {
        "enable" | "disable" => Ok(json!({})),
        "getFullAXTree" => {
            let page = ctx
                .get_session_page(session_id)
                .ok_or("No page for session")?;
            let depth = params
                .get("depth")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let nodes = page
                .with_dom(|dom| {
                    let root = document_element(dom).unwrap_or_else(|| dom.document());
                    let mut nodes = Vec::new();
                    build_ax_tree(dom, root, depth, 0, &mut nodes);
                    nodes
                })
                .unwrap_or_default();
            Ok(json!({ "nodes": nodes }))
        }
        _ => Err(format!("Unknown Accessibility method: {}", method)),
    }
}

pub fn aria_snapshot_for_page(page: &Page, selector: &str) -> Option<String> {
    page.with_dom(|dom| {
        let root = if selector == ":root" {
            document_element(dom).unwrap_or_else(|| dom.document())
        } else {
            dom.query_selector(selector)
                .ok()
                .flatten()
                .unwrap_or_else(|| document_element(dom).unwrap_or_else(|| dom.document()))
        };

        let mut lines = Vec::new();
        build_aria_lines(dom, root, 0, &mut lines);
        lines.join("\n")
    })
}

fn build_ax_tree(
    dom: &DomTree,
    node_id: NodeId,
    max_depth: Option<usize>,
    depth: usize,
    out: &mut Vec<Value>,
) -> Option<String> {
    let node = dom.get_node(node_id)?;
    if should_skip_subtree(&node) {
        return None;
    }
    if is_hidden_or_ignored(&node) {
        return None;
    }

    let role = role_for_node(&node)?;
    let name = accessible_name(dom, node_id, &node, &role);
    let child_ids: Vec<String> = if max_depth.map(|max| depth < max).unwrap_or(true) {
        dom.children(node_id)
            .into_iter()
            .filter_map(|child| build_ax_tree(dom, child, max_depth, depth + 1, out))
            .collect()
    } else {
        Vec::new()
    };

    let ax_id = ax_node_id(node_id);
    let mut ax_node = json!({
        "nodeId": ax_id,
        "ignored": false,
        "role": { "type": "role", "value": role },
        "name": { "type": "computedString", "value": name },
        "backendDOMNodeId": node_id.index(),
        "childIds": child_ids,
    });

    let properties = ax_properties(&node, &role);
    if !properties.is_empty() {
        ax_node["properties"] = json!(properties);
    }
    out.push(ax_node);
    Some(ax_id)
}

fn build_aria_lines(dom: &DomTree, node_id: NodeId, depth: usize, out: &mut Vec<String>) {
    let Some(node) = dom.get_node(node_id) else {
        return;
    };
    if should_skip_subtree(&node) {
        return;
    }
    if is_hidden_or_ignored(&node) {
        return;
    }

    let Some(role) = role_for_node(&node) else {
        for child in dom.children(node_id) {
            build_aria_lines(dom, child, depth, out);
        }
        return;
    };

    let name = accessible_name(dom, node_id, &node, &role);
    let mut line = format!("{}- {}", "  ".repeat(depth), role);
    if !name.is_empty() {
        line.push(' ');
        line.push('"');
        line.push_str(&escape_aria_string(&name));
        line.push('"');
    }
    if let Some(level) = heading_level(&node) {
        line.push_str(&format!(" [level={}]", level));
    }
    out.push(line);

    for child in dom.children(node_id) {
        build_aria_lines(dom, child, depth + 1, out);
    }
}

fn role_for_node(node: &Node) -> Option<String> {
    match &node.data {
        NodeData::Document => Some("document".to_string()),
        NodeData::Text { contents } => {
            if normalize_text(contents).is_empty() {
                None
            } else {
                Some("text".to_string())
            }
        }
        NodeData::Element { name, .. } => {
            if let Some(role) = node.get_attribute("role").filter(|r| !r.trim().is_empty()) {
                return Some(role.trim().to_string());
            }
            let tag = name.local.as_ref();
            let role = match tag {
                "html" | "body" | "div" | "span" | "section" | "header" | "footer" => "generic",
                "main" => "main",
                "nav" => "navigation",
                "article" => "article",
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
                "a" | "area" if node.get_attribute("href").is_some() => "link",
                "button" => "button",
                "textarea" => "textbox",
                "select" => "combobox",
                "ul" | "ol" => "list",
                "li" => "listitem",
                "img" => "img",
                "table" => "table",
                "tr" => "row",
                "th" => "columnheader",
                "td" => "cell",
                "input" => input_role(node),
                "script" | "style" | "meta" | "link" | "title" | "head" => return None,
                _ => "generic",
            };
            Some(role.to_string())
        }
        _ => None,
    }
}

fn input_role(node: &Node) -> &'static str {
    match node.get_attribute("type").unwrap_or("text") {
        "button" | "submit" | "reset" => "button",
        "checkbox" => "checkbox",
        "radio" => "radio",
        "range" => "slider",
        "number" => "spinbutton",
        "search" => "searchbox",
        "hidden" => "none",
        _ => "textbox",
    }
}

fn accessible_name(dom: &DomTree, node_id: NodeId, node: &Node, role: &str) -> String {
    if let Some(label) = node.get_attribute("aria-label") {
        return normalize_text(label);
    }
    if let Some(labelled_by) = node.get_attribute("aria-labelledby") {
        let mut parts = Vec::new();
        for id in labelled_by.split_whitespace() {
            if let Some(label_node) = dom.get_element_by_id(id) {
                let text = normalize_text(&dom.text_content(label_node));
                if !text.is_empty() {
                    parts.push(text);
                }
            }
        }
        if !parts.is_empty() {
            return parts.join(" ");
        }
    }

    for attr in ["alt", "title", "placeholder", "value"] {
        if let Some(value) = node.get_attribute(attr) {
            let value = normalize_text(value);
            if !value.is_empty() {
                return value;
            }
        }
    }

    match role {
        "text" => node
            .text_content_of_text_node()
            .map(normalize_text)
            .unwrap_or_default(),
        "generic" | "document" | "main" | "navigation" | "list" | "table" | "row" => String::new(),
        _ => normalize_text(&dom.text_content(node_id)),
    }
}

fn ax_properties(node: &Node, role: &str) -> Vec<Value> {
    let mut properties = Vec::new();
    if let Some(level) = heading_level(node) {
        properties.push(json!({
            "name": "level",
            "value": { "type": "integer", "value": level }
        }));
    }
    if node.get_attribute("disabled").is_some()
        || node.get_attribute("aria-disabled") == Some("true")
    {
        properties.push(json!({
            "name": "disabled",
            "value": { "type": "boolean", "value": true }
        }));
    }
    if matches!(role, "checkbox" | "radio") {
        let checked = node.get_attribute("checked").is_some()
            || node.get_attribute("aria-checked") == Some("true");
        properties.push(json!({
            "name": "checked",
            "value": { "type": "tristate", "value": if checked { "true" } else { "false" } }
        }));
    }
    properties
}

fn heading_level(node: &Node) -> Option<u8> {
    let NodeData::Element { name, .. } = &node.data else {
        return None;
    };
    match name.local.as_ref() {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => node
            .get_attribute("aria-level")
            .and_then(|v| v.parse::<u8>().ok()),
    }
}

fn document_element(dom: &DomTree) -> Option<NodeId> {
    dom.children(dom.document())
        .into_iter()
        .find(|id| dom.get_node(*id).map(|n| n.is_element()).unwrap_or(false))
}

fn is_hidden_or_ignored(node: &Node) -> bool {
    if let Some(hidden) = node.get_attribute("hidden") {
        if hidden.is_empty() || hidden.eq_ignore_ascii_case("true") {
            return true;
        }
    }
    node.get_attribute("aria-hidden") == Some("true")
        || matches!(
            role_for_hidden_attr(node).as_deref(),
            Some("presentation" | "none")
        )
}

fn should_skip_subtree(node: &Node) -> bool {
    match &node.data {
        NodeData::Element { name, .. } => {
            matches!(
                name.local.as_ref(),
                "script" | "style" | "meta" | "link" | "title" | "head"
            )
        }
        _ => false,
    }
}

fn role_for_hidden_attr(node: &Node) -> Option<String> {
    node.get_attribute("role").map(|r| r.trim().to_string())
}

fn ax_node_id(node_id: NodeId) -> String {
    format!("ax-{}", node_id.index())
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escape_aria_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
