// SOT: ai-service, byok-llm, natural-language-to-sql, query-explainer, anthropic-messages-api

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::model::{AiModel, AiProvider, AiSettings, Engine, Family, PlanReport, StatementResult, Value};
use std::time::Duration;

// WHAT:  Bring-your-own-key assistant: schema-aware NL→SQL and plan explanations.
// WHY:   PRD §4.5. Keys never leave the store unsealed except for the request.
// HOW:   One HTTP client; provider shapes: Anthropic Messages API (with default
//        server-side refusal fallbacks), OpenAI-compatible chat completions
// WHERE: src-tauri/src/services/settings.rs (settings + sealed key)
const MAX_SCHEMA_TABLES: usize = 60;

pub struct AiRequest<'a> {
    pub settings: &'a AiSettings,
    pub api_key: Option<&'a str>,
}

fn engine_label(engine: Engine) -> &'static str {
    engine.label()
}

// WHAT:  Dialect rules the assistant follows, keyed by adapter family so every
//        wire-compatible engine (Supabase, Neon, TimescaleDB…) inherits them.
pub fn engine_guidelines(engine: Engine) -> &'static str {
    match engine.family() {
        Family::Postgres => {
            "PostgreSQL Dialect Rules:\n\
             - Use standard PostgreSQL SQL syntax (double-quoted identifiers for mixed-case/reserved names, single quotes for strings).\n\
             - Standard schema is usually 'public'; qualify table names if non-default schema.\n\
             - Use ILIKE for case-insensitive text matching, ::type for casting, and NOW() for current timestamps.\n\
             - Supports CTEs (WITH), window functions, JSONB operators (->, ->>), and RETURNING clauses."
        }
        Family::Mysql => {
            "MySQL Dialect Rules:\n\
             - Use standard MySQL / MariaDB SQL syntax with backticks (`table`, `column`) for identifiers when necessary.\n\
             - Use standard MySQL functions: CONCAT(), DATE_SUB(), NOW(), IFNULL(), COALESCE().\n\
             - Use LIMIT count OFFSET offset for pagination."
        }
        Family::Mssql => {
            "SQL Server (T-SQL) Dialect Rules:\n\
             - Use standard T-SQL syntax with square brackets ([schema].[table], [column]) for identifiers when necessary.\n\
             - Use TOP (n) or OFFSET x ROWS FETCH NEXT y ROWS ONLY for pagination (requires ORDER BY).\n\
             - Use T-SQL functions: GETDATE(), ISNULL(), COALESCE(), LEN(), SUBSTRING(), CHARINDEX().\n\
             - Standard schema is usually 'dbo'."
        }
        Family::Oracle => {
            "Oracle Dialect Rules:\n\
             - Use Oracle SQL: double-quoted identifiers are case-sensitive, unquoted are upper-cased.\n\
             - Use FETCH FIRST n ROWS ONLY / OFFSET n ROWS for pagination, SYSDATE / SYSTIMESTAMP for now, NVL() for null defaults.\n\
             - Do not terminate statements with a semicolon."
        }
        Family::Sqlite | Family::Libsql | Family::ValTown | Family::CloudflareD1 => {
            "SQLite Dialect Rules:\n\
             - Use standard SQLite SQL syntax. SQLite uses dynamic typing.\n\
             - Use SQLite date/time functions: datetime('now'), strftime(), coalesce(), instr().\n\
             - Avoid FULL OUTER JOIN or RIGHT JOIN (unsupported in older versions); use LEFT JOIN or UNION ALL if needed.\n\
             - Use LIMIT count OFFSET offset for pagination."
        }
        Family::Duckdb => {
            "DuckDB Dialect Rules:\n\
             - Use DuckDB SQL (Postgres-like). Supports QUALIFY, EXCLUDE/REPLACE in SELECT *, list/struct types, read_csv()/read_parquet().\n\
             - Use LIMIT count OFFSET offset for pagination; ILIKE is available."
        }
        Family::Clickhouse => {
            "ClickHouse Dialect Rules:\n\
             - Use ClickHouse SQL syntax. Tables may require FINAL modifier if using ReplacingMergeTree.\n\
             - Use ClickHouse functions: formatDateTime(), toDate(), toDateTime(), and specialized array/tuple functions.\n\
             - Identifiers are case-sensitive. Use backticks or double quotes if needed."
        }
        Family::Druid => {
            "Apache Druid SQL Rules:\n\
             - Use Druid SQL (Calcite). Time column is __time; use TIME_FLOOR() for bucketing and double quotes for identifiers.\n\
             - No UPDATE/DELETE; queries are read-only analytics."
        }
        Family::Snowflake => {
            "Snowflake SQL Rules:\n\
             - Use Snowflake SQL: unquoted identifiers are upper-cased, qualify as DATABASE.SCHEMA.TABLE.\n\
             - Use ILIKE, QUALIFY, FLATTEN() for semi-structured VARIANT data, and LIMIT count OFFSET offset."
        }
        Family::Bigquery => {
            "BigQuery SQL Rules:\n\
             - Use GoogleSQL (standard SQL). Qualify tables as `project.dataset.table` with backticks.\n\
             - Use STRING/INT64/FLOAT64 types, UNNEST() for arrays, and LIMIT count OFFSET offset."
        }
        Family::Cassandra => {
            "Cassandra CQL Rules:\n\
             - Output CQL, not SQL. Every SELECT must restrict on the partition key or use ALLOW FILTERING (avoid on large tables).\n\
             - No JOINs, no OFFSET, no arbitrary ORDER BY (only clustering columns). Use LIMIT n."
        }
        Family::Immudb => {
            "immudb SQL Rules:\n\
             - Use immudb SQL (a SQL subset). Supports SELECT/INSERT/UPSERT, no DELETE of history; tables are append-only and cryptographically verified."
        }
        Family::Qldb => {
            "Amazon QLDB PartiQL Rules:\n\
             - Output PartiQL. Documents are Ion; use SELECT * FROM table BY id for document ids, and history(table) for audit trails."
        }
        Family::Surrealdb => {
            "SurrealQL Rules:\n\
             - Output SurrealQL. Records are addressed as table:id; use SELECT * FROM table WHERE …, RELATE for graph edges, and FETCH for joins."
        }
        Family::Orientdb => {
            "OrientDB SQL Rules:\n\
             - Output OrientDB SQL. Classes act as tables (SELECT FROM Class WHERE …), @rid identifies records, TRAVERSE and MATCH walk graph edges."
        }
        Family::Influxdb => {
            "InfluxDB Rules:\n\
             - Output InfluxQL or SQL (v3): SELECT … FROM measurement WHERE time > now() - 1h. Tags are indexed, fields are values. Never output relational DDL."
        }
        Family::Prometheus => {
            "PromQL Rules:\n\
             - Output a PromQL expression, not SQL (e.g. rate(http_requests_total[5m]), sum by (job) (up))."
        }
        Family::Rocksdb => {
            "RocksDB Command Rules:\n\
             - RocksDB is an embedded key-value store. Output commands: GET key, PUT key value, DELETE key, SCAN prefix [limit], KEYS [prefix]. Never output SQL."
        }
        Family::Redis | Family::Memcached => {
            "Redis Command Rules:\n\
             - Redis is a key-value store, NOT a relational SQL database. Never output SQL statements.\n\
             - Output valid single or multi-line Redis CLI commands (e.g. GET key, SET key val, HGETALL key, LRANGE key 0 -1, SCAN 0 MATCH pattern COUNT 100, ZREVRANGEBYSCORE key +inf -inf WITHSCORES LIMIT 0 10)."
        }
        Family::Mongodb => {
            "MongoDB Command Rules:\n\
             - MongoDB is a document store, NOT a SQL database. Never output SQL statements.\n\
             - Output either a MongoDB shell command (e.g. db.collection.find({...}).sort({...}).limit(10)) or a valid JSON query / aggregation pipeline document."
        }
        Family::Couchdb => {
            "CouchDB Mango Rules:\n\
             - Output a Mango selector JSON document ({\"selector\": {...}, \"limit\": 10, \"sort\": [...]}) for _find. Never output SQL."
        }
        Family::Firestore => {
            "Firestore Rules:\n\
             - Output a structured query as JSON (collection, where: [[field, op, value]], orderBy, limit). Never output SQL."
        }
        Family::Dynamodb => {
            "DynamoDB PartiQL Rules:\n\
             - Output PartiQL for DynamoDB: SELECT * FROM \"table\" WHERE pk = 'value'. Filters on non-key attributes scan the table."
        }
        Family::Hbase => {
            "HBase Rules:\n\
             - Output HBase shell-style commands (scan 'table', get 'table', 'row', put 'table', 'row', 'cf:col', 'value'). Never output SQL."
        }
        Family::Neo4j | Family::Tigergraph => {
            "Cypher Rules:\n\
             - Output Cypher (MATCH (n:Label)-[:REL]->(m) WHERE … RETURN … LIMIT 25). Never output SQL."
        }
        Family::Arangodb => {
            "AQL Rules:\n\
             - Output AQL (FOR doc IN collection FILTER … SORT … LIMIT 25 RETURN doc). Never output SQL."
        }
        Family::Qdrant | Family::Milvus | Family::Weaviate | Family::Pinecone | Family::Chroma => {
            "Vector Database Rules:\n\
             - Output a JSON request body for the engine's search / scroll API (filter, limit, with_payload, optional vector). Never output SQL."
        }
        Family::Elasticsearch => {
            "Elasticsearch Query DSL Rules:\n\
             - Output a JSON Query DSL body for _search ({\"query\": {...}, \"size\": 10, \"sort\": [...]}) or an ES|SQL statement. Never output relational DDL."
        }
        Family::Meilisearch | Family::Typesense => {
            "Search Engine Rules:\n\
             - Output a JSON search request ({\"q\": \"…\", \"filter\": \"…\", \"limit\": 20}). Never output SQL."
        }
        Family::Kafka => {
            "Kafka Rules:\n\
             - Output a consume request as JSON ({\"topic\": \"…\", \"partition\": 0, \"offset\": \"latest\", \"limit\": 100}). Never output SQL."
        }
        Family::Rabbitmq => {
            "RabbitMQ Rules:\n\
             - Output a consume request as JSON ({\"queue\": \"…\", \"limit\": 100}), a publish body ({\"publish\": {\"exchange\": \"…\", \"routing_key\": \"…\", \"payload\": \"…\"}}), or one of the shorthands QUEUES, EXCHANGES, CONSUME <queue> [n]. Browsing never consumes: deliveries requeue. Never output SQL."
        }
        Family::Convex => {
            "Convex Rules:\n\
             - Output a function call as JSON ({\"function\": \"module:name\", \"args\": {...}}) with optional \"kind\": \"query\" (default), \"mutation\" or \"action\". Shorthands QUERY <module:function> [{…}], MUTATE … and ACTION … are also accepted. Never output SQL."
        }
        Family::Objectdb => {
            "JPQL Rules:\n\
             - Output JPQL (SELECT e FROM Entity e WHERE e.field = :value). Never output relational DDL."
        }
        Family::S3 => {
            "S3 Command Rules:\n\
             - S3 has no query language: output one of BUCKETS, LIST <bucket> [prefix], GET <bucket> <key>, HEAD <bucket> <key>, DELETE <bucket> <key>.\n\
             - Never output SQL. A prefix is a key prefix such as logs/2026/, not a glob."
        }
        Family::Spacetimedb => {
            "SpacetimeDB SQL Rules:\n\
             - Output the SpacetimeDB SQL subset: SELECT with WHERE, JOIN and LIMIT over module tables.\n\
             - There are no schemas: reference tables by bare name. The catalogue lives in st_table and st_column.\n\
             - Schema changes come from module code, not DDL: never output CREATE TABLE or ALTER TABLE."
        }
        Family::Sparql => {
            "SPARQL Rules:\n\
             - Output SPARQL 1.1 (SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 25) with PREFIX declarations. Never output SQL."
        }
        Family::Basex | Family::Existdb => {
            "XQuery Rules:\n\
             - Output XQuery 3.1 (for $x in db:open('db')//item where … return $x). Never output SQL."
        }
    }
}

pub async fn schema_context(
    ctx: &SessionCtx,
    prompt: &str,
    current_table: Option<&str>,
) -> AppResult<String> {
    let catalog = ctx.integration.catalog().await?;
    let foreign_keys = ctx.integration.foreign_keys().await.unwrap_or_default();
    let server_ver = ctx.integration.server_version().await.unwrap_or(None);

    let mut out = String::new();
    let current_db = ctx.integration.current_database().or_else(|| ctx.connection.database.clone());
    if let Some(db) = current_db {
        out.push_str(&format!("Database: {db}\n"));
    }
    out.push_str(&format!("Engine: {}", engine_label(ctx.connection.engine)));
    if let Some(v) = server_ver {
        out.push_str(&format!(" (Server Version: {v})"));
    }
    out.push('\n');

    if catalog.schemas.len() > 1 {
        if let Some(first) = catalog.schemas.first() {
            out.push_str(&format!("Default Schema: {}\n", first.name));
        }
        let schema_names: Vec<&str> = catalog.schemas.iter().map(|s| s.name.as_str()).collect();
        out.push_str(&format!("Available Schemas: {}\n", schema_names.join(", ")));
    }

    // Collect all tables
    let mut all_tables = Vec::new();
    for schema in &catalog.schemas {
        for table in &schema.tables {
            all_tables.push(table.clone());
        }
    }

    let prompt_lower = prompt.to_lowercase();
    let curr_tbl_lower = current_table.map(|t| t.to_lowercase());

    // Sort/prioritize tables:
    // High priority: matches current_table, or table name appears in prompt
    all_tables.sort_by(|a, b| {
        let a_match = curr_tbl_lower.as_deref() == Some(&a.name.to_lowercase())
            || prompt_lower.contains(&a.name.to_lowercase());
        let b_match = curr_tbl_lower.as_deref() == Some(&b.name.to_lowercase())
            || prompt_lower.contains(&b.name.to_lowercase());
        b_match.cmp(&a_match)
    });

    let total_tables = all_tables.len();
    let tables_to_include = all_tables.into_iter().take(MAX_SCHEMA_TABLES).collect::<Vec<_>>();
    let omitted = total_tables.saturating_sub(MAX_SCHEMA_TABLES);

    out.push_str("Schema Tables:\n");
    for table in &tables_to_include {
        let table_ref = crate::model::TableRef {
            schema: table.schema.clone(),
            name: table.name.clone(),
        };
        let cols = ctx.integration.columns(&table_ref).await.unwrap_or_default();
        let col_text: Vec<String> = cols
            .iter()
            .map(|c| {
                let mut s = format!("{} {}", c.name, c.data_type);
                if c.primary_key {
                    s.push_str(" PK");
                }
                if !c.nullable && !c.primary_key {
                    s.push_str(" NOT NULL");
                }
                s
            })
            .collect();

        let full_name = match &table.schema {
            Some(s) if !s.is_empty() => format!("{s}.{}", table.name),
            _ => table.name.clone(),
        };

        let kind_str = match table.kind {
            crate::model::TableKind::Table => "",
            crate::model::TableKind::View => " [VIEW]",
        };

        let row_est = match table.row_estimate {
            Some(rows) if rows >= 0 => format!(" (~{rows} rows)"),
            _ => String::new(),
        };

        out.push_str(&format!("- {full_name}{kind_str}{row_est}: ({})\n", col_text.join(", ")));
    }

    if omitted > 0 {
        out.push_str(&format!("-- ({omitted} more tables omitted)\n"));
    }

    if !foreign_keys.is_empty() {
        out.push_str("\nForeign Keys & Relationships:\n");
        for (fk_count, fk) in foreign_keys.iter().enumerate() {
            if fk_count >= 50 {
                out.push_str("-- (more foreign keys omitted)\n");
                break;
            }
            let from_tbl = match &fk.from_schema {
                Some(s) if !s.is_empty() => format!("{s}.{}", fk.from_table),
                _ => fk.from_table.clone(),
            };
            let to_tbl = match &fk.to_schema {
                Some(s) if !s.is_empty() => format!("{s}.{}", fk.to_table),
                _ => fk.to_table.clone(),
            };
            out.push_str(&format!(
                "- {from_tbl}({}) -> {to_tbl}({})\n",
                fk.from_columns.join(", "),
                fk.to_columns.join(", ")
            ));
        }
    }

    // If an active table is specified, provide its exact DDL if available
    if let Some(target) = current_table {
        let matched = tables_to_include.iter().find(|t| t.name.eq_ignore_ascii_case(target));
        if let Some(tbl) = matched {
            let t_ref = crate::model::TableRef {
                schema: tbl.schema.clone(),
                name: tbl.name.clone(),
            };
            if let Ok(Some(ddl)) = ctx.integration.ddl(&t_ref).await {
                out.push_str(&format!("\nActive Table DDL ({target}):\n```sql\n{}\n```\n", ddl.trim()));
            }
        }
    }

    Ok(out)
}

// WHY:   EXPLAIN prefixes only the first statement; a script would run its rest un-explained.
fn is_one_statement(sql: &str) -> bool {
    crate::guard::destructive::split_statements(sql).iter().filter(|s| !s.trim().trim_end_matches(';').trim().is_empty()).count() <= 1
}

pub async fn explain(ctx: &SessionCtx, req: &AiRequest<'_>, sql: &str, max_rows: usize) -> AppResult<PlanReport> {
    if !is_one_statement(sql) {
        return Err(AppError::invalid_input("Explain takes one statement at a time."));
    }
    let engine = ctx.connection.engine;
    let explain_sql = match engine.family() {
        Family::Postgres | Family::Duckdb => format!("EXPLAIN (FORMAT TEXT) {sql}"),
        Family::Sqlite | Family::Libsql | Family::ValTown | Family::CloudflareD1 => format!("EXPLAIN QUERY PLAN {sql}"),
        Family::Mysql | Family::Clickhouse | Family::Druid | Family::Snowflake | Family::Bigquery | Family::Surrealdb | Family::Orientdb | Family::Influxdb => format!("EXPLAIN {sql}"),
        Family::Oracle => format!("EXPLAIN PLAN FOR {sql}"),
        Family::Mssql => format!("SET SHOWPLAN_TEXT ON;\n{sql};\nSET SHOWPLAN_TEXT OFF;"),
        Family::Cassandra => format!("TRACING ON; {sql}"),
        Family::Neo4j | Family::Tigergraph => format!("EXPLAIN {sql}"),
        Family::Arangodb => format!("EXPLAIN {sql}"),
        _ => return Err(AppError::invalid_input("Execution plans are only available for SQL and Cypher engines.")),
    };
    let results = ctx.integration.execute(&explain_sql, max_rows).await?;
    let mut plan = String::new();
    for r in results {
        if let StatementResult::Rows { result } = r {
            for row in result.rows {
                let line: Vec<String> = row.iter().map(cell_text).collect();
                plan.push_str(&line.join("  "));
                plan.push('\n');
            }
        }
    }
    // The visual plan is a bonus on top of the text: an engine without a
    // structured EXPLAIN, or one that rejects it, still returns the text plan.
    let plan_tree = match ctx.integration.explain_tree(sql).await {
        Ok(tree) => tree,
        Err(err) => {
            log::debug!("structured plan unavailable: {err}");
            None
        }
    };
    let explanation = if req.settings.provider == AiProvider::None {
        None
    } else {
        let schema = schema_context(ctx, sql, None).await.unwrap_or_default();
        let system = format!(
            "You are an expert {} query tuning and performance specialist. Explain execution plans to developers in plain language.\n\
             Point out sequential scans, unindexed filters, missing indexes, costly nested loops, hash joins, or heavy sorts.\n\
             Suggest concrete, runnable improvements (e.g. exact CREATE INDEX statements referencing real columns from the schema).\n\
             Be concise; use short bullet points.\n\n\
             Database Schema:\n{}",
            engine_label(engine),
            schema
        );
        let prompt = format!("Query:\n{sql}\n\nExecution Plan:\n{plan}");
        let raw = complete(req, &system, &prompt, None).await?;
        Some(strip_think_blocks(&raw))
    };
    Ok(PlanReport { plan, explanation, plan_tree })
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Decimal(s) | Value::Text(s) | Value::Bytes(s) | Value::DateTime(s) | Value::Unsupported(s) => s.clone(),
        Value::Json(j) => j.to_string(),
    }
}

pub fn strip_think_blocks(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("</think>") {
            remaining = &remaining[start + end + "</think>".len()..];
        } else {
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result.trim().to_string()
}

pub fn extract_fence(text: &str) -> Option<String> {
    // 1. Search for fenced code blocks
    let mut start_idx = 0;
    while let Some(open_rel) = text[start_idx..].find("```") {
        let fence_start = start_idx + open_rel;
        let after_fence = &text[fence_start + 3..];
        let body_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
        let body = &after_fence[body_start..];
        if let Some(end) = body.find("```") {
            let code = body[..end].trim();
            if !code.is_empty() {
                return Some(code.to_string());
            }
            start_idx = fence_start + 3 + body_start + end + 3;
        } else {
            break;
        }
    }

    // 2. Fallback heuristic for raw statements if model omitted code fences
    let trimmed = text.trim();
    let upper = trimmed.to_uppercase();
    let sql_starters = [
        "SELECT ", "WITH ", "INSERT ", "UPDATE ", "DELETE ", "CREATE ", "ALTER ", "DROP ",
        "EXPLAIN ", "SHOW ", "DESCRIBE ", "SET ", "GET ", "HGET", "SCAN ", "KEYS ", "db.",
    ];
    for starter in sql_starters {
        if upper.starts_with(starter) {
            let code = if let Some(cutoff) = trimmed.find("\n\n") {
                trimmed[..cutoff].trim()
            } else {
                trimmed
            };
            if !code.is_empty() {
                return Some(code.to_string());
            }
        }
    }

    None
}

async fn complete(
    req: &AiRequest<'_>,
    system: &str,
    user: &str,
    conversation_history: Option<&[crate::commands::ai::ChatMessage]>,
) -> AppResult<String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(120)).build().map_err(AppError::internal)?;
    let model = req.settings.model.trim();
    if model.is_empty() {
        return Err(AppError::invalid_input("Set an AI model in Settings → AI."));
    }
    match req.settings.provider {
        AiProvider::None => Err(AppError::invalid_input("Choose an AI provider in Settings → AI first.")),
        AiProvider::Anthropic => {
            let key = req.api_key.ok_or_else(|| AppError::invalid_input("Add your Anthropic API key in Settings → AI."))?;
            let base = req.settings.base_url.clone().unwrap_or_else(|| "https://api.anthropic.com".to_string());
            let mut messages_json = Vec::new();
            if let Some(history) = conversation_history {
                for msg in history {
                    messages_json.push(serde_json::json!({
                        "role": if msg.role == "assistant" { "assistant" } else { "user" },
                        "content": msg.content
                    }));
                }
            }
            messages_json.push(serde_json::json!({
                "role": "user",
                "content": user
            }));
            let body = serde_json::json!({
                "model": model,
                "max_tokens": 4000,
                "system": system,
                "messages": messages_json,
                "fallbacks": "default"
            });
            let response = client
                .post(format!("{}/v1/messages", base.trim_end_matches('/')))
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .header("anthropic-beta", "server-side-fallback-2026-07-01")
                .json(&body)
                .send()
                .await?;
            let status = response.status();
            let json: serde_json::Value = response.json().await?;
            if !status.is_success() {
                let msg = json.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("request failed");
                return Err(AppError::driver(format!("Anthropic API {status}: {msg}")));
            }
            if json.get("stop_reason").and_then(|s| s.as_str()) == Some("refusal") {
                let why = json.pointer("/stop_details/explanation").and_then(|e| e.as_str()).unwrap_or("the request was declined");
                return Err(AppError::driver(format!("The model declined: {why}")));
            }
            let text = json
                .get("content")
                .and_then(|c| c.as_array())
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            Ok(text)
        }
        AiProvider::Openai | AiProvider::Openrouter => {
            let key = req.api_key.ok_or_else(|| AppError::invalid_input("Add your API key in Settings → AI."))?;
            let default_base = if req.settings.provider == AiProvider::Openai { "https://api.openai.com/v1" } else { "https://openrouter.ai/api/v1" };
            let base = req.settings.base_url.clone().unwrap_or_else(|| default_base.to_string());
            let mut messages_json = vec![
                serde_json::json!({"role": "system", "content": system})
            ];
            if let Some(history) = conversation_history {
                for msg in history {
                    messages_json.push(serde_json::json!({
                        "role": if msg.role == "assistant" { "assistant" } else { "user" },
                        "content": msg.content
                    }));
                }
            }
            messages_json.push(serde_json::json!({
                "role": "user",
                "content": user
            }));
            let body = serde_json::json!({
                "model": model,
                "messages": messages_json
            });
            let response = client
                .post(format!("{}/chat/completions", base.trim_end_matches('/')))
                .bearer_auth(key)
                .json(&body)
                .send()
                .await?;
            let status = response.status();
            let json: serde_json::Value = response.json().await?;
            if !status.is_success() {
                let msg = json.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("request failed");
                return Err(AppError::driver(format!("API {status}: {msg}")));
            }
            Ok(json.pointer("/choices/0/message/content").and_then(|c| c.as_str()).unwrap_or_default().to_string())
        }
        AiProvider::Google => {
            let key = req.api_key.ok_or_else(|| AppError::invalid_input("Add your Google API key in Settings → AI."))?;
            let base = req.settings.base_url.clone().unwrap_or_else(|| GOOGLE_BASE.to_string());
            let mut contents = Vec::new();
            if let Some(history) = conversation_history {
                for msg in history {
                    contents.push(serde_json::json!({
                        "role": if msg.role == "assistant" { "model" } else { "user" },
                        "parts": [{ "text": msg.content }]
                    }));
                }
            }
            contents.push(serde_json::json!({ "role": "user", "parts": [{ "text": user }] }));
            let body = serde_json::json!({
                "systemInstruction": { "parts": [{ "text": system }] },
                "contents": contents
            });
            let response = client
                .post(format!("{}/v1beta/models/{model}:generateContent", base.trim_end_matches('/')))
                .header("x-goog-api-key", key)
                .json(&body)
                .send()
                .await?;
            let status = response.status();
            let json: serde_json::Value = response.json().await?;
            if !status.is_success() {
                let msg = json.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("request failed");
                return Err(AppError::driver(format!("Gemini API {status}: {msg}")));
            }
            Ok(gemini_text(&json))
        }
        AiProvider::Ollama => {
            let base = req.settings.base_url.clone().unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
            let mut messages_json = vec![
                serde_json::json!({"role": "system", "content": system})
            ];
            if let Some(history) = conversation_history {
                for msg in history {
                    messages_json.push(serde_json::json!({
                        "role": if msg.role == "assistant" { "assistant" } else { "user" },
                        "content": msg.content
                    }));
                }
            }
            messages_json.push(serde_json::json!({
                "role": "user",
                "content": user
            }));
            let body = serde_json::json!({
                "model": model,
                "stream": false,
                "messages": messages_json
            });
            let response = client.post(format!("{}/api/chat", base.trim_end_matches('/'))).json(&body).send().await?;
            let status = response.status();
            let json: serde_json::Value = response.json().await?;
            if !status.is_success() {
                return Err(AppError::driver(format!("Ollama {status}: {}", json.get("error").and_then(|e| e.as_str()).unwrap_or("request failed"))));
            }
            Ok(json.pointer("/message/content").and_then(|c| c.as_str()).unwrap_or_default().to_string())
        }
    }
}

/// Text parts of a Gemini `generateContent` reply, joined.
fn gemini_text(json: &serde_json::Value) -> String {
    json.pointer("/candidates/0/content/parts")
        .and_then(|parts| parts.as_array())
        .map(|parts| parts.iter().filter_map(|part| part.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(""))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Model discovery
// ---------------------------------------------------------------------------

const OPENAI_BASE: &str = "https://api.openai.com/v1";
const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
const ANTHROPIC_BASE: &str = "https://api.anthropic.com";
const GOOGLE_BASE: &str = "https://generativelanguage.googleapis.com";
const OLLAMA_BASE: &str = "http://127.0.0.1:11434";

// WHAT:  Ask a provider which chat models it offers.
// WHY:   The model picker and Settings → AI list real model ids instead of
//        asking people to remember them, and the same request doubles as the
//        "Test" button: it needs the key and the endpoint to be right.
// HOW:   One GET per provider (OpenAI-compatible `/models`, Anthropic's models
//        API, Gemini's `v1beta/models`, Ollama's `/api/tags`). The parsers are
//        pure so each response shape is pinned by a test. `timeout` bounds the
//        whole request: 5 s for Test, longer for a plain refresh.
// WHERE: src-tauri/src/services/ai_profiles.rs (cache + orchestration)
pub async fn fetch_models(
    provider: AiProvider,
    base_url: Option<&str>,
    api_key: Option<&str>,
    timeout: Duration,
) -> AppResult<Vec<AiModel>> {
    let client = reqwest::Client::builder().timeout(timeout).build().map_err(AppError::internal)?;
    let base = base_url.map(str::trim).filter(|b| !b.is_empty()).map(|b| b.trim_end_matches('/').to_string());
    let need_key = |name: &str| -> AppResult<String> {
        api_key
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .ok_or_else(|| AppError::invalid_input(format!("Add your {name} API key first.")))
    };

    let request = match provider {
        AiProvider::None => return Err(AppError::invalid_input("Choose a provider first.")),
        AiProvider::Openai => {
            let key = need_key("OpenAI")?;
            client.get(format!("{}/models", base.as_deref().unwrap_or(OPENAI_BASE))).bearer_auth(key)
        }
        AiProvider::Openrouter => {
            let key = need_key("OpenRouter")?;
            client.get(format!("{}/models", base.as_deref().unwrap_or(OPENROUTER_BASE))).bearer_auth(key)
        }
        AiProvider::Anthropic => {
            let key = need_key("Anthropic")?;
            client
                .get(format!("{}/v1/models?limit=1000", base.as_deref().unwrap_or(ANTHROPIC_BASE)))
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
        }
        AiProvider::Google => {
            let key = need_key("Google")?;
            client
                .get(format!("{}/v1beta/models?pageSize=1000", base.as_deref().unwrap_or(GOOGLE_BASE)))
                .header("x-goog-api-key", key)
        }
        AiProvider::Ollama => client.get(format!("{}/api/tags", base.as_deref().unwrap_or(OLLAMA_BASE))),
    };

    let response = request.send().await.map_err(|err| {
        if err.is_timeout() {
            AppError::timeout(format!("The provider did not answer within {} s.", timeout.as_secs()))
        } else if err.is_connect() {
            AppError::driver(format!("Could not reach the provider: {err}"))
        } else {
            AppError::driver(format!("Model request failed: {err}"))
        }
    })?;
    let status = response.status();
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|err| AppError::driver(format!("The provider answered with something that is not JSON ({status}): {err}")))?;
    if !status.is_success() {
        let message = payload
            .pointer("/error/message")
            .or_else(|| payload.get("error").filter(|e| e.is_string()))
            .or_else(|| payload.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("request failed");
        return Err(if status.as_u16() == 401 || status.as_u16() == 403 {
            AppError::invalid_input(format!("The provider rejected the key ({status}): {message}"))
        } else {
            AppError::driver(format!("{status}: {message}"))
        });
    }

    // A custom OpenAI base URL is a proxy or a local server, whose model ids are
    // not OpenAI's: filtering them by OpenAI's naming would hide all of them.
    let filter_openai = base.is_none();
    Ok(parse_models(provider, &payload, filter_openai))
}

// WHAT:  A provider's response body as a list of chat models.
pub fn parse_models(provider: AiProvider, payload: &serde_json::Value, filter_openai: bool) -> Vec<AiModel> {
    let items = |key: &str| payload.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let text = |item: &serde_json::Value, key: &str| item.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let mut models: Vec<AiModel> = match provider {
        AiProvider::None => Vec::new(),
        AiProvider::Openai => {
            let mut data = items("data");
            // Newest first, the way the provider's own picker lists them.
            data.sort_by_key(|m| std::cmp::Reverse(m.get("created").and_then(|c| c.as_i64()).unwrap_or(0)));
            data.iter()
                .filter_map(|m| text(m, "id"))
                .filter(|id| !filter_openai || is_openai_chat_model(id))
                .map(|id| AiModel { label: id.clone(), id })
                .collect()
        }
        AiProvider::Openrouter => items("data")
            .iter()
            .filter_map(|m| text(m, "id").map(|id| AiModel { label: text(m, "name").unwrap_or_else(|| id.clone()), id }))
            .collect(),
        AiProvider::Anthropic => items("data")
            .iter()
            .filter_map(|m| text(m, "id").map(|id| AiModel { label: text(m, "display_name").unwrap_or_else(|| id.clone()), id }))
            .collect(),
        AiProvider::Google => items("models")
            .iter()
            .filter(|m| is_google_chat_model(m))
            .filter_map(|m| {
                let id = text(m, "name")?.trim_start_matches("models/").to_string();
                Some(AiModel { label: text(m, "displayName").unwrap_or_else(|| id.clone()), id })
            })
            .collect(),
        AiProvider::Ollama => items("models")
            .iter()
            .filter_map(|m| text(m, "name").or_else(|| text(m, "model")))
            .map(|id| AiModel { label: id.clone(), id })
            .collect(),
    };
    let mut seen = std::collections::HashSet::new();
    models.retain(|m| !m.id.is_empty() && seen.insert(m.id.clone()));
    models
}

// WHAT:  OpenAI's `/models` also lists embeddings, speech, image and moderation
//        models; only the ones a chat can use belong in the picker.
fn is_openai_chat_model(id: &str) -> bool {
    let base = if let Some(rest) = id.strip_prefix("ft:") { rest.split(':').next().unwrap_or(rest) } else { id };
    let lower = base.to_lowercase();
    let chatty = lower.starts_with("gpt-")
        || lower.starts_with("chatgpt-")
        || ["o1", "o3", "o4"].iter().any(|p| lower == *p || lower.starts_with(&format!("{p}-")));
    chatty
        && !["audio", "embedding", "image", "moderation", "realtime", "search", "transcri", "tts"]
            .iter()
            .any(|bad| lower.contains(bad))
}

fn is_google_chat_model(model: &serde_json::Value) -> bool {
    let id = model.get("name").and_then(|n| n.as_str()).unwrap_or("").trim_start_matches("models/").to_lowercase();
    let can_generate = model
        .get("supportedGenerationMethods")
        .and_then(|m| m.as_array())
        .is_some_and(|methods| methods.iter().any(|m| m.as_str() == Some("generateContent")));
    id.starts_with("gemini-")
        && can_generate
        && !["embedding", "image", "imagen", "live", "native-audio", "tts", "veo"].iter().any(|bad| id.contains(bad))
}

// `From<reqwest::Error>` for AppError lives in integrations/clickhouse.rs (one impl per crate).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explain_takes_one_statement() {
        assert!(is_one_statement("SELECT 1"));
        assert!(is_one_statement("SELECT 1;"));
        assert!(is_one_statement("SELECT 'a;b' FROM t"), "a semicolon inside a literal is not a separator");
        assert!(!is_one_statement("SELECT 1; DELETE FROM t"));
    }

    #[test]
    fn fence_extraction() {
        assert_eq!(extract_fence("Here:\n```sql\nSELECT 1;\n```\nDone"), Some("SELECT 1;".into()));
        assert_eq!(extract_fence("```\nGET k\n```"), Some("GET k".into()));
        assert_eq!(extract_fence("```redis\nHGETALL user:1\n```"), Some("HGETALL user:1".into()));
        assert_eq!(extract_fence("```json\n{\"find\": \"users\"}\n```"), Some("{\"find\": \"users\"}".into()));
    }

    #[test]
    fn raw_sql_fallback() {
        assert_eq!(
            extract_fence("SELECT * FROM users WHERE active = true;\n\nThis selects all active users."),
            Some("SELECT * FROM users WHERE active = true;".into())
        );
        assert_eq!(extract_fence("no code here, just a helpful answer"), None);
    }

    #[test]
    fn think_blocks_removal() {
        let text = "<think>\nLet me analyze the tables...\n</think>\nHere is the query:\n```sql\nSELECT * FROM orders;\n```";
        assert_eq!(
            strip_think_blocks(text),
            "Here is the query:\n```sql\nSELECT * FROM orders;\n```"
        );
        let unclosed = "<think>still thinking";
        assert_eq!(strip_think_blocks(unclosed), "");
    }

    #[test]
    fn openai_models_keep_chat_models_newest_first() {
        let payload = serde_json::json!({ "data": [
            { "id": "text-embedding-3-small", "created": 5 },
            { "id": "gpt-4o", "created": 10 },
            { "id": "whisper-1", "created": 3 },
            { "id": "o3-mini", "created": 30 },
            { "id": "gpt-4o-audio-preview", "created": 40 },
            { "id": "gpt-4o-realtime-preview", "created": 41 },
            { "id": "gpt-4.1", "created": 50 },
            { "id": "dall-e-3", "created": 2 },
            { "id": "ft:gpt-4o-mini:acme::abc", "created": 20 },
            { "id": "omni-moderation-latest", "created": 1 },
        ]});
        let ids: Vec<String> = parse_models(AiProvider::Openai, &payload, true).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["gpt-4.1", "o3-mini", "ft:gpt-4o-mini:acme::abc", "gpt-4o"]);
    }

    #[test]
    fn a_custom_openai_endpoint_is_not_filtered_by_openai_naming() {
        let payload = serde_json::json!({ "data": [{ "id": "llama-3.1-8b-instruct" }, { "id": "qwen2.5-coder" }] });
        let ids: Vec<String> = parse_models(AiProvider::Openai, &payload, false).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["llama-3.1-8b-instruct", "qwen2.5-coder"]);
    }

    #[test]
    fn anthropic_models_use_display_names() {
        let payload = serde_json::json!({ "data": [{ "id": "claude-opus-5", "display_name": "Claude Opus 5" }, { "id": "claude-haiku" }] });
        let models = parse_models(AiProvider::Anthropic, &payload, true);
        assert_eq!(models[0], AiModel { id: "claude-opus-5".into(), label: "Claude Opus 5".into() });
        assert_eq!(models[1].label, "claude-haiku");
    }

    #[test]
    fn gemini_models_need_generate_content_and_drop_media_models() {
        let payload = serde_json::json!({ "models": [
            { "name": "models/gemini-2.5-pro", "displayName": "Gemini 2.5 Pro", "supportedGenerationMethods": ["generateContent", "countTokens"] },
            { "name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"] },
            { "name": "models/gemini-embedding-001", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-2.0-flash-live", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-2.5-flash", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-no-methods" },
        ]});
        let models = parse_models(AiProvider::Google, &payload, true);
        assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["gemini-2.5-pro", "gemini-2.5-flash"]);
        assert_eq!(models[0].label, "Gemini 2.5 Pro");
    }

    #[test]
    fn ollama_and_openrouter_models() {
        let tags = serde_json::json!({ "models": [{ "name": "llama3.1:latest" }, { "model": "qwen2.5:7b" }, { "name": "llama3.1:latest" }] });
        let ids: Vec<String> = parse_models(AiProvider::Ollama, &tags, true).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["llama3.1:latest", "qwen2.5:7b"], "duplicates collapse");
        let router = serde_json::json!({ "data": [{ "id": "anthropic/claude-opus-5", "name": "Claude Opus 5" }, { "id": "" }] });
        let models = parse_models(AiProvider::Openrouter, &router, true);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].label, "Claude Opus 5");
    }

    #[test]
    fn gemini_reply_text_is_joined() {
        let reply = serde_json::json!({ "candidates": [{ "content": { "parts": [{ "text": "SELECT " }, { "text": "1;" }] } }] });
        assert_eq!(gemini_text(&reply), "SELECT 1;");
        assert_eq!(gemini_text(&serde_json::json!({})), "");
    }
}
