// SOT: rabbitmq-integration, rabbitmq-management-api, queue-browser, rabbitmq-console-commands, rabbitmq-object-explorer, rabbitmq-server-stats

use crate::error::{AppError, AppResult};
use crate::integrations::http::local;
use crate::integrations::http::{json_to_value, HttpClient};
use crate::integrations::kafka::split_statements;
use crate::integrations::sql::validate_columns;
use crate::integrations::{Capabilities, Integration};
use crate::model::{
    CodeLanguage, ColumnInfo, ColumnMeta, Engine, FilterRule, ObjectAction, ObjectDetail,
    ObjectKind, ObjectRef, ObjectSummary, PageQuery, ResolvedConnection, ResultSet, SchemaCatalog,
    SchemaInfo, ServerStats, Stat, StatGroup, StatementResult, TableInfo, TableKind, TableRef,
    Value,
};
use async_trait::async_trait;
use reqwest::Method;
use serde_json::{json, Map, Value as Json};
use std::sync::Arc;

// ============================================================================
// RABBITMQ ADAPTER (management HTTP API)
//
// WHAT:  Maps a RabbitMQ broker onto the engine-neutral `Integration` through
//        the management plugin's HTTP API (no AMQP crate is spoken: publishing
//        test messages and browsing queues is the workbench use case, and the
//        management API answers all of it over one authenticated client).
// WHY:   Queues are the browsable unit (one table per queue, one row per
//        message); vhosts are the namespaces; exchanges are explorer objects
//        with publish/delete actions.
// HOW:   vhost      = `database` field, default "/" (encoded as %2F in paths)
//        catalog    = one schema per vhost, one table per queue
//        fetch_page = POST /queues/{v}/{q}/get (requeue, never consumes),
//                     then http::local::page for filter/sort/slice
//        count      = messages_ready from the queue info (exact)
//        execute    = JSON {"queue", …} / {"publish": {…}} / {"declare_queue"…}
//                     or the shorthands `QUEUES`, `EXCHANGES`, `CONSUME <q> [n]`
//        `reqwest` is used only for verb+path request shapes; auth, timeouts
//        and status mapping stay in integrations::http::HttpClient.
// WHERE: src-tauri/src/integrations/mod.rs (trait), src/lib/engines.ts (UI meta)
// ============================================================================

const DEFAULT_PORT: u16 = 15672;
const DEFAULT_VHOST: &str = "/";
const MAX_GET: u64 = 1_000;
const OBJECT_CAP: usize = 2_000;
const VHOST_WALK: usize = 20;
const COLUMN_NAMES: [&str; 6] = [
    "payload",
    "routing_key",
    "exchange",
    "redelivered",
    "message_count",
    "properties",
];

pub struct RabbitmqIntegration {
    engine: Engine,
    http: HttpClient,
    vhost: String,
    read_only: bool,
}

// WHAT:  Percent-encode one path segment (`/` → %2F, the default vhost).
fn pct(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn json_string(value: &Json) -> String {
    match value {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn opt_i64(value: Option<&Json>) -> Option<i64> {
    value.and_then(Json::as_i64)
}

pub async fn connect(conn: &ResolvedConnection) -> AppResult<Arc<dyn Integration>> {
    let s = &conn.summary;
    let base = crate::integrations::http::base_url(conn, Some(DEFAULT_PORT), false);
    let insecure = s.ssl_mode == crate::model::SslMode::Require;
    let http = HttpClient::new(
        format!("{base}/api"),
        HttpClient::auth_from_connection(conn),
        insecure,
    )?;
    let vhost = s
        .database
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .unwrap_or(DEFAULT_VHOST)
        .to_string();
    let integration = RabbitmqIntegration {
        engine: s.engine,
        http,
        vhost,
        read_only: s.read_only,
    };
    integration.ping().await?;
    Ok(Arc::new(integration))
}

// ---------------------------------------------------------------------------
// Management API shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct QueueInfo {
    name: String,
    vhost: String,
    durable: bool,
    messages: i64,
    messages_ready: i64,
    messages_unacknowledged: i64,
    consumers: i64,
}

fn parse_queue(v: &Json) -> Option<QueueInfo> {
    let o = v.as_object()?;
    Some(QueueInfo {
        name: o.get("name")?.as_str()?.to_string(),
        vhost: o
            .get("vhost")
            .and_then(Json::as_str)
            .unwrap_or(DEFAULT_VHOST)
            .to_string(),
        durable: o.get("durable").and_then(Json::as_bool).unwrap_or(false),
        messages: opt_i64(o.get("messages")).unwrap_or(0),
        messages_ready: opt_i64(o.get("messages_ready")).unwrap_or(0),
        messages_unacknowledged: opt_i64(o.get("messages_unacknowledged")).unwrap_or(0),
        consumers: opt_i64(o.get("consumers")).unwrap_or(0),
    })
}

#[derive(Debug, Clone)]
struct ExchangeInfo {
    name: String,
    vhost: String,
    kind: String,
    durable: bool,
}

fn parse_exchange(v: &Json) -> Option<ExchangeInfo> {
    let o = v.as_object()?;
    Some(ExchangeInfo {
        name: o.get("name")?.as_str()?.to_string(),
        vhost: o
            .get("vhost")
            .and_then(Json::as_str)
            .unwrap_or(DEFAULT_VHOST)
            .to_string(),
        kind: o
            .get("type")
            .and_then(Json::as_str)
            .unwrap_or("direct")
            .to_string(),
        durable: o.get("durable").and_then(Json::as_bool).unwrap_or(false),
    })
}

// WHAT:  One GET message → grid row (fixed column order, see COLUMN_NAMES).
fn message_row(item: &Json) -> Vec<Value> {
    let o = item.as_object();
    let payload = o.and_then(|m| m.get("payload"));
    let encoding = o
        .and_then(|m| m.get("payload_encoding"))
        .and_then(Json::as_str)
        .unwrap_or("string");
    let body = match (payload, encoding) {
        (Some(Json::String(s)), "base64") => Value::Bytes(s.clone()),
        (Some(v), _) => {
            let text = json_string(v);
            let trimmed = text.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if let Ok(json) = serde_json::from_str::<Json>(&text) {
                    if json.is_object() || json.is_array() {
                        return row_with(Value::Json(json), o);
                    }
                }
            }
            Value::Text(text)
        }
        (None, _) => Value::Null,
    };
    row_with(body, o)
}

fn row_with(body: Value, o: Option<&Map<String, Json>>) -> Vec<Value> {
    vec![
        body,
        o.and_then(|m| m.get("routing_key"))
            .map(json_string)
            .map(Value::Text)
            .unwrap_or(Value::Null),
        o.and_then(|m| m.get("exchange"))
            .map(json_string)
            .map(Value::Text)
            .unwrap_or(Value::Null),
        o.and_then(|m| m.get("redelivered"))
            .and_then(Json::as_bool)
            .map(Value::Bool)
            .unwrap_or(Value::Null),
        o.and_then(|m| m.get("message_count"))
            .and_then(Json::as_i64)
            .map(Value::Int)
            .unwrap_or(Value::Null),
        o.and_then(|m| m.get("properties"))
            .map(|p| json_to_value(p))
            .unwrap_or(Value::Null),
    ]
}

fn fixed_columns() -> Vec<ColumnInfo> {
    let types = ["text", "text", "text", "boolean", "int", "json"];
    COLUMN_NAMES
        .iter()
        .zip(types)
        .enumerate()
        .map(|(i, (name, ty))| ColumnInfo {
            name: (*name).to_string(),
            data_type: (*ty).to_string(),
            nullable: true,
            primary_key: false,
            ordinal: u32::try_from(i + 1).unwrap_or(u32::MAX),
        })
        .collect()
}

fn metas() -> Vec<ColumnMeta> {
    fixed_columns()
        .into_iter()
        .map(|c| ColumnMeta {
            name: c.name,
            type_name: c.data_type,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Console commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Queues,
    Exchanges,
    Consume {
        queue: String,
        limit: u64,
    },
    Publish {
        exchange: String,
        routing_key: String,
        payload: String,
        headers: Map<String, Json>,
    },
    DeclareQueue {
        queue: String,
        durable: bool,
    },
    DeleteQueue {
        queue: String,
    },
    PurgeQueue {
        queue: String,
    },
    DeclareExchange {
        exchange: String,
        kind: String,
        durable: bool,
    },
    DeleteExchange {
        exchange: String,
    },
}

// WHAT:  JSON body or shorthand → Command. `max_rows` caps every consume.
pub fn parse_command(text: &str, max_rows: usize) -> AppResult<Command> {
    let text = text.trim();
    let cap = u64::try_from(max_rows).unwrap_or(u64::MAX).min(MAX_GET);
    if text.starts_with('{') {
        let json: Json = serde_json::from_str(text)
            .map_err(|e| AppError::invalid_input(format!("Command is not valid JSON: {e}")))?;
        let obj = json
            .as_object()
            .ok_or_else(|| AppError::invalid_input("Command must be a JSON object."))?;
        if let Some(publish) = obj.get("publish") {
            let p = publish
                .as_object()
                .ok_or_else(|| AppError::invalid_input("\"publish\" must be an object."))?;
            let exchange = p
                .get("exchange")
                .map(json_string)
                .filter(|e| !e.is_empty())
                .unwrap_or_default();
            let routing_key = p.get("routing_key").map(json_string).unwrap_or_default();
            let payload = p
                .get("payload")
                .map(json_string)
                .ok_or_else(|| AppError::invalid_input("\"publish.payload\" is required."))?;
            let headers = p
                .get("headers")
                .and_then(Json::as_object)
                .cloned()
                .unwrap_or_default();
            return Ok(Command::Publish {
                exchange,
                routing_key,
                payload,
                headers,
            });
        }
        if let Some(declare) = obj.get("declare_queue") {
            let queue = queue_name(declare)?;
            let durable = declare
                .as_object()
                .and_then(|d| d.get("durable"))
                .and_then(Json::as_bool)
                .unwrap_or(true);
            return Ok(Command::DeclareQueue { queue, durable });
        }
        if let Some(delete) = obj.get("delete_queue") {
            return Ok(Command::DeleteQueue {
                queue: queue_name(delete)?,
            });
        }
        if let Some(purge) = obj.get("purge") {
            return Ok(Command::PurgeQueue {
                queue: queue_name(purge)?,
            });
        }
        if let Some(declare) = obj.get("declare_exchange") {
            let (exchange, kind, durable) = exchange_spec(declare)?;
            return Ok(Command::DeclareExchange {
                exchange,
                kind,
                durable,
            });
        }
        if let Some(delete) = obj.get("delete_exchange") {
            return Ok(Command::DeleteExchange {
                exchange: queue_name(delete)?,
            });
        }
        if obj.get("queues").is_some() && obj.get("queue").is_none() {
            return Ok(Command::Queues);
        }
        if obj.get("exchanges").is_some() && obj.get("exchange").is_none() {
            return Ok(Command::Exchanges);
        }
        let queue = obj
            .get("queue")
            .map(json_string)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| AppError::invalid_input("\"queue\" is required."))?;
        let limit = obj
            .get("limit")
            .and_then(Json::as_u64)
            .unwrap_or(cap.min(100))
            .clamp(1, cap);
        return Ok(Command::Consume { queue, limit });
    }
    let mut words = text.split_whitespace();
    match words.next().unwrap_or_default().to_ascii_lowercase().as_str() {
        "queues" => Ok(Command::Queues),
        "exchanges" => Ok(Command::Exchanges),
        "consume" => {
            let queue = words.next().ok_or_else(|| AppError::invalid_input("Usage: CONSUME <queue> [n]"))?.to_string();
            let limit = words.next().map(|n| n.parse::<u64>()).transpose().map_err(|_| AppError::invalid_input("Usage: CONSUME <queue> [n]"))?;
            Ok(Command::Consume { queue, limit: limit.unwrap_or(cap.min(100)).clamp(1, cap) })
        }
        "publish" => {
            let exchange = words.next().ok_or_else(|| AppError::invalid_input("Usage: PUBLISH <exchange> <routing_key> <payload>"))?.to_string();
            let routing_key = words.next().ok_or_else(|| AppError::invalid_input("Usage: PUBLISH <exchange> <routing_key> <payload>"))?.to_string();
            let payload = words.collect::<Vec<_>>().join(" ");
            if payload.is_empty() {
                return Err(AppError::invalid_input("Usage: PUBLISH <exchange> <routing_key> <payload>"));
            }
            Ok(Command::Publish { exchange, routing_key, payload, headers: Map::new() })
        }
        _ => Err(AppError::invalid_input(
            "Enter `QUEUES`, `EXCHANGES`, `CONSUME <queue> [n]`, `PUBLISH <exchange> <routing_key> <payload>`, or a JSON body {\"queue\": \"…\"}, {\"publish\": {\"exchange\": \"…\", \"payload\": \"…\"}}, {\"declare_queue\": {\"queue\": \"…\"}} or {\"delete_queue\": \"…\"}.",
        )),
    }
}

fn queue_name(v: &Json) -> AppResult<String> {
    v.as_object()
        .and_then(|d| d.get("queue"))
        .map(json_string)
        .or_else(|| v.as_str().map(str::to_string))
        .filter(|q| !q.is_empty())
        .ok_or_else(|| AppError::invalid_input("A \"queue\" name is required."))
}

fn exchange_spec(v: &Json) -> AppResult<(String, String, bool)> {
    let name = v
        .as_object()
        .and_then(|d| d.get("exchange"))
        .map(json_string)
        .or_else(|| v.as_str().map(str::to_string))
        .filter(|e| !e.is_empty())
        .ok_or_else(|| AppError::invalid_input("An \"exchange\" name is required."))?;
    let kind = v
        .as_object()
        .and_then(|d| d.get("type"))
        .map(json_string)
        .filter(|k| !k.is_empty())
        .unwrap_or_else(|| "direct".to_string());
    let durable = v
        .as_object()
        .and_then(|d| d.get("durable"))
        .and_then(Json::as_bool)
        .unwrap_or(true);
    Ok((name, kind, durable))
}

// ---------------------------------------------------------------------------
// Integration
// ---------------------------------------------------------------------------

impl RabbitmqIntegration {
    fn vhost_path(&self, vhost: &str) -> String {
        pct(vhost)
    }

    fn queue_path(&self, vhost: &str, queue: &str) -> String {
        format!("queues/{}/{}", self.vhost_path(vhost), pct(queue))
    }

    async fn vhosts(&self) -> AppResult<Vec<String>> {
        let v: Json = self.http.get_json("vhosts").await?;
        let mut names: Vec<String> = v
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.get("name").and_then(Json::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        Ok(names)
    }

    async fn queues_in(&self, vhost: &str) -> AppResult<Vec<QueueInfo>> {
        let v: Json = self
            .http
            .get_json(&format!("queues/{}", self.vhost_path(vhost)))
            .await?;
        let mut out: Vec<QueueInfo> = v
            .as_array()
            .map(|a| a.iter().filter_map(parse_queue).collect())
            .unwrap_or_default();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn exchanges_in(&self, vhost: &str) -> AppResult<Vec<ExchangeInfo>> {
        let v: Json = self
            .http
            .get_json(&format!("exchanges/{}", self.vhost_path(vhost)))
            .await?;
        let mut out: Vec<ExchangeInfo> = v
            .as_array()
            .map(|a| a.iter().filter_map(parse_exchange).collect())
            .unwrap_or_default();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn queue_info(&self, vhost: &str, queue: &str) -> AppResult<QueueInfo> {
        let v: Json = self.http.get_json(&self.queue_path(vhost, queue)).await?;
        parse_queue(&v).ok_or_else(|| {
            AppError::not_found(format!("Queue \"{queue}\" not found in vhost \"{vhost}\"."))
        })
    }

    // WHAT:  Peek at most `count` messages without consuming them: requeue mode
    //        puts every delivery straight back, so browsing never loses data.
    async fn peek(&self, vhost: &str, queue: &str, count: u64) -> AppResult<Vec<Json>> {
        let body = json!({ "count": count, "ackmode": "ack_requeue_true", "encoding": "auto", "truncate": 50000 });
        let v: Json = self
            .http
            .post_json(&format!("{}/get", self.queue_path(vhost, queue)), &body)
            .await?;
        Ok(v.as_array().cloned().unwrap_or_default())
    }

    fn resolve_vhost(&self, table: &TableRef) -> String {
        table
            .schema
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.vhost.clone())
    }

    async fn queues_result(&self) -> AppResult<ResultSet> {
        let mut docs = Vec::new();
        for vhost in self.vhosts().await?.into_iter().take(VHOST_WALK) {
            for q in self
                .queues_in(&vhost)
                .await
                .unwrap_or_default()
                .into_iter()
                .take(OBJECT_CAP)
            {
                docs.push(json!({
                    "vhost": q.vhost, "queue": q.name, "durable": q.durable,
                    "messages": q.messages, "ready": q.messages_ready,
                    "unacknowledged": q.messages_unacknowledged, "consumers": q.consumers,
                }));
            }
        }
        Ok(crate::integrations::http::objects_to_result_set(
            &docs,
            None,
            usize::MAX,
        ))
    }

    async fn exchanges_result(&self) -> AppResult<ResultSet> {
        let mut docs = Vec::new();
        for vhost in self.vhosts().await?.into_iter().take(VHOST_WALK) {
            for e in self
                .exchanges_in(&vhost)
                .await
                .unwrap_or_default()
                .into_iter()
                .take(OBJECT_CAP)
            {
                docs.push(json!({ "vhost": e.vhost, "exchange": e.name, "type": e.kind, "durable": e.durable }));
            }
        }
        Ok(crate::integrations::http::objects_to_result_set(
            &docs,
            None,
            usize::MAX,
        ))
    }

    async fn consume(&self, vhost: &str, queue: &str, limit: u64) -> AppResult<ResultSet> {
        self.queue_info(vhost, queue).await?;
        let mut items = self.peek(vhost, queue, limit.min(MAX_GET).max(1)).await?;
        // The GET endpoint returns oldest first; the default view is newest first.
        items.reverse();
        let rows = items.iter().map(message_row).collect();
        Ok(ResultSet {
            columns: metas(),
            rows,
            truncated: false,
        })
    }

    async fn publish(
        &self,
        vhost: &str,
        exchange: &str,
        routing_key: &str,
        payload: &str,
        headers: Map<String, Json>,
    ) -> AppResult<StatementResult> {
        if self.read_only {
            return Err(AppError::read_only(
                "This connection is read-only; publishing is blocked.",
            ));
        }
        let target = if exchange.is_empty() {
            "amq.default"
        } else {
            exchange
        };
        let body = json!({ "routing_key": routing_key, "payload": payload, "payload_encoding": "string", "properties": { "headers": headers } });
        let v: Json = self
            .http
            .post_json(
                &format!(
                    "exchanges/{}/{}/publish",
                    self.vhost_path(vhost),
                    pct(target)
                ),
                &body,
            )
            .await?;
        let routed = v.get("routed").and_then(Json::as_bool).unwrap_or(false);
        if routed {
            Ok(StatementResult::Affected { rows_affected: 1 })
        } else {
            Err(AppError::invalid_input(format!("Message accepted but routed nowhere (exchange \"{target}\", routing key \"{routing_key}\").")))
        }
    }

    async fn declare_queue(
        &self,
        vhost: &str,
        queue: &str,
        durable: bool,
    ) -> AppResult<StatementResult> {
        if self.read_only {
            return Err(AppError::read_only(
                "This connection is read-only; declaring a queue is blocked.",
            ));
        }
        let body = json!({ "durable": durable, "auto_delete": false, "arguments": {} });
        self.http
            .send(
                self.http
                    .request(Method::PUT, &self.queue_path(vhost, queue))
                    .json(&body),
            )
            .await?;
        Ok(StatementResult::Affected { rows_affected: 1 })
    }

    async fn delete_queue(&self, vhost: &str, queue: &str) -> AppResult<StatementResult> {
        if self.read_only {
            return Err(AppError::read_only(
                "This connection is read-only; deleting a queue is blocked.",
            ));
        }
        self.http
            .send(
                self.http
                    .request(Method::DELETE, &self.queue_path(vhost, queue)),
            )
            .await?;
        Ok(StatementResult::Affected { rows_affected: 1 })
    }

    async fn purge_queue(&self, vhost: &str, queue: &str) -> AppResult<StatementResult> {
        if self.read_only {
            return Err(AppError::read_only(
                "This connection is read-only; purging a queue is blocked.",
            ));
        }
        self.queue_info(vhost, queue).await?;
        self.http
            .send(self.http.request(
                Method::DELETE,
                &format!("{}/contents", self.queue_path(vhost, queue)),
            ))
            .await?;
        Ok(StatementResult::Affected { rows_affected: 1 })
    }

    async fn declare_exchange(
        &self,
        vhost: &str,
        exchange: &str,
        kind: &str,
        durable: bool,
    ) -> AppResult<StatementResult> {
        if self.read_only {
            return Err(AppError::read_only(
                "This connection is read-only; declaring an exchange is blocked.",
            ));
        }
        let body =
            json!({ "type": kind, "durable": durable, "auto_delete": false, "arguments": {} });
        let path = format!("exchanges/{}/{}", self.vhost_path(vhost), pct(exchange));
        self.http
            .send(self.http.request(Method::PUT, &path).json(&body))
            .await?;
        Ok(StatementResult::Affected { rows_affected: 1 })
    }

    async fn delete_exchange(&self, vhost: &str, exchange: &str) -> AppResult<StatementResult> {
        if self.read_only {
            return Err(AppError::read_only(
                "This connection is read-only; deleting an exchange is blocked.",
            ));
        }
        let path = format!("exchanges/{}/{}", self.vhost_path(vhost), pct(exchange));
        self.http
            .send(self.http.request(Method::DELETE, &path))
            .await?;
        Ok(StatementResult::Affected { rows_affected: 1 })
    }
}

// ---------------------------------------------------------------------------
// Object explorer
// ---------------------------------------------------------------------------

impl RabbitmqIntegration {
    async fn queue_objects(&self, parent: Option<&str>) -> AppResult<Vec<ObjectSummary>> {
        let vhosts: Vec<String> = match parent {
            Some(v) => vec![v.to_string()],
            None => self.vhosts().await?.into_iter().take(VHOST_WALK).collect(),
        };
        let mut out = Vec::new();
        for vhost in vhosts {
            for q in self
                .queues_in(&vhost)
                .await
                .unwrap_or_default()
                .into_iter()
                .take(OBJECT_CAP)
            {
                out.push(
                    ObjectSummary::new(ObjectKind::Queue, q.name.clone(), Some(q.vhost.clone()))
                        .with_detail(format!(
                            "{} messages · {} consumers",
                            q.messages, q.consumers
                        ))
                        .with_badge(if q.durable {
                            "durable".to_string()
                        } else {
                            "transient".to_string()
                        }),
                );
            }
        }
        out.sort_by(|a, b| {
            a.reference
                .parent
                .cmp(&b.reference.parent)
                .then_with(|| a.reference.name.cmp(&b.reference.name))
        });
        out.truncate(OBJECT_CAP);
        Ok(out)
    }

    async fn exchange_objects(&self, parent: Option<&str>) -> AppResult<Vec<ObjectSummary>> {
        let vhosts: Vec<String> = match parent {
            Some(v) => vec![v.to_string()],
            None => self.vhosts().await?.into_iter().take(VHOST_WALK).collect(),
        };
        let mut out = Vec::new();
        for vhost in vhosts {
            for e in self
                .exchanges_in(&vhost)
                .await
                .unwrap_or_default()
                .into_iter()
                .take(OBJECT_CAP)
            {
                if e.name.is_empty() {
                    continue;
                }
                out.push(
                    ObjectSummary::new(ObjectKind::Exchange, e.name.clone(), Some(e.vhost.clone()))
                        .with_detail(format!("{} exchange", e.kind)),
                );
            }
        }
        out.sort_by(|a, b| {
            a.reference
                .parent
                .cmp(&b.reference.parent)
                .then_with(|| a.reference.name.cmp(&b.reference.name))
        });
        out.truncate(OBJECT_CAP);
        Ok(out)
    }

    fn vhost_of(&self, reference: &ObjectRef) -> String {
        reference
            .parent
            .clone()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| self.vhost.clone())
    }

    async fn queue_detail(&self, reference: &ObjectRef) -> AppResult<ObjectDetail> {
        let vhost = self.vhost_of(reference);
        let q = self.queue_info(&vhost, &reference.name).await?;
        let consume = json!({ "queue": q.name, "limit": 100 }).to_string();
        let publish =
            json!({ "publish": { "exchange": "", "routing_key": q.name, "payload": "hello" } })
                .to_string();
        let mut detail = ObjectDetail::empty(reference)
            .definition(consume.clone(), CodeLanguage::Json)
            .property("vhost", q.vhost.clone())
            .property("durable", q.durable.to_string())
            .property("messages", q.messages.to_string())
            .property("ready", q.messages_ready.to_string())
            .property("unacknowledged", q.messages_unacknowledged.to_string())
            .property("consumers", q.consumers.to_string());
        detail.columns = fixed_columns();
        detail = detail
            .action(ObjectAction::new("consume", "Peek messages", consume))
            .action(ObjectAction::new(
                "publish",
                "Publish via default exchange",
                publish,
            ))
            .action(ObjectAction::destructive(
                "purge",
                "Purge messages",
                json!({ "purge": q.name }).to_string(),
            ))
            .action(ObjectAction::destructive(
                "delete",
                "Delete queue",
                json!({ "delete_queue": q.name }).to_string(),
            ));
        Ok(detail)
    }

    async fn exchange_detail(&self, reference: &ObjectRef) -> AppResult<ObjectDetail> {
        let vhost = self.vhost_of(reference);
        let found = self
            .exchanges_in(&vhost)
            .await?
            .into_iter()
            .find(|e| e.name == reference.name)
            .ok_or_else(|| {
                AppError::not_found(format!(
                    "Exchange \"{}\" not found in vhost \"{}\".",
                    reference.name, vhost
                ))
            })?;
        let publish =
            json!({ "publish": { "exchange": found.name, "routing_key": "", "payload": "hello" } })
                .to_string();
        let mut detail = ObjectDetail::empty(reference)
            .definition(publish.clone(), CodeLanguage::Json)
            .property("vhost", found.vhost.clone())
            .property("type", found.kind.clone())
            .property("durable", found.durable.to_string());
        detail = detail
            .action(ObjectAction::new("publish", "Publish", publish))
            .action(ObjectAction::destructive(
                "delete",
                "Delete exchange",
                json!({ "delete_exchange": found.name }).to_string(),
            ));
        Ok(detail)
    }

    async fn stats(&self) -> AppResult<ServerStats> {
        let overview: Json = self.http.get_json("overview").await?;
        let o = overview.as_object();
        let num = |key: &str| {
            o.and_then(|m| m.get(key))
                .and_then(Json::as_str)
                .and_then(|s| s.parse::<f64>().ok())
        };
        let obj_total = |key: &str| {
            o.and_then(|m| m.get("object_totals"))
                .and_then(Json::as_object)
                .and_then(|t| t.get(key))
                .and_then(Json::as_u64)
                .map(|n| n as f64)
        };
        let queue_total = |key: &str| {
            o.and_then(|m| m.get("queue_totals"))
                .and_then(Json::as_object)
                .and_then(|t| t.get(key))
                .and_then(Json::as_u64)
                .map(|n| n as f64)
        };
        let server = vec![
            Stat::text("Engine", "RabbitMQ"),
            Stat::text(
                "Version",
                o.and_then(|m| m.get("rabbitmq_version"))
                    .map(json_string)
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
            Stat::text(
                "Management",
                o.and_then(|m| m.get("management_version"))
                    .map(json_string)
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
            Stat::text(
                "Node",
                o.and_then(|m| m.get("node"))
                    .map(json_string)
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
            Stat::text("Vhost", self.vhost.clone()),
        ];
        let mut queues = vec![
            Stat::number("Queues", obj_total("queues").unwrap_or(0.0), None),
            Stat::number("Exchanges", obj_total("exchanges").unwrap_or(0.0), None),
            Stat::number("Messages", queue_total("messages").unwrap_or(0.0), None),
            Stat::number("Ready", queue_total("messages_ready").unwrap_or(0.0), None),
            Stat::number(
                "Unacknowledged",
                queue_total("messages_unacknowledged").unwrap_or(0.0),
                None,
            ),
        ];
        if let Some(rate) = num("messages_details.rate").or_else(|| {
            o.and_then(|m| m.get("message_stats"))
                .and_then(Json::as_object)
                .and_then(|t| t.get("publish_details"))
                .and_then(Json::as_object)
                .and_then(|t| t.get("rate"))
                .and_then(Json::as_f64)
        }) {
            queues.push(Stat::number("Publish rate", rate, Some("msg/s")));
        }
        let activity = vec![
            Stat::number("Connections", obj_total("connections").unwrap_or(0.0), None),
            Stat::number("Channels", obj_total("channels").unwrap_or(0.0), None),
            Stat::number("Consumers", obj_total("consumers").unwrap_or(0.0), None),
        ];
        let groups = [
            ("Server", server),
            ("Queues", queues),
            ("Activity", activity),
        ]
        .into_iter()
        .map(|(title, stats)| StatGroup {
            title: title.to_string(),
            stats,
        })
        .collect();
        Ok(ServerStats::now(groups))
    }
}

// WHAT:  What this family offers the object explorer and the tool tabs.
// WHY:   Declared here, next to the adapter that must answer `objects()` for
//        every kind listed; rendered by the capability matrix for every engine.
// WHERE: src-tauri/src/integrations/mod.rs (FamilyProfile), src/lib/objects.ts
pub fn profile() -> crate::integrations::FamilyProfile {
    use crate::model::{ObjectKind as K, Tool as T};
    crate::integrations::FamilyProfile {
        capabilities: Capabilities {
            describes_fields: true,
            sql: false,
            namespaces: true,
            fixed_columns: true,
            paging: true,
            row_estimate: true,
            views: false,
            transactions: false,
            exact_estimate: true,
        },
        object_kinds: vec![K::Queue, K::Exchange],
        tools: vec![T::Stats, T::MessageViewer],
    }
}

#[async_trait]
impl Integration for RabbitmqIntegration {
    fn engine(&self) -> Engine {
        self.engine
    }

    fn capabilities(&self) -> Capabilities {
        profile().capabilities
    }

    async fn ping(&self) -> AppResult<()> {
        let _: Json = self.http.get_json("overview").await?;
        Ok(())
    }

    async fn server_version(&self) -> AppResult<Option<String>> {
        let overview: Json = self.http.get_json("overview").await?;
        Ok(overview
            .get("rabbitmq_version")
            .and_then(Json::as_str)
            .map(|v| format!("RabbitMQ {v}"))
            .or_else(|| Some("RabbitMQ".to_string())))
    }

    fn current_database(&self) -> Option<String> {
        Some(self.vhost.clone())
    }

    async fn databases(&self) -> AppResult<Vec<String>> {
        self.vhosts().await
    }

    async fn catalog(&self) -> AppResult<SchemaCatalog> {
        let mut schemas = Vec::new();
        for vhost in self.vhosts().await?.into_iter().take(VHOST_WALK) {
            let mut tables = Vec::new();
            for q in self.queues_in(&vhost).await.unwrap_or_default() {
                tables.push(TableInfo {
                    schema: Some(vhost.clone()),
                    name: q.name,
                    kind: TableKind::Table,
                    row_estimate: Some(q.messages_ready),
                });
            }
            schemas.push(SchemaInfo {
                name: vhost,
                tables,
            });
        }
        Ok(SchemaCatalog { schemas })
    }

    async fn columns(&self, table: &TableRef) -> AppResult<Vec<ColumnInfo>> {
        let vhost = self.resolve_vhost(table);
        // Resolving first turns a typo into "not found" instead of an empty grid.
        self.queue_info(&vhost, &table.name).await?;
        Ok(fixed_columns())
    }

    async fn row_estimate(&self, table: &TableRef) -> AppResult<Option<i64>> {
        let vhost = self.resolve_vhost(table);
        Ok(Some(
            self.queue_info(&vhost, &table.name).await?.messages_ready,
        ))
    }

    async fn count(&self, table: &TableRef, filters: &[FilterRule]) -> AppResult<i64> {
        let vhost = self.resolve_vhost(table);
        if filters.is_empty() {
            return Ok(self.queue_info(&vhost, &table.name).await?.messages_ready);
        }
        let items = self.peek(&vhost, &table.name, MAX_GET).await?;
        let names: Vec<String> = COLUMN_NAMES.iter().map(|n| (*n).to_string()).collect();
        Ok(
            local::apply_filters(&names, items.iter().map(message_row).collect(), filters).len()
                as i64,
        )
    }

    async fn fetch_page(&self, table: &TableRef, query: &PageQuery) -> AppResult<ResultSet> {
        let cols = self.columns(table).await?;
        validate_columns(&cols, &query.sort, &query.filters)?;
        let vhost = self.resolve_vhost(table);
        let wanted = query
            .offset
            .saturating_add(u64::from(query.limit))
            .min(MAX_GET);
        let mut items = self.peek(&vhost, &table.name, wanted.max(1)).await?;
        // The GET endpoint returns oldest first; the default view is newest first.
        if query.sort.is_empty() && query.filters.is_empty() {
            items.reverse();
        }
        let names: Vec<String> = COLUMN_NAMES.iter().map(|n| (*n).to_string()).collect();
        let rows = local::page(&names, items.iter().map(message_row).collect(), query);
        Ok(ResultSet {
            columns: metas(),
            rows,
            truncated: false,
        })
    }

    async fn execute(&self, script: &str, max_rows: usize) -> AppResult<Vec<StatementResult>> {
        let statements = split_statements(script);
        if statements.is_empty() {
            return Err(AppError::invalid_input("Nothing to run."));
        }
        let mut out = Vec::with_capacity(statements.len());
        for statement in statements {
            let result = match parse_command(statement, max_rows)? {
                Command::Queues => StatementResult::Rows {
                    result: self.queues_result().await?,
                },
                Command::Exchanges => StatementResult::Rows {
                    result: self.exchanges_result().await?,
                },
                Command::Consume { queue, limit } => StatementResult::Rows {
                    result: self.consume(&self.vhost, &queue, limit).await?,
                },
                Command::Publish {
                    exchange,
                    routing_key,
                    payload,
                    headers,
                } => {
                    self.publish(&self.vhost, &exchange, &routing_key, &payload, headers)
                        .await?
                }
                Command::DeclareQueue { queue, durable } => {
                    self.declare_queue(&self.vhost, &queue, durable).await?
                }
                Command::DeleteQueue { queue } => self.delete_queue(&self.vhost, &queue).await?,
                Command::PurgeQueue { queue } => self.purge_queue(&self.vhost, &queue).await?,
                Command::DeclareExchange {
                    exchange,
                    kind,
                    durable,
                } => {
                    self.declare_exchange(&self.vhost, &exchange, &kind, durable)
                        .await?
                }
                Command::DeleteExchange { exchange } => {
                    self.delete_exchange(&self.vhost, &exchange).await?
                }
            };
            out.push(result);
        }
        Ok(out)
    }

    async fn close(&self) {}

    fn object_table(&self, reference: &ObjectRef) -> TableRef {
        match reference.kind {
            ObjectKind::Queue => TableRef {
                schema: reference
                    .parent
                    .clone()
                    .filter(|p| !p.is_empty())
                    .or(Some(self.vhost.clone())),
                name: reference.name.clone(),
            },
            _ => TableRef {
                schema: reference.parent.clone().filter(|p| !p.is_empty()),
                name: reference.name.clone(),
            },
        }
    }

    async fn objects(
        &self,
        kind: ObjectKind,
        parent: Option<&str>,
    ) -> AppResult<Vec<ObjectSummary>> {
        match kind {
            ObjectKind::Queue => self.queue_objects(parent).await,
            ObjectKind::Exchange => self.exchange_objects(parent).await,
            _ => Ok(Vec::new()),
        }
    }

    async fn object_detail(&self, reference: &ObjectRef) -> AppResult<ObjectDetail> {
        match reference.kind {
            ObjectKind::Queue => self.queue_detail(reference).await,
            ObjectKind::Exchange => self.exchange_detail(reference).await,
            _ => Ok(ObjectDetail::empty(reference)),
        }
    }

    async fn server_stats(&self) -> AppResult<ServerStats> {
        self.stats().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vhost_and_names_encode() {
        assert_eq!(pct("/"), "%2F");
        assert_eq!(pct("my vhost"), "my%20vhost");
        assert_eq!(pct("orders"), "orders");
    }

    #[test]
    fn shorthands_parse() {
        assert_eq!(parse_command("QUEUES", 10).unwrap(), Command::Queues);
        assert_eq!(parse_command("exchanges", 10).unwrap(), Command::Exchanges);
        assert_eq!(
            parse_command("CONSUME orders 5", 100).unwrap(),
            Command::Consume {
                queue: "orders".into(),
                limit: 5
            }
        );
        assert_eq!(
            parse_command("CONSUME orders", 10).unwrap(),
            Command::Consume {
                queue: "orders".into(),
                limit: 10
            }
        );
        let cmd = parse_command("PUBLISH events user.created {\"id\":1}", 10).unwrap();
        assert_eq!(
            cmd,
            Command::Publish {
                exchange: "events".into(),
                routing_key: "user.created".into(),
                payload: "{\"id\":1}".into(),
                headers: Map::new()
            }
        );
        assert!(parse_command("PUBLISH only-one", 10).is_err());
        assert!(parse_command("DROP orders", 10).is_err());
    }

    #[test]
    fn json_bodies_parse() {
        assert_eq!(
            parse_command(r#"{"queue": "orders", "limit": 7}"#, 100).unwrap(),
            Command::Consume {
                queue: "orders".into(),
                limit: 7
            }
        );
        assert_eq!(
            parse_command(r#"{"declare_queue": {"queue": "q"}}"#, 10).unwrap(),
            Command::DeclareQueue {
                queue: "q".into(),
                durable: true
            }
        );
        assert_eq!(
            parse_command(r#"{"delete_queue": "q"}"#, 10).unwrap(),
            Command::DeleteQueue { queue: "q".into() }
        );
        assert_eq!(
            parse_command(r#"{"purge": "q"}"#, 10).unwrap(),
            Command::PurgeQueue { queue: "q".into() }
        );
        assert_eq!(
            parse_command(
                r#"{"declare_exchange": {"exchange": "e", "type": "topic"}}"#,
                10
            )
            .unwrap(),
            Command::DeclareExchange {
                exchange: "e".into(),
                kind: "topic".into(),
                durable: true
            }
        );
        assert_eq!(
            parse_command(r#"{"delete_exchange": "e"}"#, 10).unwrap(),
            Command::DeleteExchange {
                exchange: "e".into()
            }
        );
        let cmd = parse_command(
            r#"{"publish": {"exchange": "e", "routing_key": "k", "payload": "v"}}"#,
            10,
        )
        .unwrap();
        assert!(matches!(cmd, Command::Publish { .. }));
        assert!(parse_command(r#"{"queue": ""}"#, 10).is_err());
        assert!(parse_command("not json {", 10).is_err());
    }

    #[test]
    fn queue_payloads_shape() {
        let q = json!({"name": "orders", "vhost": "/", "durable": true, "messages": 12, "messages_ready": 9, "messages_unacknowledged": 3, "consumers": 2});
        let info = parse_queue(&q).unwrap();
        assert_eq!(info.name, "orders");
        assert_eq!(info.messages_ready, 9);
        assert!(info.durable);
        assert!(parse_queue(&json!({"vhost": "/"})).is_none());
        let e = json!({"name": "events", "vhost": "/", "type": "topic", "durable": true});
        let ex = parse_exchange(&e).unwrap();
        assert_eq!(ex.kind, "topic");
    }

    #[test]
    fn message_rows_map() {
        let item = json!({"payload": "{\"id\": 1}", "payload_encoding": "string", "routing_key": "k", "exchange": "e", "redelivered": false, "message_count": 4, "properties": {"delivery_mode": 2}});
        let row = message_row(&item);
        assert_eq!(row[0], Value::Json(json!({"id": 1})));
        assert_eq!(row[1], Value::Text("k".into()));
        assert_eq!(row[3], Value::Bool(false));
        assert_eq!(row[4], Value::Int(4));
        let b64 = message_row(&json!({"payload": "aGk=", "payload_encoding": "base64"}));
        assert_eq!(b64[0], Value::Bytes("aGk=".into()));
        assert_eq!(message_row(&json!({}))[0], Value::Null);
    }
}
