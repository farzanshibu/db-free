// SOT: chat-mentions, at-mention-resolution, mention-context

use crate::guard::SessionCtx;
use crate::model::{ChatMention, DocumentKind, MentionKind};
use crate::services;
use crate::services::agent::tools;
use crate::state::AppState;
use serde_json::Value as Json;

// WHAT:  Turns the things a user pointed at with `@` into text the model reads.
// WHY:   "Write a query for @orders" should work without the model first having
//        to discover what `orders` is. Resolving on the server (by id, against
//        the live catalogue and the user's own library) means the chat cannot
//        smuggle in text that is not really a table or a saved query, and what
//        the model sees is always current.
// HOW:   A table becomes its full description (columns, keys, definition) — the
//        same text `describe_table` returns. A saved query becomes its SQL. A
//        dashboard, workflow or diagram becomes its id (so the model can update
//        it), its name and its body as JSON. Everything is capped: a mention
//        must never be the reason a request outgrows the context window. Rows
//        are never included, so this is safe for a schema-only chat.
// WHERE: src-tauri/src/services/agent/mod.rs (called once per user message)

/// Mentions honoured in one message; the rest are named but not expanded.
const MAX_MENTIONS: usize = 8;
/// Characters of one document's body shown to the model.
const MAX_BODY_CHARS: usize = 6_000;
/// Characters of all mentions together.
const MAX_TOTAL_CHARS: usize = 24_000;

pub async fn resolve(state: &AppState, ctx: &SessionCtx, mentions: &[ChatMention]) -> String {
    let mut out = String::new();
    for (index, mention) in mentions.iter().enumerate() {
        if index >= MAX_MENTIONS || out.chars().count() >= MAX_TOTAL_CHARS {
            out.push_str(&format!("- @{} (not expanded: too many references in one message)\n", mention.label));
            continue;
        }
        let block = match expand(state, ctx, mention).await {
            Ok(text) => text,
            Err(reason) => format!("(could not read @{}: {reason})", mention.label),
        };
        out.push_str(&format!("### @{}\n{}\n\n", mention.label, block.trim()));
    }
    out
}

async fn expand(state: &AppState, ctx: &SessionCtx, mention: &ChatMention) -> Result<String, String> {
    match mention.kind {
        MentionKind::Table => {
            let table = tools::table_ref(ctx, &mention.id, None).await.map_err(|e| e.message().to_string())?;
            let (text, _) = tools::describe_text(ctx, &table).await.map_err(|e| e.message().to_string())?;
            Ok(text)
        }
        MentionKind::SavedQuery => {
            let queries = services::saved_queries::list(state).map_err(|e| e.message().to_string())?;
            let query = queries.iter().find(|q| q.id == mention.id).ok_or("that saved query no longer exists")?;
            Ok(format!("Saved query \"{}\":\n```sql\n{}\n```", query.name, query.sql.trim()))
        }
        MentionKind::Dashboard => document_text(state, &mention.id, DocumentKind::Dashboard),
        MentionKind::Workflow => document_text(state, &mention.id, DocumentKind::Workflow),
        MentionKind::Diagram => document_text(state, &mention.id, DocumentKind::Diagram),
    }
}

// WHAT:  A document as text: id, kind, name and its body as JSON.
// WHY:   The JSON is the document model itself, so whatever the models gain
//        later reaches the assistant with no change here. It is trimmed of what
//        only costs room: empty fields, and where things sit on a canvas (an
//        edit keeps those on the server, so the model never needs to echo them).
fn document_text(state: &AppState, id: &str, kind: DocumentKind) -> Result<String, String> {
    let doc = services::documents::get(state, id).map_err(|e| e.message().to_string())?;
    if doc.kind != kind {
        return Err(format!("it is a {}, not a {}", doc.kind.as_str(), kind.as_str()));
    }
    let mut value = serde_json::to_value(&doc.body).map_err(|e| e.to_string())?;
    compact(&mut value, kind);
    let body = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    let (body, cut) = clip(&body, MAX_BODY_CHARS);
    Ok(format!(
        "{} \"{}\" — documentId: {}\n```json\n{body}\n```{}",
        kind.as_str(),
        doc.name,
        doc.id,
        if cut { "\n(body shortened)" } else { "" }
    ))
}

/// Keys of a workflow node or diagram table that only say where it sits.
const POSITION_KEYS: [&str; 4] = ["x", "y", "width", "height"];
/// Keys of a diagram relation that only say how its line is drawn.
const LINE_KEYS: [&str; 3] = ["waypoints", "fromSide", "toSide"];

// WHAT:  Drop nulls and empty lists everywhere, and canvas geometry from workflows
//        and diagrams (a dashboard's x / y / w / h are its layout, so they stay).
fn compact(value: &mut Json, kind: DocumentKind) {
    let strip = |items: Option<&mut Json>, keys: &[&str]| {
        if let Some(Json::Array(list)) = items {
            for item in list {
                if let Json::Object(map) = item {
                    for key in keys {
                        map.remove(*key);
                    }
                }
            }
        }
    };
    if let Some(data) = value.get_mut("data") {
        match kind {
            DocumentKind::Workflow => strip(data.get_mut("nodes"), &POSITION_KEYS),
            DocumentKind::Diagram => {
                strip(data.get_mut("tables"), &POSITION_KEYS);
                strip(data.get_mut("notes"), &POSITION_KEYS);
                strip(data.get_mut("relations"), &LINE_KEYS);
            }
            DocumentKind::Dashboard => {}
        }
    }
    drop_empty(value);
}

fn drop_empty(value: &mut Json) {
    match value {
        Json::Object(map) => {
            for child in map.values_mut() {
                drop_empty(child);
            }
            map.retain(|_, child| !matches!(child, Json::Null) && !matches!(child, Json::Array(items) if items.is_empty()));
        }
        Json::Array(items) => items.iter_mut().for_each(drop_empty),
        _ => {}
    }
}

/// The first `limit` characters, and whether anything was cut.
fn clip(text: &str, limit: usize) -> (String, bool) {
    if text.chars().count() <= limit {
        return (text.to_string(), false);
    }
    (text.chars().take(limit).collect(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workflow_is_shown_without_canvas_positions_or_empty_fields() {
        let mut body = serde_json::json!({ "kind": "workflow", "data": {
            "nodes": [{ "id": "q", "x": 80.0, "y": 80.0, "label": "", "continueOnError": false,
                "config": { "kind": "query", "sql": "SELECT 1", "connectionId": null } }],
            "edges": []
        }});
        compact(&mut body, DocumentKind::Workflow);
        let node = &body["data"]["nodes"][0];
        assert!(node.get("x").is_none() && node.get("y").is_none());
        assert!(node["config"].get("connectionId").is_none(), "nulls go");
        assert!(body["data"].get("edges").is_none(), "empty lists go");
        assert_eq!(node["config"]["sql"], "SELECT 1");
        assert_eq!(node["continueOnError"], false, "a switch that is off still says so");
    }

    #[test]
    fn a_diagram_loses_geometry_but_a_dashboard_keeps_its_layout() {
        let mut diagram = serde_json::json!({ "kind": "diagram", "data": {
            "tables": [{ "id": "t", "name": "T", "x": 1.0, "y": 2.0, "color": null, "columns": [{ "name": "id", "dataType": "int" }] }],
            "relations": [{ "id": "r", "fromTable": "a", "waypoints": [{ "x": 1.0, "y": 1.0 }], "fromSide": "left" }],
            "notes": [{ "id": "n", "x": 5.0, "y": 5.0, "width": 220.0, "height": 120.0, "text": "why" }]
        }});
        compact(&mut diagram, DocumentKind::Diagram);
        assert!(diagram["data"]["tables"][0].get("x").is_none());
        assert!(diagram["data"]["relations"][0].get("waypoints").is_none() && diagram["data"]["relations"][0].get("fromSide").is_none());
        assert_eq!(diagram["data"]["notes"][0]["text"], "why");
        assert!(diagram["data"]["notes"][0].get("width").is_none());
        assert_eq!(diagram["data"]["tables"][0]["columns"][0]["dataType"], "int");

        let mut dashboard = serde_json::json!({ "kind": "dashboard", "data": { "widgets": [{ "id": "w", "x": 3, "y": 0, "w": 4, "h": 5, "tint": null }] }});
        compact(&mut dashboard, DocumentKind::Dashboard);
        let widget = &dashboard["data"]["widgets"][0];
        assert_eq!((widget["x"].as_u64(), widget["w"].as_u64()), (Some(3), Some(4)));
        assert!(widget.get("tint").is_none());
    }

    #[test]
    fn clip_cuts_on_a_character_boundary() {
        assert_eq!(clip("abc", 5), ("abc".to_string(), false));
        assert_eq!(clip("ééééé", 3), ("ééé".to_string(), true));
    }
}
