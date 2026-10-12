// SOT: workflow-http-node, workflow-read-file-node, workflow-save-file-node, json-path-rows

use super::{or_stopped, NodeCtx, NodeFailure, NodeOutput, NodeResult};
use crate::integrations::http::{self, OutboundRequest};
use crate::model::workflow::{HttpConfig, ReadFileConfig, SaveFileConfig};
use crate::model::Value;
use crate::services::workflows::data::Dataset;
use crate::services::workflows::{files, template};

// WHAT:  The HTTP node: calls a URL and turns the JSON answer into rows.
// HOW:   The URL, the header values and the body are filled from the first input
//        row. `response_path` ("data.results") picks where the rows are; an array
//        becomes one row per element and an object becomes one row. A non-2xx
//        status is an error carrying the status line.
pub async fn http(ctx: &NodeCtx<'_>, config: &HttpConfig) -> NodeResult {
    let url = template::render_text(config.url.trim(), ctx.input);
    if url.trim().is_empty() {
        return Err(NodeFailure::new("URL is empty."));
    }
    let headers = parse_headers(&config.headers, ctx)?;
    let body = (config.method.has_body() && !config.body.trim().is_empty()).then(|| template::render_text(config.body.trim(), ctx.input));
    let request = OutboundRequest { method: config.method.as_str().to_string(), url: url.trim().to_string(), headers, body };
    let reply = or_stopped(ctx.cancel, http::send_outbound(&request)).await??;
    if !(200..300).contains(&reply.status) {
        return Err(NodeFailure::new(format!("HTTP {} {}", reply.status, reply.status_text).trim().to_string()));
    }
    if config.method.as_str() == "HEAD" {
        return Ok(NodeOutput::data(Dataset::single_row(vec![("status", Value::Int(i64::from(reply.status)))])));
    }
    let parsed: serde_json::Value = serde_json::from_str(&reply.body).map_err(|_| NodeFailure::new("Response is not valid JSON."))?;
    Ok(NodeOutput::data(rows_from_response(&parsed, config.response_path.trim())?))
}

// WHAT:  Reads the header template: a JSON object whose values may hold placeholders.
// HOW:   Parsed first and filled value by value, so a data value can never add or
//        break a header. A template that is not JSON until it is filled (a bare
//        `{{n}}` as a value) is filled as text and parsed after.
fn parse_headers(text: &str, ctx: &NodeCtx<'_>) -> Result<Vec<(String, String)>, NodeFailure> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let (parsed, filled_already) = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => (value, false),
        Err(_) => {
            let filled = template::render_text(text, ctx.input);
            let value = serde_json::from_str::<serde_json::Value>(&filled).map_err(|e| NodeFailure::new(format!("Headers JSON is invalid: {e}")))?;
            (value, true)
        }
    };
    let serde_json::Value::Object(map) = parsed else {
        return Err(NodeFailure::new("Headers must be a JSON object."));
    };
    let fill = |s: &str| if filled_already { s.to_string() } else { template::render_text(s, ctx.input) };
    let mut headers = Vec::with_capacity(map.len());
    for (name, value) in map {
        let value = match value {
            serde_json::Value::String(s) => fill(&s),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => continue,
            _ => return Err(NodeFailure::new(format!("Header \"{name}\" must be text."))),
        };
        headers.push((fill(&name), value));
    }
    Ok(headers)
}

fn pluck<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.').map(str::trim).filter(|s| !s.is_empty()) {
        current = match current {
            serde_json::Value::Object(map) => map.get(segment)?,
            serde_json::Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn rows_from_response(body: &serde_json::Value, path: &str) -> Result<Dataset, NodeFailure> {
    let picked = pluck(body, path).ok_or_else(|| NodeFailure::new(format!("Nothing at \"{path}\" in the response.")))?;
    match picked {
        serde_json::Value::Array(items) => Ok(Dataset::from_json_rows(items)),
        serde_json::Value::Object(_) => Ok(Dataset::from_json_rows(std::slice::from_ref(picked))),
        _ if path.is_empty() => Err(NodeFailure::new("The response is neither an array nor an object: nothing to turn into rows.")),
        _ => Err(NodeFailure::new(format!("The value at \"{path}\" is neither an array nor an object."))),
    }
}

pub async fn read_file(config: &ReadFileConfig) -> NodeResult {
    let path = config.path.trim();
    if path.is_empty() {
        return Err(NodeFailure::new("No file chosen."));
    }
    let bytes = tokio::fs::read(path).await.map_err(|e| NodeFailure::new(format!("Could not read {path}: {e}")))?;
    let text = String::from_utf8(bytes).map_err(|_| NodeFailure::new("The file is not valid UTF-8 text."))?;
    Ok(NodeOutput::data(files::parse(&text, config.format)?))
}

// WHAT:  The Save to file node: writes the merged input rows to disk.
// WHY:   It overwrites, so a Repeat that feeds it leaves the last pass's rows.
pub async fn save_file(ctx: &NodeCtx<'_>, config: &SaveFileConfig) -> NodeResult {
    if ctx.input_count == 0 {
        return Err(NodeFailure::new("No input: connect an upstream node first."));
    }
    let path = config.path.trim();
    if path.is_empty() {
        return Err(NodeFailure::new("No file path set. Choose where to save it in the node's settings."));
    }
    let table = if config.table_name.trim().is_empty() { files::table_from_path(path) } else { config.table_name.trim().to_string() };
    let content = files::render(ctx.input, config.format, &table)?;
    tokio::fs::write(path, content).await.map_err(|e| NodeFailure::new(format!("Could not write {path}: {e}")))?;
    Ok(NodeOutput { rows: Some(ctx.input.row_count()), file_path: Some(path.to_string()), ..NodeOutput::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn response_paths_walk_objects_and_arrays() {
        let body = json!({"data": {"results": [{"id": 1}, {"id": 2}], "meta": {"page": 3}}});
        assert_eq!(rows_from_response(&body, "data.results").unwrap().rows.len(), 2);
        assert_eq!(rows_from_response(&body, "data.meta").unwrap().rows.len(), 1);
        assert_eq!(rows_from_response(&body, "data.results.1").unwrap().rows[0][0], Value::Int(2));
        assert!(rows_from_response(&body, "data.nope").is_err());
        assert!(rows_from_response(&json!({"n": 1}), "n").is_err());
        assert!(rows_from_response(&json!("text"), "").is_err());
        assert_eq!(rows_from_response(&json!([{"a": 1}]), "").unwrap().rows.len(), 1);
    }

    #[test]
    fn pluck_ignores_stray_dots() {
        let body = json!({"a": {"b": [10, 20]}});
        assert_eq!(pluck(&body, " a . b ").map(|v| v.is_array()), Some(true));
        assert_eq!(pluck(&body, ".a.b.0"), Some(&json!(10)));
        assert_eq!(pluck(&body, "a.b.9"), None);
        assert_eq!(pluck(&body, ""), Some(&body));
    }
}
