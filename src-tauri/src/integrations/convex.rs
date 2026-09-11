// SOT: convex-integration, convex-functions-api, convex-function-console, convex-deploy-key-auth

use crate::error::{AppError, AppResult};
use crate::integrations::http::{json_result, Auth, HttpClient};
use crate::integrations::kafka::split_statements;
use crate::integrations::{Capabilities, Integration};
use crate::model::{
    ColumnInfo, Engine, FilterRule, ObjectKind, ObjectSummary, PageQuery, ResolvedConnection,
    ResultSet, SchemaCatalog, StatementResult, TableRef,
};
use async_trait::async_trait;
use serde_json::{json, Map, Value as Json};
use std::sync::Arc;

// ============================================================================
// CONVEX ADAPTER (functions HTTP API)
//
// WHAT:  Runs Convex backend functions over the deployment's HTTP API:
//        POST /api/query, /api/mutation, /api/action with
//        {"path": "module:function", "args": {…}, "format": "json"}.
// WHY:   Convex exposes no table or function listing over HTTP (tables are
//        visible only in the dashboard / CLI admin API), so the adapter is an
//        honest function console: the query tab runs functions and renders
//        their return values, and the catalog stays empty. Nothing is
//        invented: only what /api/query|mutation|action can answer.
// HOW:   `host` = deployment URL (https://<name>.convex.cloud, kept whole);
//        the secret is the dashboard deploy key, sent as
//        `Authorization: Convex <key>`. Without a key, public functions still
//        run; mutations and actions are refused on read-only connections.
// WHERE: https://docs.convex.dev/http-api/,
//        src-tauri/src/integrations/http.rs (client)
// ============================================================================

const NO_TABLES: &str = "Convex exposes no table listing over HTTP; run a query function such as {\"function\": \"messages:list\"}.";

pub struct ConvexIntegration {
    engine: Engine,
    http: HttpClient,
    label: String,
    read_only: bool,
}

fn json_string(value: &Json) -> String {
    match value {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn deployment_label(conn: &ResolvedConnection) -> String {
    let host = conn
        .summary
        .host
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .trim_end_matches('/');
    if host.is_empty() {
        return "convex".to_string();
    }
    host.rsplit("://")
        .next()
        .unwrap_or(host)
        .split('.')
        .next()
        .unwrap_or(host)
        .to_string()
}

pub async fn connect(conn: &ResolvedConnection) -> AppResult<Arc<dyn Integration>> {
    let base = crate::integrations::http::base_url(conn, None, true);
    let insecure = conn.summary.ssl_mode == crate::model::SslMode::Require;
    let auth = match conn.secret.as_deref().filter(|s| !s.is_empty()) {
        Some(key) => Auth::Header {
            name: "Authorization".to_string(),
            value: format!("Convex {key}"),
        },
        None => Auth::None,
    };
    let http = HttpClient::new(base, auth, insecure)?;
    let label = deployment_label(conn);
    let integration = ConvexIntegration {
        engine: conn.summary.engine,
        http,
        label,
        read_only: conn.summary.read_only,
    };
    integration.ping().await?;
    Ok(Arc::new(integration))
}

// ---------------------------------------------------------------------------
// Console commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    Query,
    Mutation,
    Action,
}

impl FunctionKind {
    fn endpoint(self) -> &'static str {
        match self {
            FunctionKind::Query => "api/query",
            FunctionKind::Mutation => "api/mutation",
            FunctionKind::Action => "api/action",
        }
    }

    fn label(self) -> &'static str {
        match self {
            FunctionKind::Query => "query",
            FunctionKind::Mutation => "mutation",
            FunctionKind::Action => "action",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub kind: FunctionKind,
    pub path: String,
    pub args: Map<String, Json>,
}

// WHAT:  JSON body or shorthand → Call. Reads default to queries; anything
//        that writes must say MUTATE / mutation explicitly.
pub fn parse_command(text: &str) -> AppResult<Call> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::invalid_input("Nothing to run."));
    }
    if text.starts_with('{') {
        let json: Json = serde_json::from_str(text)
            .map_err(|e| AppError::invalid_input(format!("Command is not valid JSON: {e}")))?;
        let obj = json
            .as_object()
            .ok_or_else(|| AppError::invalid_input("Command must be a JSON object."))?;
        let path = obj
            .get("function")
            .or_else(|| obj.get("path"))
            .map(json_string)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| {
                AppError::invalid_input(
                    "JSON body needs a \"function\" path like \"messages:list\".",
                )
            })?;
        let kind = match obj
            .get("kind")
            .map(json_string)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "mutation" | "mutate" => FunctionKind::Mutation,
            "action" => FunctionKind::Action,
            _ => FunctionKind::Query,
        };
        let args = obj
            .get("args")
            .and_then(Json::as_object)
            .cloned()
            .unwrap_or_default();
        return Ok(Call { kind, path, args });
    }
    let mut words = text.split_whitespace();
    let verb = words.next().unwrap_or_default().to_ascii_lowercase();
    let (kind, path) = match verb.as_str() {
        "query" | "q" => (FunctionKind::Query, words.next()),
        "mutate" | "mutation" | "m" => (FunctionKind::Mutation, words.next()),
        "action" | "a" => (FunctionKind::Action, words.next()),
        // Bare `module:function […]` reads as a query.
        _ => (FunctionKind::Query, Some(verb.as_str())),
    };
    let path = path
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::invalid_input("Usage: QUERY <module:function> [{\"arg\": …}]"))?
        .to_string();
    let rest: String = words.collect::<Vec<_>>().join(" ");
    let args = if rest.trim().is_empty() {
        Map::new()
    } else {
        serde_json::from_str::<Json>(&rest)
            .map_err(|_| {
                AppError::invalid_input("Arguments after the path must be one JSON object.")
            })?
            .as_object()
            .cloned()
            .ok_or_else(|| {
                AppError::invalid_input("Arguments after the path must be one JSON object.")
            })?
    };
    Ok(Call { kind, path, args })
}

// WHAT:  Convex function response → grid. Success renders the return value;
//        a function error becomes an input error carrying the backend message.
fn response_to_result(value: &Json) -> AppResult<ResultSet> {
    match value.get("status").and_then(Json::as_str) {
        Some("success") => Ok(json_result(
            value.get("value").cloned().unwrap_or(Json::Null),
        )),
        _ => {
            let message = value
                .get("errorMessage")
                .map(json_string)
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| value.to_string());
            Err(AppError::driver(format!(
                "Convex function failed: {message}"
            )))
        }
    }
}

impl ConvexIntegration {
    async fn call(&self, call: &Call) -> AppResult<ResultSet> {
        if self.read_only && !matches!(call.kind, FunctionKind::Query) {
            return Err(AppError::read_only(format!(
                "This connection is read-only; {}s are blocked.",
                call.kind.label()
            )));
        }
        let body = json!({ "path": call.path, "args": call.args, "format": "json" });
        let value: Json = self.http.post_json(call.kind.endpoint(), &body).await?;
        response_to_result(&value)
    }
}

// WHAT:  What this family offers the object explorer and the tool tabs: the
//        functions API answers calls only, so there is nothing to list.
// WHERE: src-tauri/src/integrations/mod.rs (FamilyProfile), src/lib/objects.ts
pub fn profile() -> crate::integrations::FamilyProfile {
    crate::integrations::FamilyProfile {
        capabilities: Capabilities {
            describes_fields: false,
            sql: false,
            namespaces: false,
            fixed_columns: false,
            paging: false,
            row_estimate: false,
            views: false,
            transactions: false,
            exact_estimate: false,
        },
        object_kinds: vec![],
        tools: vec![],
    }
}

#[async_trait]
impl Integration for ConvexIntegration {
    fn engine(&self) -> Engine {
        self.engine
    }

    fn capabilities(&self) -> Capabilities {
        profile().capabilities
    }

    async fn ping(&self) -> AppResult<()> {
        let body = json!({ "path": "__db_free_ping__", "args": {} });
        match self.http.post_json::<Json>("api/query", &body).await {
            Ok(_) => Ok(()),
            // Any shaped Convex answer — including "no such function" — proves
            // the deployment is reachable; only auth and transport failures
            // propagate.
            Err(AppError::NotFound { .. }) => Ok(()),
            Err(other) => Err(other),
        }
    }

    async fn server_version(&self) -> AppResult<Option<String>> {
        // The functions API reports no server version.
        Ok(None)
    }

    fn current_database(&self) -> Option<String> {
        Some(self.label.clone())
    }

    async fn databases(&self) -> AppResult<Vec<String>> {
        Ok(vec![self.label.clone()])
    }

    async fn catalog(&self) -> AppResult<SchemaCatalog> {
        Ok(SchemaCatalog { schemas: vec![] })
    }

    async fn columns(&self, _table: &TableRef) -> AppResult<Vec<ColumnInfo>> {
        Err(AppError::not_found(NO_TABLES))
    }

    async fn row_estimate(&self, _table: &TableRef) -> AppResult<Option<i64>> {
        Ok(None)
    }

    async fn count(&self, _table: &TableRef, _filters: &[FilterRule]) -> AppResult<i64> {
        Err(AppError::not_found(NO_TABLES))
    }

    async fn fetch_page(&self, _table: &TableRef, _query: &PageQuery) -> AppResult<ResultSet> {
        Err(AppError::not_found(NO_TABLES))
    }

    async fn execute(&self, script: &str, _max_rows: usize) -> AppResult<Vec<StatementResult>> {
        let statements = split_statements(script);
        if statements.is_empty() {
            return Err(AppError::invalid_input("Nothing to run."));
        }
        let mut out = Vec::with_capacity(statements.len());
        for statement in statements {
            let call = parse_command(statement)?;
            out.push(StatementResult::Rows {
                result: self.call(&call).await?,
            });
        }
        Ok(out)
    }

    async fn close(&self) {}

    async fn objects(
        &self,
        _kind: ObjectKind,
        _parent: Option<&str>,
    ) -> AppResult<Vec<ObjectSummary>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthands_parse() {
        assert_eq!(
            parse_command("QUERY messages:list").unwrap(),
            Call {
                kind: FunctionKind::Query,
                path: "messages:list".into(),
                args: Map::new()
            }
        );
        assert_eq!(
            parse_command("MUTATE messages:send {\"body\": \"hi\"}")
                .unwrap()
                .args["body"],
            json!("hi")
        );
        assert_eq!(
            parse_command("messages:list").unwrap().kind,
            FunctionKind::Query
        );
        assert_eq!(
            parse_command("ACTION summarize:run").unwrap().kind,
            FunctionKind::Action
        );
        assert!(parse_command("QUERY").is_err());
        assert!(parse_command("QUERY f [1,2]").is_err());
        assert!(parse_command("").is_err());
    }

    #[test]
    fn json_bodies_parse() {
        let call = parse_command(r#"{"function": "messages:list", "args": {"n": 5}}"#).unwrap();
        assert_eq!(call.path, "messages:list");
        assert_eq!(call.kind, FunctionKind::Query);
        assert_eq!(call.args["n"], json!(5));
        let call = parse_command(r#"{"path": "m:run", "kind": "mutation"}"#).unwrap();
        assert_eq!(call.kind, FunctionKind::Mutation);
        assert!(parse_command(r#"{"args": {}}"#).is_err());
        assert!(parse_command("{oops").is_err());
    }

    #[test]
    fn responses_map() {
        let rs =
            response_to_result(&json!({"status": "success", "value": [{"id": 1}], "logLines": []}))
                .unwrap();
        assert_eq!(rs.rows.len(), 1);
        let rs = response_to_result(&json!({"status": "success", "value": 42})).unwrap();
        assert_eq!(rs.columns[0].name, "result");
        assert!(response_to_result(&json!({"status": "error", "errorMessage": "boom"})).is_err());
    }
}
