// SOT: agent-tools, tool-registry, progressive-schema-disclosure, agent-artifact-tools, tool-permission-gate

use crate::error::{AppError, AppResult};
use crate::guard::destructive::{StatementKind, classify};
use crate::guard::{self, SessionCtx};
use crate::integrations;
use crate::model::{
    AgentArtifact, AgentAutonomy, ObjectKind, ObjectRef, PageQuery, PermissionDecision, PermissionRequest,
    Family, QueryOutcome, StatementIntent, StatementResult, TableRef, Tool, UiBlock, UiGraphEdge,
    UiGraphNode, UiStat, UiTone, Value, WidgetKind,
};
use crate::services;
use crate::services::agent::provider::ToolSpec;
use crate::services::agent::skills;
use serde_json::{Value as Json, json};

// WHAT:  Every tool the model may call, and the dispatcher that runs one.
// WHY:   The old assistant pasted up to 60 tables with all their columns into
//        every single request. Most of it was never read, it blew the budget on
//        large databases, and it still missed the table the user meant. Tools
//        invert that: the model is told only what exists in outline and pulls
//        detail for the handful of objects it actually needs.
// HOW:   Each tool maps onto a service function, so the guard, the row caps and
//        the read-only lock apply exactly as they do for the UI. Nothing here
//        reaches an integration or the store directly.
// WHERE: src-tauri/src/services/agent/mod.rs (the loop), CLAUDE.md (layer table)

/// Rows a single sample or query tool call may return. Small on purpose: the
/// model needs a shape and a few values, not a data dump billed as input tokens.
const SAMPLE_ROWS: usize = 20;
const QUERY_ROWS: usize = 200;
/// Cell text longer than this is elided; a base64 blob teaches the model nothing.
const MAX_CELL: usize = 200;
const MAX_LIST: usize = 300;

/// What one tool call produced.
pub struct ToolRun {
    /// Fed back to the model.
    pub content: String,
    pub is_error: bool,
    /// One-line headline for the timeline row.
    pub summary: String,
    /// A view the UI should draw, when the tool asked for one.
    pub artifact: Option<AgentArtifact>,
    /// A statement worth offering to the editor.
    pub sql: Option<String>,
}

impl ToolRun {
    fn ok(summary: impl Into<String>, content: impl Into<String>) -> ToolRun {
        ToolRun {
            content: content.into(),
            is_error: false,
            summary: summary.into(),
            artifact: None,
            sql: None,
        }
    }

    /// Public constructor for tools that live in a sibling module (skills).
    pub fn from_text(summary: impl Into<String>, content: impl Into<String>) -> ToolRun {
        ToolRun::ok(summary, content)
    }

    fn failed(message: impl Into<String>) -> ToolRun {
        let text = message.into();
        ToolRun {
            summary: "failed".to_string(),
            content: text,
            is_error: true,
            artifact: None,
            sql: None,
        }
    }

    fn with_artifact(mut self, artifact: AgentArtifact) -> ToolRun {
        self.artifact = Some(artifact);
        self
    }

    fn with_sql(mut self, sql: impl Into<String>) -> ToolRun {
        self.sql = Some(sql.into());
        self
    }
}

// WHAT:  The permission hook. The loop implements it; the tools call it before
//        anything that writes.
// WHY:   Approval has to suspend the run and wait for a human, which no
//        provider-side loop can express. Keeping it a trait means the gate is
//        testable without a UI and cannot be bypassed by adding a tool.
// `Send` because a tauri command's future must be `Send`, and this trait object
// is held across every await in the run loop.
pub trait PermissionGate: Send {
    fn ask(
        &mut self,
        request: PermissionRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = PermissionDecision> + Send + '_>>;
}

/// Everything a dispatch needs beyond the arguments.
pub struct ToolCtx<'a> {
    pub session: &'a SessionCtx,
    pub autonomy: AgentAutonomy,
    /// The timeline row this call belongs to; a permission prompt quotes it.
    pub call_id: String,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

// WHAT:  The tools this connection actually supports.
// WHY:   Advertising `list_schemas` to Redis or `explain_query` to S3 invites
//        the model to call something that can only fail. The family profile
//        already states what the adapter can do, so it decides the tool list.
pub fn definitions(ctx: &SessionCtx) -> Vec<ToolSpec> {
    let engine = ctx.connection.engine;
    let profile = integrations::profile(engine.family());
    let caps = profile.capabilities;
    let language = language_of(engine.family());
    let mut tools = Vec::new();

    tools.push(spec(
        "list_databases",
        "List the databases this connection can see. Call this only when the user asks about \
         databases other than the one in use, or you need to confirm which database you are in.",
        json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    ));

    if caps.namespaces {
        tools.push(spec(
            "list_schemas",
            "List schemas (namespaces / keyspaces) with how many tables each holds. \
             Call this first when you do not yet know where the user's tables live.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ));
    }

    tools.push(spec(
        "list_tables",
        "List tables and views by name, with row estimates. Returns names only — no columns. \
         Call this to find out what exists; then call describe_table for the few you need.",
        json!({
            "type": "object",
            "properties": {
                "schema": { "type": "string", "description": "Restrict to one schema. Omit for every schema." },
                "search": { "type": "string", "description": "Case-insensitive substring the table name must contain." }
            },
            "additionalProperties": false
        }),
    ));

    tools.push(spec(
        "describe_table",
        "Full detail for ONE table: every column with type, nullability and primary key, plus the \
         foreign keys touching it and its CREATE statement. This is the tool to reach for before \
         writing a query against a table.",
        json!({
            "type": "object",
            "properties": {
                "table": { "type": "string", "description": "Table name. Qualify as schema.table when ambiguous." },
                "schema": { "type": "string", "description": "Schema, if not already in `table`." }
            },
            "required": ["table"],
            "additionalProperties": false
        }),
    ));

    tools.push(spec(
        "search_schema",
        "Find tables and columns whose names contain a keyword, across the whole database. \
         Use this when the user names a concept ('orders', 'email') and you do not know which \
         table holds it — it is far cheaper than listing and describing everything.",
        json!({
            "type": "object",
            "properties": { "keyword": { "type": "string" } },
            "required": ["keyword"],
            "additionalProperties": false
        }),
    ));

    tools.push(spec(
        "sample_rows",
        &format!(
            "Read up to {SAMPLE_ROWS} rows from one table, so you can see real values before \
             writing a query. Always safe: it cannot write, and it is paged and capped."
        ),
        json!({
            "type": "object",
            "properties": {
                "table": { "type": "string" },
                "schema": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": SAMPLE_ROWS }
            },
            "required": ["table"],
            "additionalProperties": false
        }),
    ));

    tools.push(spec(
        "run_query",
        &format!(
            "Execute a {language} statement and return the rows. Use it to answer questions with \
             real data rather than guessing. Reads run immediately; anything that writes or \
             destroys data is shown to the user for approval first, so prefer a read unless the \
             user asked you to change something."
        ),
        json!({
            "type": "object",
            "properties": {
                "statement": { "type": "string", "description": format!("A single {language} statement.") },
                "purpose": { "type": "string", "description": "One short line on what this answers — shown to the user when approval is needed." }
            },
            "required": ["statement"],
            "additionalProperties": false
        }),
    ));

    if caps.sql {
        tools.push(spec(
            "explain_query",
            "Return the execution plan for a statement without running it. Use it when the user \
             asks why something is slow, or to justify an index.",
            json!({
                "type": "object",
                "properties": { "statement": { "type": "string" } },
                "required": ["statement"],
                "additionalProperties": false
            }),
        ));
    }

    if !profile.object_kinds.is_empty() {
        let kinds: Vec<String> = profile.object_kinds.iter().map(|k| kind_name(*k)).collect();
        tools.push(spec(
            "list_objects",
            "List database objects of one kind (views, indexes, functions, users, topics…). \
             Use it for questions about the database's structure rather than its data.",
            json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": kinds },
                    "parent": { "type": "string", "description": "Owning schema, for kinds that live in one." }
                },
                "required": ["kind"],
                "additionalProperties": false
            }),
        ));
        tools.push(spec(
            "describe_object",
            "Definition and properties of one object listed by list_objects.",
            json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string" },
                    "name": { "type": "string" },
                    "parent": { "type": "string" }
                },
                "required": ["kind", "name"],
                "additionalProperties": false
            }),
        ));
    }

    tools.push(spec(
        "server_stats",
        "Live server metrics: connections, memory, cache hit rates, replication. \
         For questions about the health of the server rather than its data.",
        json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    ));

    // --- Tools that draw, rather than read -------------------------------
    tools.push(spec(
        "render_chart",
        &format!(
            "Run a {language} query and draw the result as a chart in the conversation. Use this \
             whenever the answer is a trend, a comparison or a breakdown — a chart the user can \
             read beats a table of numbers. The query's first non-numeric column becomes the \
             x axis and every numeric column becomes a series, so shape it that way."
        ),
        json!({
            "type": "object",
            "properties": {
                "statement": { "type": "string" },
                "chart": {
                    "type": "string",
                    "enum": ["line", "area", "bar", "pie"],
                    "description": "bar for comparing categories, line or area for change over time, pie for parts of a whole."
                },
                "title": { "type": "string" },
                "xLabel": { "type": "string" },
                "yLabel": { "type": "string" }
            },
            "required": ["statement", "chart", "title"],
            "additionalProperties": false
        }),
    ));

    tools.push(spec(
        "render_table",
        "Run a query and show the result as a sortable grid in the conversation. Use it when the \
         user wants to see the rows themselves rather than a summary of them.",
        json!({
            "type": "object",
            "properties": {
                "statement": { "type": "string" },
                "title": { "type": "string" }
            },
            "required": ["statement", "title"],
            "additionalProperties": false
        }),
    ));

    // Relationship graphs work on any engine that reports foreign keys; a native
    // graph query only makes sense where the engine has one.
    let graph_query = profile.tools.contains(&Tool::GraphView);
    let graph_modes: Vec<&str> =
        if graph_query { vec!["relationships", "query"] } else { vec!["relationships"] };
    tools.push(spec(
        "render_graph",
        &format!(
            "Draw an interactive node-and-edge graph in the conversation. Use \"relationships\" to \
             map how tables connect through their foreign keys — the fastest way to answer \"how \
             does this database fit together\".{}",
            if graph_query {
                format!(
                    " Use \"query\" to draw the nodes and relationships returned by a {language} \
                     statement."
                )
            } else {
                String::new()
            }
        ),
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": graph_modes,
                    "description": "relationships = build from foreign keys; query = draw what a statement returns."
                },
                "title": { "type": "string" },
                "tables": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "For relationships mode: restrict to these tables. Omit for the whole database."
                },
                "statement": { "type": "string", "description": "For query mode." }
            },
            "required": ["mode", "title"],
            "additionalProperties": false
        }),
    ));

    tools.push(spec(
        "render_ui",
        &format!(
            "Compose a small view out of the app's own components and show it in the conversation: \
             a row of headline figures, a chart, a table, a graph, notes. Reach for this when the \
             answer deserves a layout rather than a paragraph — a summary of a table, a health \
             check, a before/after comparison, a report. Blocks render top to bottom in the order \
             you give them, and any block with a `sql` field is executed for you, so write \
             {language} that returns exactly the shape that block needs."
        ),
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string" },
                "blocks": {
                    "type": "array",
                    "description": "Two to six blocks reads best. Lead with `stats`, then the detail.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "block": {
                                "type": "string",
                                "enum": ["heading", "text", "stats", "chart", "table", "graph", "facts", "callout", "divider"]
                            },
                            "text": { "type": "string", "description": "heading." },
                            "markdown": { "type": "string", "description": "text: prose, markdown allowed." },
                            "title": { "type": "string" },
                            "sql": { "type": "string", "description": "chart / table: the statement to run." },
                            "chart": { "type": "string", "enum": ["line", "area", "bar", "pie"] },
                            "xLabel": { "type": "string" },
                            "yLabel": { "type": "string" },
                            "stats": {
                                "type": "array",
                                "description": "stats / facts: label and value pairs.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string" },
                                        "value": { "type": "string" },
                                        "hint": { "type": "string" },
                                        "trend": { "type": "number", "description": "Percentage change." }
                                    },
                                    "required": ["label", "value"],
                                    "additionalProperties": false
                                }
                            },
                            "mode": { "type": "string", "enum": ["relationships", "query"], "description": "graph." },
                            "tables": { "type": "array", "items": { "type": "string" } },
                            "tone": { "type": "string", "enum": ["info", "success", "warning", "danger"], "description": "callout." },
                            "body": { "type": "string", "description": "callout." }
                        },
                        "required": ["block"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["title", "blocks"],
            "additionalProperties": false
        }),
    ));

    tools.push(spec(
        "list_skills",
        "List the task guides available for this database. Each is a short playbook (auditing a \
         schema, tuning a slow query, profiling data quality). Load one before starting that kind \
         of task instead of improvising.",
        json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    ));

    tools.push(spec(
        "load_skill",
        "Read one task guide in full, by id, from list_skills.",
        json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"],
            "additionalProperties": false
        }),
    ));

    tools
}

// WHAT:  The serde wire name of an object kind, which is what the UI and the
//        tool schema both use — so the model names a kind the way the app does.
fn kind_name(kind: ObjectKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

// WHAT:  What to call the language this engine speaks, in prose aimed at the model.
// WHY:   Telling a Neo4j user's agent to "write SQL" is how you get invalid Cypher.
fn language_of(family: Family) -> &'static str {
    match family {
        Family::Neo4j | Family::Tigergraph => "Cypher",
        Family::Arangodb => "AQL",
        Family::Mongodb => "MongoDB query",
        Family::Couchdb => "Mango selector",
        Family::Redis | Family::Memcached => "Redis command",
        Family::Rocksdb => "RocksDB command",
        Family::Elasticsearch => "Query DSL",
        Family::Meilisearch | Family::Typesense => "search request",
        Family::Prometheus => "PromQL",
        Family::Influxdb => "InfluxQL",
        Family::Sparql => "SPARQL",
        Family::Basex | Family::Existdb => "XQuery",
        Family::Cassandra => "CQL",
        Family::Surrealdb => "SurrealQL",
        Family::Kafka | Family::Rabbitmq => "consume request",
        Family::Convex => "function call",
        Family::S3 => "S3 command",
        Family::Objectdb => "JPQL",
        Family::Qldb | Family::Dynamodb => "PartiQL",
        Family::Hbase => "HBase command",
        Family::Qdrant | Family::Milvus | Family::Weaviate | Family::Pinecone | Family::Chroma => {
            "vector search request"
        }
        Family::Firestore => "structured query",
        _ => "SQL",
    }
}

fn spec(name: &str, description: &str, parameters: Json) -> ToolSpec {
    crate::services::agent::provider::tool(name, description, parameters)
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// A short human title for the timeline, derived before the tool runs.
pub fn title_for(name: &str, args: &Json) -> String {
    let s = |key: &str| args.get(key).and_then(Json::as_str).unwrap_or("").to_string();
    match name {
        "list_databases" => "List databases".to_string(),
        "list_schemas" => "List schemas".to_string(),
        "list_tables" => match (s("schema").as_str(), s("search").as_str()) {
            ("", "") => "List tables".to_string(),
            (schema, "") => format!("List tables in {schema}"),
            ("", search) => format!("Find tables matching \"{search}\""),
            (schema, search) => format!("Find tables matching \"{search}\" in {schema}"),
        },
        "describe_table" => format!("Describe {}", s("table")),
        "search_schema" => format!("Search schema for \"{}\"", s("keyword")),
        "sample_rows" => format!("Sample rows from {}", s("table")),
        "run_query" => {
            let purpose = s("purpose");
            if purpose.is_empty() { "Run query".to_string() } else { purpose }
        }
        "explain_query" => "Explain query plan".to_string(),
        "list_objects" => format!("List {}", s("kind")),
        "describe_object" => format!("Describe {} {}", s("kind"), s("name")),
        "server_stats" => "Read server stats".to_string(),
        "render_chart" => format!("Chart: {}", s("title")),
        "render_table" => format!("Table: {}", s("title")),
        "render_graph" => format!("Graph: {}", s("title")),
        "render_ui" => format!("View: {}", s("title")),
        "list_skills" => "List task guides".to_string(),
        "load_skill" => format!("Load guide \"{}\"", s("id")),
        other => other.to_string(),
    }
}

// WHAT:  Run one tool call.
// WHY:   Single entry point so the guard, the caps and the permission gate
//        cannot be skipped by a tool that forgets to call them.
pub async fn dispatch(
    ctx: &ToolCtx<'_>,
    name: &str,
    args: &Json,
    gate: &mut dyn PermissionGate,
) -> ToolRun {
    match run(ctx, name, args, gate).await {
        Ok(result) => result,
        // A failed tool is not a failed run: the model is told what went wrong
        // and usually recovers by trying a different call.
        Err(err) => ToolRun::failed(err.message().to_string()),
    }
}

async fn run(ctx: &ToolCtx<'_>, name: &str, args: &Json, gate: &mut dyn PermissionGate) -> AppResult<ToolRun> {
    match name {
        "list_databases" => list_databases(ctx).await,
        "list_schemas" => list_schemas(ctx).await,
        "list_tables" => list_tables(ctx, args).await,
        "describe_table" => describe_table(ctx, args).await,
        "search_schema" => search_schema(ctx, args).await,
        "sample_rows" => sample_rows(ctx, args).await,
        "run_query" => run_query(ctx, args, gate).await,
        "explain_query" => explain_query(ctx, args).await,
        "list_objects" => list_objects(ctx, args).await,
        "describe_object" => describe_object(ctx, args).await,
        "server_stats" => server_stats(ctx).await,
        "render_chart" => render_chart(ctx, args, gate).await,
        "render_table" => render_table(ctx, args, gate).await,
        "render_graph" => render_graph(ctx, args, gate).await,
        "render_ui" => render_ui(ctx, args, gate).await,
        "list_skills" => Ok(skills::list_run(ctx.session)),
        "load_skill" => skills::load_run(ctx.session, arg_str(args, "id")?),
        other => Err(AppError::invalid_input(format!("Unknown tool \"{other}\"."))),
    }
}

fn arg_str<'a>(args: &'a Json, key: &str) -> AppResult<&'a str> {
    args.get(key)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::invalid_input(format!("The tool call is missing \"{key}\".")))
}

// ---------------------------------------------------------------------------
// Read tools
// ---------------------------------------------------------------------------

async fn list_databases(ctx: &ToolCtx<'_>) -> AppResult<ToolRun> {
    let names = ctx.session.integration.databases().await.unwrap_or_default();
    let current = ctx
        .session
        .integration
        .current_database()
        .or_else(|| ctx.session.connection.database.clone());
    let mut out = String::new();
    if let Some(db) = &current {
        out.push_str(&format!("Currently attached to: {db}\n"));
    }
    if names.is_empty() {
        out.push_str("This engine exposes no other databases.");
    } else {
        out.push_str("Databases:\n");
        for name in names.iter().take(MAX_LIST) {
            out.push_str(&format!("- {name}\n"));
        }
    }
    Ok(ToolRun::ok(format!("{} databases", names.len()), out))
}

async fn list_schemas(ctx: &ToolCtx<'_>) -> AppResult<ToolRun> {
    let catalog = services::schema::catalog(ctx.session).await?;
    let mut out = String::from("Schemas (name — table count):\n");
    for schema in &catalog.schemas {
        out.push_str(&format!("- {} — {} tables\n", schema.name, schema.tables.len()));
    }
    Ok(ToolRun::ok(format!("{} schemas", catalog.schemas.len()), out))
}

async fn list_tables(ctx: &ToolCtx<'_>, args: &Json) -> AppResult<ToolRun> {
    let catalog = services::schema::catalog(ctx.session).await?;
    let want_schema = args.get("schema").and_then(Json::as_str).map(str::to_lowercase);
    let search = args.get("search").and_then(Json::as_str).map(str::to_lowercase);

    let mut lines = Vec::new();
    let mut count = 0usize;
    for schema in &catalog.schemas {
        if let Some(target) = &want_schema {
            if !schema.name.eq_ignore_ascii_case(target) {
                continue;
            }
        }
        for table in &schema.tables {
            if let Some(needle) = &search {
                if !table.name.to_lowercase().contains(needle) {
                    continue;
                }
            }
            count += 1;
            if lines.len() >= MAX_LIST {
                continue;
            }
            let kind = if matches!(table.kind, crate::model::TableKind::View) { " [view]" } else { "" };
            let rows = match table.row_estimate {
                Some(n) if n >= 0 => format!(" ~{n} rows"),
                _ => String::new(),
            };
            lines.push(format!("- {}{kind}{rows}", qualified(&table.schema, &table.name)));
        }
    }

    let mut out = if lines.is_empty() {
        "No tables matched.".to_string()
    } else {
        format!("Tables ({count}):\n{}\n", lines.join("\n"))
    };
    if count > lines.len() {
        out.push_str(&format!("… {} more not shown; narrow with `search`.\n", count - lines.len()));
    }
    out.push_str("\nCall describe_table for the columns of any table above.");
    Ok(ToolRun::ok(format!("{count} tables"), out))
}

async fn describe_table(ctx: &ToolCtx<'_>, args: &Json) -> AppResult<ToolRun> {
    let table = resolve_table(ctx, args).await?;
    let columns = services::schema::columns(ctx.session, &table).await?;
    if columns.is_empty() {
        return Err(AppError::not_found(format!(
            "\"{}\" has no columns, or does not exist.",
            qualified(&table.schema, &table.name)
        )));
    }

    let mut out = format!("Table {}\n\nColumns:\n", qualified(&table.schema, &table.name));
    for column in &columns {
        let mut flags = Vec::new();
        if column.primary_key {
            flags.push("PK");
        }
        if !column.nullable {
            flags.push("NOT NULL");
        }
        let suffix = if flags.is_empty() { String::new() } else { format!(" [{}]", flags.join(", ")) };
        out.push_str(&format!("- {} {}{suffix}\n", column.name, column.data_type));
    }

    let fks = services::schema::foreign_keys(ctx.session).await.unwrap_or_default();
    let related: Vec<&crate::model::ForeignKey> = fks
        .iter()
        .filter(|fk| {
            fk.from_table.eq_ignore_ascii_case(&table.name) || fk.to_table.eq_ignore_ascii_case(&table.name)
        })
        .take(40)
        .collect();
    if !related.is_empty() {
        out.push_str("\nForeign keys:\n");
        for fk in related {
            out.push_str(&format!(
                "- {}({}) -> {}({})\n",
                qualified(&fk.from_schema, &fk.from_table),
                fk.from_columns.join(", "),
                qualified(&fk.to_schema, &fk.to_table),
                fk.to_columns.join(", ")
            ));
        }
    }

    if let Ok(Some(ddl)) = services::schema::ddl(ctx.session, &table).await {
        out.push_str(&format!("\nDefinition:\n{}\n", ddl.trim()));
    }

    Ok(ToolRun::ok(format!("{} columns", columns.len()), out))
}

async fn search_schema(ctx: &ToolCtx<'_>, args: &Json) -> AppResult<ToolRun> {
    let keyword = arg_str(args, "keyword")?.to_lowercase();
    let catalog = services::schema::catalog(ctx.session).await?;

    let mut table_hits = Vec::new();
    let mut column_hits = Vec::new();
    for schema in &catalog.schemas {
        for table in &schema.tables {
            let name = qualified(&table.schema, &table.name);
            if table.name.to_lowercase().contains(&keyword) {
                table_hits.push(name.clone());
            }
            // Columns cost a round trip each, so only look inside tables while
            // the answer is still small enough to be worth it.
            if column_hits.len() < 60 {
                let reference = TableRef { schema: table.schema.clone(), name: table.name.clone() };
                if let Ok(columns) = services::schema::columns(ctx.session, &reference).await {
                    for column in columns {
                        if column.name.to_lowercase().contains(&keyword) {
                            column_hits.push(format!("{name}.{} {}", column.name, column.data_type));
                        }
                    }
                }
            }
        }
    }

    let mut out = String::new();
    if table_hits.is_empty() && column_hits.is_empty() {
        out.push_str(&format!("Nothing in the schema matches \"{keyword}\"."));
    } else {
        if !table_hits.is_empty() {
            out.push_str(&format!("Tables named like \"{keyword}\":\n"));
            for hit in table_hits.iter().take(60) {
                out.push_str(&format!("- {hit}\n"));
            }
        }
        if !column_hits.is_empty() {
            out.push_str(&format!("\nColumns named like \"{keyword}\":\n"));
            for hit in column_hits.iter().take(60) {
                out.push_str(&format!("- {hit}\n"));
            }
        }
    }
    Ok(ToolRun::ok(
        format!("{} tables, {} columns", table_hits.len(), column_hits.len()),
        out,
    ))
}

async fn sample_rows(ctx: &ToolCtx<'_>, args: &Json) -> AppResult<ToolRun> {
    let table = resolve_table(ctx, args).await?;
    let limit = args
        .get("limit")
        .and_then(Json::as_u64)
        .unwrap_or(SAMPLE_ROWS as u64)
        .min(SAMPLE_ROWS as u64) as u32;
    let query = PageQuery {
        sort: Vec::new(),
        filters: Vec::new(),
        offset: 0,
        limit: guard::clamp_page_limit(limit),
    };
    let page = services::data::table_page(ctx.session, &table, &query).await?;
    let headers: Vec<String> = page.columns.iter().map(|c| c.name.clone()).collect();
    let out = format!(
        "{} — {} sample rows\n\n{}",
        qualified(&table.schema, &table.name),
        page.rows.len(),
        render_rows(&headers, &page.rows)
    );
    Ok(ToolRun::ok(format!("{} rows", page.rows.len()), out))
}

async fn explain_query(ctx: &ToolCtx<'_>, args: &Json) -> AppResult<ToolRun> {
    let statement = arg_str(args, "statement")?;
    let settings = crate::model::AiSettings { provider: crate::model::AiProvider::None, ..Default::default() };
    let report = services::ai::explain(
        ctx.session,
        &services::ai::AiRequest { settings: &settings, api_key: None },
        statement,
        QUERY_ROWS,
    )
    .await?;
    Ok(ToolRun::ok("plan read", format!("Execution plan:\n{}", report.plan)))
}

async fn list_objects(ctx: &ToolCtx<'_>, args: &Json) -> AppResult<ToolRun> {
    let kind = parse_kind(ctx, arg_str(args, "kind")?)?;
    let parent = args.get("parent").and_then(Json::as_str).filter(|p| !p.is_empty());
    let objects = services::objects::list(ctx.session, kind, parent).await?;
    let mut out = format!("{} ({}):\n", kind_name(kind), objects.len());
    for object in objects.iter().take(MAX_LIST) {
        let detail = object.detail.as_deref().map(|d| format!(" — {d}")).unwrap_or_default();
        let parent = object.reference.parent.as_deref().map(|p| format!("{p}.")).unwrap_or_default();
        out.push_str(&format!("- {parent}{}{detail}\n", object.reference.name));
    }
    Ok(ToolRun::ok(format!("{} {}", objects.len(), kind_name(kind)), out))
}

async fn describe_object(ctx: &ToolCtx<'_>, args: &Json) -> AppResult<ToolRun> {
    let kind = parse_kind(ctx, arg_str(args, "kind")?)?;
    let reference = ObjectRef {
        kind,
        name: arg_str(args, "name")?.to_string(),
        parent: args.get("parent").and_then(Json::as_str).map(str::to_string),
    };
    let detail = services::objects::detail(ctx.session, &reference).await?;
    let mut out = format!("{} {}\n", kind_name(kind), reference.name);
    for property in detail.properties.iter().take(40) {
        out.push_str(&format!("- {}: {}\n", property.name, property.value));
    }
    if let Some(definition) = &detail.definition {
        out.push_str(&format!("\nDefinition:\n{}\n", definition.trim()));
    }
    Ok(ToolRun::ok("described", out))
}

async fn server_stats(ctx: &ToolCtx<'_>) -> AppResult<ToolRun> {
    let stats = services::objects::stats(ctx.session).await?;
    let mut out = String::new();
    for group in &stats.groups {
        out.push_str(&format!("{}:\n", group.title));
        for stat in &group.stats {
            let unit = stat.unit.as_deref().unwrap_or("");
            out.push_str(&format!("- {}: {}{unit}\n", stat.label, stat.value));
        }
    }
    if out.is_empty() {
        out.push_str("This engine reports no server statistics.");
    }
    Ok(ToolRun::ok(format!("{} groups", stats.groups.len()), out))
}

// ---------------------------------------------------------------------------
// Statement tools — these are the ones that can change data
// ---------------------------------------------------------------------------

async fn run_query(ctx: &ToolCtx<'_>, args: &Json, gate: &mut dyn PermissionGate) -> AppResult<ToolRun> {
    let statement = arg_str(args, "statement")?.to_string();
    let purpose = args.get("purpose").and_then(Json::as_str).unwrap_or("").to_string();
    let outcome = authorize_and_execute(ctx, &statement, &purpose, gate, QUERY_ROWS).await?;
    let (summary, body) = summarize_outcome(&outcome);
    Ok(ToolRun::ok(summary, body).with_sql(statement))
}

async fn render_chart(ctx: &ToolCtx<'_>, args: &Json, gate: &mut dyn PermissionGate) -> AppResult<ToolRun> {
    let statement = arg_str(args, "statement")?.to_string();
    let title = arg_str(args, "title")?.to_string();
    let chart = parse_chart(arg_str(args, "chart")?)?;
    let outcome = authorize_and_execute(ctx, &statement, &title, gate, QUERY_ROWS).await?;
    let (summary, body) = summarize_outcome(&outcome);
    let artifact = AgentArtifact::Chart {
        id: format!("chart-{}", ctx.session.elapsed_ms()),
        title,
        chart,
        x_label: args.get("xLabel").and_then(Json::as_str).map(str::to_string),
        y_label: args.get("yLabel").and_then(Json::as_str).map(str::to_string),
        sql: statement.clone(),
        outcome,
    };
    Ok(ToolRun::ok(summary, format!("Chart drawn for the user.\n{body}"))
        .with_artifact(artifact)
        .with_sql(statement))
}

async fn render_table(ctx: &ToolCtx<'_>, args: &Json, gate: &mut dyn PermissionGate) -> AppResult<ToolRun> {
    let statement = arg_str(args, "statement")?.to_string();
    let title = arg_str(args, "title")?.to_string();
    let outcome = authorize_and_execute(ctx, &statement, &title, gate, QUERY_ROWS).await?;
    let (summary, body) = summarize_outcome(&outcome);
    let artifact = AgentArtifact::Table {
        id: format!("table-{}", ctx.session.elapsed_ms()),
        title,
        sql: statement.clone(),
        outcome,
    };
    Ok(ToolRun::ok(summary, format!("Grid shown to the user.\n{body}"))
        .with_artifact(artifact)
        .with_sql(statement))
}

// WHAT:  Draw a node-and-edge graph.
// WHY:   "How does this database fit together" is a shape question, and a list
//        of foreign keys is the worst possible answer to it. Relationship mode
//        works on every engine that reports keys, so the agent can draw a map
//        even where there is no graph query language.
async fn render_graph(ctx: &ToolCtx<'_>, args: &Json, gate: &mut dyn PermissionGate) -> AppResult<ToolRun> {
    let title = arg_str(args, "title")?.to_string();
    let mode = args.get("mode").and_then(Json::as_str).unwrap_or("relationships");
    let id = format!("graph-{}", ctx.call_id);

    if mode == "query" {
        let statement = arg_str(args, "statement")?.to_string();
        let outcome = authorize_and_execute(ctx, &statement, &title, gate, QUERY_ROWS).await?;
        let (summary, body) = summarize_outcome(&outcome);
        let artifact = AgentArtifact::Graph {
            id,
            title,
            sql: statement.clone(),
            nodes: Vec::new(),
            edges: Vec::new(),
            outcome: Some(outcome),
        };
        return Ok(ToolRun::ok(summary, format!("Graph drawn for the user.\n{body}"))
            .with_artifact(artifact)
            .with_sql(statement));
    }

    let (nodes, edges) = relationship_graph(ctx, args).await?;
    if nodes.is_empty() {
        return Err(AppError::not_found(
            "No foreign keys were found, so there is nothing to draw. Say so rather than \
             inventing relationships."
                .to_string(),
        ));
    }
    let summary = format!("{} tables, {} links", nodes.len(), edges.len());
    let content = format!(
        "Relationship graph drawn for the user: {} tables joined by {} foreign keys.",
        nodes.len(),
        edges.len()
    );
    let artifact = AgentArtifact::Graph {
        id,
        title,
        sql: String::new(),
        nodes,
        edges,
        outcome: None,
    };
    Ok(ToolRun::ok(summary, content).with_artifact(artifact))
}

// WHAT:  Tables as nodes, foreign keys as edges.
// WHY:   This is the one graph every relational engine can produce, and it is
//        the one users actually ask for.
async fn relationship_graph(
    ctx: &ToolCtx<'_>,
    args: &Json,
) -> AppResult<(Vec<UiGraphNode>, Vec<UiGraphEdge>)> {
    let wanted: Vec<String> = args
        .get("tables")
        .and_then(Json::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Json::as_str)
                .map(|name| name.to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let keep = |name: &str| wanted.is_empty() || wanted.iter().any(|w| name.to_lowercase().ends_with(w));

    let foreign_keys = services::schema::foreign_keys(ctx.session).await.unwrap_or_default();
    let mut nodes: Vec<UiGraphNode> = Vec::new();
    let mut edges: Vec<UiGraphEdge> = Vec::new();

    let push_node = |nodes: &mut Vec<UiGraphNode>, schema: &Option<String>, table: &str| {
        let id = qualified(schema, table);
        if !nodes.iter().any(|n| n.id == id) {
            nodes.push(UiGraphNode {
                id,
                // Schema decides the colour, so a cross-schema link is visible.
                label: schema.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "table".to_string()),
                caption: table.to_string(),
            });
        }
    };

    for key in &foreign_keys {
        let from = qualified(&key.from_schema, &key.from_table);
        let to = qualified(&key.to_schema, &key.to_table);
        if !keep(&from) && !keep(&to) {
            continue;
        }
        push_node(&mut nodes, &key.from_schema, &key.from_table);
        push_node(&mut nodes, &key.to_schema, &key.to_table);
        edges.push(UiGraphEdge {
            id: if key.name.is_empty() { format!("{from}->{to}") } else { key.name.clone() },
            from,
            to,
            label: key.from_columns.join(", "),
        });
    }

    // Tables the user asked about that join to nothing still belong on the map.
    if !wanted.is_empty() {
        let catalog = services::schema::catalog(ctx.session).await?;
        for schema in &catalog.schemas {
            for table in &schema.tables {
                if keep(&qualified(&table.schema, &table.name)) {
                    push_node(&mut nodes, &table.schema, &table.name);
                }
            }
        }
    }
    Ok((nodes, edges))
}

// WHAT:  Build the layout the agent composed.
// WHY:   Blocks are a closed vocabulary, so the model assembles components the
//        app already ships and cannot emit markup or script. Any block carrying
//        `sql` is executed here, through the same gate as every other statement.
async fn render_ui(ctx: &ToolCtx<'_>, args: &Json, gate: &mut dyn PermissionGate) -> AppResult<ToolRun> {
    let title = arg_str(args, "title")?.to_string();
    let raw = args
        .get("blocks")
        .and_then(Json::as_array)
        .ok_or_else(|| AppError::invalid_input("render_ui needs a `blocks` array."))?;
    if raw.is_empty() {
        return Err(AppError::invalid_input("render_ui was given no blocks to draw."));
    }

    let mut blocks: Vec<UiBlock> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut last_sql: Option<String> = None;

    for entry in raw.iter().take(12) {
        let kind = entry.get("block").and_then(Json::as_str).unwrap_or_default();
        match kind {
            "heading" => blocks.push(UiBlock::Heading { text: arg_str(entry, "text")?.to_string() }),
            "text" => blocks.push(UiBlock::Text { markdown: arg_str(entry, "markdown")?.to_string() }),
            "divider" => blocks.push(UiBlock::Divider),
            "callout" => blocks.push(UiBlock::Callout {
                tone: parse_tone(entry.get("tone").and_then(Json::as_str).unwrap_or("info")),
                title: arg_str(entry, "title")?.to_string(),
                body: entry.get("body").and_then(Json::as_str).unwrap_or_default().to_string(),
            }),
            "stats" => blocks.push(UiBlock::Stats { stats: parse_stats(entry)? }),
            "facts" => blocks.push(UiBlock::Facts {
                title: entry.get("title").and_then(Json::as_str).unwrap_or_default().to_string(),
                rows: parse_stats(entry)?,
            }),
            "chart" => {
                let sql = arg_str(entry, "sql")?.to_string();
                let block_title = entry.get("title").and_then(Json::as_str).unwrap_or("").to_string();
                let outcome = authorize_and_execute(ctx, &sql, &block_title, gate, QUERY_ROWS).await?;
                let (summary, _) = summarize_outcome(&outcome);
                notes.push(format!("chart \"{block_title}\": {summary}"));
                last_sql = Some(sql.clone());
                blocks.push(UiBlock::Chart {
                    title: block_title,
                    chart: parse_chart(entry.get("chart").and_then(Json::as_str).unwrap_or("bar"))?,
                    x_label: entry.get("xLabel").and_then(Json::as_str).map(str::to_string),
                    y_label: entry.get("yLabel").and_then(Json::as_str).map(str::to_string),
                    sql,
                    outcome,
                });
            }
            "table" => {
                let sql = arg_str(entry, "sql")?.to_string();
                let block_title = entry.get("title").and_then(Json::as_str).unwrap_or("").to_string();
                let outcome = authorize_and_execute(ctx, &sql, &block_title, gate, QUERY_ROWS).await?;
                let (summary, _) = summarize_outcome(&outcome);
                notes.push(format!("table \"{block_title}\": {summary}"));
                last_sql = Some(sql.clone());
                blocks.push(UiBlock::Table { title: block_title, sql, outcome });
            }
            "graph" => {
                let (nodes, edges) = relationship_graph(ctx, entry).await?;
                notes.push(format!("graph: {} tables, {} links", nodes.len(), edges.len()));
                blocks.push(UiBlock::Graph {
                    title: entry.get("title").and_then(Json::as_str).unwrap_or_default().to_string(),
                    nodes,
                    edges,
                });
            }
            other => {
                return Err(AppError::invalid_input(format!(
                    "\"{other}\" is not a block type. Use heading, text, stats, chart, table, graph, \
                     facts, callout or divider."
                )));
            }
        }
    }

    let summary = format!("{} blocks", blocks.len());
    let content = format!(
        "View \"{title}\" drawn for the user with {} blocks. {}\nDescribe what it shows; do not \
         repeat the numbers back in full.",
        blocks.len(),
        notes.join("; ")
    );
    let artifact = AgentArtifact::Ui { id: format!("ui-{}", ctx.call_id), title, blocks };
    let mut run = ToolRun::ok(summary, content).with_artifact(artifact);
    if let Some(sql) = last_sql {
        run = run.with_sql(sql);
    }
    Ok(run)
}

fn parse_stats(entry: &Json) -> AppResult<Vec<UiStat>> {
    let list = entry
        .get("stats")
        .and_then(Json::as_array)
        .ok_or_else(|| AppError::invalid_input("This block needs a `stats` array."))?;
    Ok(list
        .iter()
        .take(8)
        .map(|stat| UiStat {
            label: stat.get("label").and_then(Json::as_str).unwrap_or_default().to_string(),
            value: stat.get("value").and_then(Json::as_str).unwrap_or_default().to_string(),
            hint: stat.get("hint").and_then(Json::as_str).map(str::to_string),
            trend: stat.get("trend").and_then(Json::as_f64),
        })
        .collect())
}

fn parse_tone(raw: &str) -> UiTone {
    match raw {
        "success" => UiTone::Success,
        "warning" => UiTone::Warning,
        "danger" => UiTone::Danger,
        _ => UiTone::Info,
    }
}

// WHAT:  The gate every statement the agent runs passes through.
// WHY:   The agent is not the user. A read may run unattended; anything that
//        writes has to be seen and approved by a human first, and a read-only
//        connection refuses regardless of what the user approves.
// HOW:   Classification is the guard's, not a second parser — the same code that
//        decides what the Run button confirms decides what the agent must ask about.
async fn authorize_and_execute(
    ctx: &ToolCtx<'_>,
    statement: &str,
    purpose: &str,
    gate: &mut dyn PermissionGate,
    max_rows: usize,
) -> AppResult<QueryOutcome> {
    let classified = classify(statement);
    let worst = classified
        .iter()
        .map(|s| s.kind)
        .max_by_key(|kind| match kind {
            StatementKind::Read => 0u8,
            StatementKind::Write => 1,
            StatementKind::Destructive => 2,
        })
        .unwrap_or(StatementKind::Read);

    // The connection's own lock outranks anything the model or the user wants.
    if ctx.session.connection.read_only && worst != StatementKind::Read {
        return Err(AppError::read_only(
            "This connection is read-only, so I cannot run statements that change data.".to_string(),
        ));
    }

    let needs_ask = match (ctx.autonomy, worst) {
        (_, StatementKind::Read) => false,
        (AgentAutonomy::ReadOnly, _) => {
            return Err(AppError::read_only(
                "The assistant is set to read-only. Change it in Settings → AI to let it write."
                    .to_string(),
            ));
        }
        (AgentAutonomy::AskOnWrite, _) => true,
        (AgentAutonomy::Full, StatementKind::Write) => false,
        (AgentAutonomy::Full, StatementKind::Destructive) => true,
    };

    if needs_ask {
        let previews: Vec<String> = classified
            .iter()
            .filter(|s| s.kind != StatementKind::Read)
            .map(|s| match &s.reason {
                Some(reason) => format!("{} — {reason}", preview(&s.text)),
                None => preview(&s.text),
            })
            .collect();
        let request = PermissionRequest {
            call_id: ctx.call_id.clone(),
            tool: "run_query".to_string(),
            title: if purpose.is_empty() { "Run a statement".to_string() } else { purpose.to_string() },
            statement: Some(statement.to_string()),
            statements: previews,
            intent: intent_of(worst),
            reason: match worst {
                StatementKind::Destructive => {
                    "This statement destroys or overwrites data and cannot be undone.".to_string()
                }
                _ => "This statement changes data in your database.".to_string(),
            },
        };
        match gate.ask(request).await {
            PermissionDecision::Deny => {
                return Err(AppError::invalid_input(
                    "The user declined to run this statement. Do not try to run it again; \
                     explain what it would have done, or offer a read-only alternative."
                        .to_string(),
                ));
            }
            PermissionDecision::Allow | PermissionDecision::AllowForRun => {}
        }
    }

    services::query::execute(ctx.session, statement, max_rows, None).await
}

fn intent_of(kind: StatementKind) -> StatementIntent {
    match kind {
        StatementKind::Read => StatementIntent::Read,
        StatementKind::Write => StatementIntent::Write,
        StatementKind::Destructive => StatementIntent::Destructive,
    }
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Turn an outcome into a headline plus a compact body the model can read.
fn summarize_outcome(outcome: &QueryOutcome) -> (String, String) {
    let mut body = String::new();
    let mut rows_seen = 0usize;
    for statement in &outcome.statements {
        match statement {
            StatementResult::Rows { result } => {
                rows_seen += result.rows.len();
                let headers: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
                body.push_str(&render_rows(&headers, &result.rows));
                if result.truncated {
                    body.push_str("\n(result truncated)\n");
                }
            }
            StatementResult::Affected { rows_affected } => {
                body.push_str(&format!("{rows_affected} rows affected.\n"));
            }
        }
    }
    if body.is_empty() {
        body.push_str("Statement completed with no rows.");
    }
    let summary = format!("{rows_seen} rows · {} ms", outcome.elapsed_ms);
    (summary, body)
}

// WHAT:  Rows as a pipe table.
// WHY:   Cheap to tokenize and the model reads it reliably; JSON would repeat
//        every column name on every row.
fn render_rows(headers: &[String], rows: &[Vec<Value>]) -> String {
    if headers.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(&headers.join(" | "));
    out.push('\n');
    out.push_str(&headers.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
    out.push('\n');
    for row in rows.iter().take(QUERY_ROWS) {
        let cells: Vec<String> = row.iter().map(cell_text).collect();
        out.push_str(&cells.join(" | "));
        out.push('\n');
    }
    out
}

fn cell_text(value: &Value) -> String {
    let raw = match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Decimal(s) | Value::Text(s) | Value::Bytes(s) | Value::DateTime(s) | Value::Unsupported(s) => {
            s.clone()
        }
        Value::Json(j) => j.to_string(),
    };
    let flat = raw.replace(['\n', '\r'], " ");
    if flat.chars().count() > MAX_CELL {
        let head: String = flat.chars().take(MAX_CELL).collect();
        format!("{head}…")
    } else {
        flat
    }
}

fn preview(sql: &str) -> String {
    let flat: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 120 {
        let head: String = flat.chars().take(117).collect();
        format!("{head}...")
    } else {
        flat
    }
}

fn qualified(schema: &Option<String>, name: &str) -> String {
    match schema {
        Some(s) if !s.is_empty() => format!("{s}.{name}"),
        _ => name.to_string(),
    }
}

// WHAT:  Accept "schema.table", a bare name, or a separate `schema` argument.
// WHY:   Models mix these freely; failing on the punctuation would waste a turn.
async fn resolve_table(ctx: &ToolCtx<'_>, args: &Json) -> AppResult<TableRef> {
    let raw = arg_str(args, "table")?;
    let explicit = args.get("schema").and_then(Json::as_str).filter(|s| !s.is_empty());
    let (schema, name) = match (explicit, raw.split_once('.')) {
        (Some(s), _) => (Some(s.to_string()), raw.rsplit('.').next().unwrap_or(raw).to_string()),
        (None, Some((s, n))) => (Some(s.to_string()), n.to_string()),
        (None, None) => (None, raw.to_string()),
    };

    // Match the catalogue so casing and an omitted schema still resolve.
    let catalog = services::schema::catalog(ctx.session).await?;
    for candidate in &catalog.schemas {
        if let Some(want) = &schema {
            if !candidate.name.eq_ignore_ascii_case(want) {
                continue;
            }
        }
        for table in &candidate.tables {
            if table.name.eq_ignore_ascii_case(&name) {
                return Ok(TableRef { schema: table.schema.clone(), name: table.name.clone() });
            }
        }
    }
    Ok(TableRef { schema, name })
}

fn parse_kind(ctx: &ToolCtx<'_>, raw: &str) -> AppResult<ObjectKind> {
    let profile = integrations::profile(ctx.session.connection.engine.family());
    profile
        .object_kinds
        .iter()
        .copied()
        .find(|kind| kind_name(*kind).eq_ignore_ascii_case(raw))
        .ok_or_else(|| {
            AppError::invalid_input(format!(
                "\"{raw}\" is not an object kind this engine exposes. Available: {}",
                profile.object_kinds.iter().map(|k| kind_name(*k)).collect::<Vec<_>>().join(", ")
            ))
        })
}

fn parse_chart(raw: &str) -> AppResult<WidgetKind> {
    match raw.to_lowercase().as_str() {
        "line" => Ok(WidgetKind::Line),
        "area" => Ok(WidgetKind::Area),
        "bar" => Ok(WidgetKind::Bar),
        "pie" => Ok(WidgetKind::Pie),
        other => Err(AppError::invalid_input(format!(
            "\"{other}\" is not a chart kind. Use line, area, bar or pie."
        ))),
    }
}
