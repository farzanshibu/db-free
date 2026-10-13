// SOT: shell-command-service, shell-meta-execution, meta-command-run

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::integrations::shell_meta::{self, Plan, Segment, Step};
use crate::model::StatementResult;
use crate::services::query::{run, Prelude};

// WHAT:  Runs a script that holds shell meta-commands (`\dt`, `.tables`, `DESC emp`,
//        `:schema`, `help`…) among, or instead of, ordinary statements.
// WHY:   The query tab accepts the structure commands of each engine's own shell. The
//        block has already classified them as reads (guard/language.rs), so what is
//        left is to turn each into the catalog queries it stands for and run those
//        in the order the script gave them, beside the text that is not a command.
// HOW:   `shell_meta::segments` cut the script. Text that is not a command is sent
//        exactly as typed. A command is planned (`shell_meta::plan`) and its steps
//        run in turn through the same path as any statement, so the editor's schema
//        picker (the prelude) applies to them: a local table is answered here; a
//        `Section` that comes back empty is left out of the answer; an `Exists` that
//        comes back empty fails with its own message. An error from a step names the
//        command that was typed, since the statement the database saw is not what the
//        user wrote, and drops the position it reports for the same reason.
// WHERE: src-tauri/src/services/query.rs (`execute`), src-tauri/src/integrations/shell_meta/mod.rs
pub async fn execute(ctx: &SessionCtx, prelude: &Prelude, segments: &[Segment], namespace: Option<&str>, max_rows: usize) -> AppResult<Vec<StatementResult>> {
    let planning = shell_meta::Ctx::new(ctx.connection.engine, Some(&ctx.connection), namespace);
    let mut out = Vec::new();
    for segment in segments {
        match segment {
            Segment::Native(text) => out.extend(run(ctx, prelude, text, max_rows).await.map_err(|err| without_position(err, None))?),
            Segment::Meta(text) => {
                let plan = match shell_meta::plan(&planning, text) {
                    Some(planned) => planned?,
                    None => return Err(AppError::internal(format!("`{text}` was cut out as a command but is not one."))),
                };
                out.extend(run_plan(ctx, prelude, &plan, text, max_rows).await?);
            }
        }
    }
    Ok(out)
}

async fn run_plan(ctx: &SessionCtx, prelude: &Prelude, plan: &Plan, typed: &str, max_rows: usize) -> AppResult<Vec<StatementResult>> {
    let mut out = Vec::new();
    for step in &plan.steps {
        match step {
            Step::Table(table) => out.push(table.result()),
            Step::Native(statement) => out.extend(run(ctx, prelude, statement, max_rows).await.map_err(|err| without_position(err, Some(typed)))?),
            Step::Section(statement) => {
                let found = run(ctx, prelude, statement, max_rows).await.map_err(|err| without_position(err, Some(typed)))?;
                out.extend(found.into_iter().filter(|result| !is_empty(result)));
            }
            Step::Exists(statement, missing) => {
                let found = run(ctx, prelude, statement, max_rows).await.map_err(|err| without_position(err, Some(typed)))?;
                if found.iter().all(is_empty) {
                    return Err(AppError::invalid_input(missing.clone()));
                }
                out.extend(found);
            }
        }
    }
    Ok(out)
}

/// A result set with no rows (a statement that affected rows is not empty).
fn is_empty(result: &StatementResult) -> bool {
    matches!(result, StatementResult::Rows { result } if result.rows.is_empty())
}

// WHAT:  A database error as the editor should show it for a run that was not the
//        user's own text: the command in front of the message, no position.
// WHY:   The position points into the statement DB Free generated, which is not in
//        the editor; underlining the same offset of the user's command would lie.
fn without_position(err: AppError, command: Option<&str>) -> AppError {
    match err {
        AppError::Driver { message, hint, detail, .. } => {
            let message = command.map_or(message.clone(), |typed| format!("{typed}: {message}"));
            AppError::driver_with(message, None, hint, detail)
        }
        other => other,
    }
}

/// Rows in hand and whether a cap cut them, for a run that has no single statement to count.
pub fn rows_in_hand(results: &[StatementResult]) -> (u64, bool) {
    let mut rows = 0_u64;
    let mut truncated = false;
    for result in results {
        if let StatementResult::Rows { result } = result {
            rows += result.row_count();
            truncated |= result.truncated;
        }
    }
    (rows, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::tests::state_with_sqlite;
    use crate::guard::{self, StatementRequest};
    use crate::model::{BatchEvent, QueryOutcome, Value};
    use crate::services::query::{self, BatchRequest};
    use crate::state::AppState;
    use std::sync::Mutex;

    /// One script through the block and the query service, as the editor's Run does.
    async fn run_script(state: &AppState, id: &str, sql: &str) -> AppResult<QueryOutcome> {
        let text = sql.to_string();
        guard::statement(state, StatementRequest { connection_id: id, sql, confirm_destructive: false, run_id: None }, |ctx| async move {
            query::execute(&ctx, &text, 1000, None).await
        })
        .await
    }

    fn rows_of(outcome: &QueryOutcome, tab: usize) -> Vec<Vec<String>> {
        match outcome.statements.get(tab) {
            Some(StatementResult::Rows { result }) => result
                .rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| match cell {
                            Value::Text(t) | Value::Decimal(t) | Value::DateTime(t) => t.clone(),
                            Value::Int(i) => i.to_string(),
                            Value::Null => String::new(),
                            other => format!("{other:?}"),
                        })
                        .collect()
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn columns_of(outcome: &QueryOutcome, tab: usize) -> Vec<String> {
        match outcome.statements.get(tab) {
            Some(StatementResult::Rows { result }) => result.columns.iter().map(|c| c.name.clone()).collect(),
            _ => Vec::new(),
        }
    }

    // A DuckDB database in a temp file, behind the same block, for the commands that
    // read DuckDB's own catalog functions instead of `sqlite_master`.
    async fn duckdb_state() -> (AppState, String) {
        use crate::adapters::keyring::MemoryKeyProvider;
        use crate::model::{ConnectionInput, ResolvedConnection};
        use crate::store::Store;
        let dir = std::env::temp_dir().join(format!("db-free-shellcmd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let path = dir.join("d.duckdb").to_string_lossy().into_owned();
        let input: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "duck", "engine": "duckdb", "environment": "local", "readOnly": false, "host": null, "port": null,
            "database": null, "username": null, "password": null, "filePath": path, "sslMode": "disable",
            "ssh": serde_json::to_value(crate::model::SshTunnel::default()).unwrap_or_default(),
            "sshSecret": null, "folder": null, "color": null, "favorite": false, "extra": {}
        }))
        .unwrap_or_else(|e| panic!("{e}"));
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let summary = store.insert_connection(&input, None).unwrap_or_else(|e| panic!("{e}"));
        let state = AppState::new(store, Box::new(MemoryKeyProvider::default()));
        let resolved = ResolvedConnection { summary: summary.clone(), secret: None };
        let integration = crate::integrations::connect(&resolved).await.unwrap_or_else(|e| panic!("{e}"));
        state.insert_session(summary.id.clone(), integration).await;
        (state, summary.id)
    }

    #[tokio::test]
    async fn duckdb_dot_commands_read_duckdbs_own_catalog() {
        let (state, id) = duckdb_state().await;
        run_script(&state, &id, "CREATE TABLE users (id INTEGER PRIMARY KEY, email VARCHAR); CREATE TABLE orders (id INTEGER, user_id INTEGER); CREATE VIEW big AS SELECT * FROM orders WHERE id > 100; CREATE INDEX orders_user ON orders (user_id); CREATE SEQUENCE ticket START 7; CREATE SCHEMA ops; CREATE TABLE ops.jobs (n INTEGER)")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let tables = run_script(&state, &id, ".tables").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(columns_of(&tables, 0), ["name", "type"]);
        assert_eq!(rows_of(&tables, 0), [["big", "view"], ["ops.jobs", "table"], ["orders", "table"], ["users", "table"]]);
        let narrowed = run_script(&state, &id, ".tables 'us%'").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(rows_of(&narrowed, 0), [["users", "table"]]);
        let schema = run_script(&state, &id, ".schema orders").await.unwrap_or_else(|e| panic!("{e}"));
        let statements: Vec<String> = rows_of(&schema, 0).into_iter().map(|r| r[0].clone()).collect();
        assert!(statements.iter().any(|s| s.starts_with("CREATE TABLE orders") && s.ends_with(';')), "{statements:?}");
        assert!(statements.iter().any(|s| s.contains("CREATE INDEX orders_user")), "{statements:?}");
        let everything = run_script(&state, &id, ".schema").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(rows_of(&everything, 0).iter().any(|r| r[0].contains("CREATE SEQUENCE ticket")), "sequences are part of the schema");
        let indexes = run_script(&state, &id, ".indexes orders").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(rows_of(&indexes, 0).first().map(|r| (r[0].clone(), r[1].clone())), Some(("orders_user".to_string(), "orders".to_string())));
        let databases = run_script(&state, &id, ".databases").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(columns_of(&databases, 0), ["name", "file", "type", "readonly"]);
        assert_eq!(rows_of(&databases, 0).len(), 1, "the system and temp databases are internal");
        let help = run_script(&state, &id, ".help").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(rows_of(&help, 0).iter().any(|r| r[0] == ".tables"));
        // sqlite3-only commands say so instead of reaching DuckDB.
        assert!(matches!(run_script(&state, &id, ".fullschema").await, Err(AppError::InvalidInput { .. })));
    }

    async fn seeded() -> (AppState, String) {
        let (state, id) = state_with_sqlite(false).await;
        run_script(&state, &id, "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL UNIQUE); CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER REFERENCES users(id), total REAL); CREATE INDEX orders_user ON orders (user_id, total); CREATE VIEW big AS SELECT * FROM orders WHERE total > 100; CREATE TRIGGER users_ai AFTER INSERT ON users BEGIN SELECT 1; END; INSERT INTO users (email) VALUES ('a@x.io')")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        (state, id)
    }

    /// Runs `plan` as the service does for a typed command, inside the block.
    async fn run_planned(state: &AppState, id: &str, plan: Plan) -> AppResult<QueryOutcome> {
        guard::statement(state, StatementRequest { connection_id: id, sql: "SELECT 1", confirm_destructive: false, run_id: None }, |ctx| async move {
            let statements = run_plan(&ctx, &None, &plan, "\\typed", 100).await?;
            Ok(QueryOutcome { statements, total_rows: None, elapsed_ms: 0 })
        })
        .await
    }

    #[tokio::test]
    async fn a_plan_keeps_its_tables_and_sections_in_order_and_drops_the_empty_sections() {
        use crate::integrations::shell_meta::{Plan, Step, Table};
        let (state, id) = seeded().await;
        let plan = Plan::default()
            .then(Step::Table(Table::new(&["Parameter", "Value"], vec![vec!["Host".into(), "h".into()]])))
            .then(Step::Native("SELECT 1 AS n".into()))
            .then(Step::Section("SELECT * FROM users WHERE 1 = 0".into()))
            .then(Step::Section("SELECT count(*) AS n FROM users".into()))
            .then(Step::Native("SELECT * FROM users WHERE 1 = 0".into()));
        let outcome = run_planned(&state, &id, plan).await.unwrap_or_else(|e| panic!("{e}"));
        let shapes: Vec<(Vec<String>, usize)> = (0..outcome.statements.len()).map(|i| (columns_of(&outcome, i), rows_of(&outcome, i).len())).collect();
        assert_eq!(
            shapes,
            vec![(vec!["Parameter".to_string(), "Value".to_string()], 1), (vec!["n".to_string()], 1), (vec!["n".to_string()], 1), (vec!["id".to_string(), "email".to_string()], 0)],
            "the empty section is gone; the empty plain statement stays"
        );
    }

    #[tokio::test]
    async fn an_exists_step_fails_with_its_message_when_nothing_matches() {
        use crate::integrations::shell_meta::{Plan, Step};
        let (state, id) = seeded().await;
        let missing = Plan::native("SELECT 1").then(Step::Exists("SELECT * FROM users WHERE id = -1".into(), "Did not find any relation named \"x\".".into()));
        let refused = run_planned(&state, &id, missing).await;
        assert!(matches!(&refused, Err(AppError::InvalidInput { message }) if message == "Did not find any relation named \"x\"."), "{refused:?}");
        let found = run_planned(&state, &id, Plan::default().then(Step::Exists("SELECT * FROM users".into(), "none".into()))).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(rows_of(&found, 0).len(), 1);
    }

    #[tokio::test]
    async fn a_failing_step_names_the_command_the_user_typed_and_drops_the_position() {
        use crate::integrations::shell_meta::{Plan, Step};
        let (state, id) = seeded().await;
        let failed = run_planned(&state, &id, Plan::default().then(Step::Native("SELECT * FORM users".into()))).await;
        match failed {
            Err(AppError::Driver { message, position, .. }) => {
                assert!(message.starts_with("\\typed: "), "{message}");
                assert_eq!(position, None, "the offset points into DB Free's statement, not the editor");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn dot_tables_lists_tables_and_views_but_not_sqlites_own() {
        let (state, id) = seeded().await;
        let outcome = run_script(&state, &id, ".tables").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(columns_of(&outcome, 0), ["name", "type"]);
        assert_eq!(rows_of(&outcome, 0), [["big", "view"], ["orders", "table"], ["users", "table"]], "sqlite_sequence is hidden");
        let narrowed = run_script(&state, &id, ".tables us%;").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(rows_of(&narrowed, 0), [["users", "table"]]);
        assert_eq!(rows_of(&run_script(&state, &id, ".tables 'zzz'").await.unwrap_or_else(|e| panic!("{e}")), 0), Vec::<Vec<String>>::new());
    }

    #[tokio::test]
    async fn dot_schema_prints_the_create_statements() {
        let (state, id) = seeded().await;
        let outcome = run_script(&state, &id, ".schema orders").await.unwrap_or_else(|e| panic!("{e}"));
        let sql: Vec<String> = rows_of(&outcome, 0).into_iter().map(|r| r[0].clone()).collect();
        assert_eq!(sql.len(), 2, "the table and its index: {sql:?}");
        assert!(sql[0].starts_with("CREATE TABLE orders") && sql[0].ends_with(';'), "{sql:?}");
        assert!(sql[1].starts_with("CREATE INDEX orders_user"), "{sql:?}");
        let user = run_script(&state, &id, ".schema --nosys users").await.unwrap_or_else(|e| panic!("{e}"));
        let kinds: Vec<String> = rows_of(&user, 0).into_iter().map(|r| r[0].clone()).collect();
        assert!(kinds.iter().any(|s| s.starts_with("CREATE TABLE users")) && kinds.iter().any(|s| s.contains("TRIGGER users_ai")), "{kinds:?}");
        let everything = run_script(&state, &id, ".schema").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(rows_of(&everything, 0).iter().any(|r| r[0].contains("sqlite_sequence")), "internal tables are shown unless --nosys");
        let full = run_script(&state, &id, ".fullschema").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(rows_of(&full, 0), rows_of(&everything, 0));
    }

    #[tokio::test]
    async fn dot_indexes_names_table_and_columns() {
        let (state, id) = seeded().await;
        let outcome = run_script(&state, &id, ".indexes orders").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(columns_of(&outcome, 0), ["index", "table", "columns", "unique", "sql"]);
        let rows = rows_of(&outcome, 0);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!((rows[0][0].as_str(), rows[0][1].as_str(), rows[0][2].as_str(), rows[0][3].as_str()), ("orders_user", "orders", "user_id, total", "no"));
        let all = run_script(&state, &id, ".indexes").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(rows_of(&all, 0).iter().any(|r| r[0].starts_with("sqlite_autoindex_users")), "automatic indexes are listed, as in sqlite3");
    }

    #[tokio::test]
    async fn dot_databases_dbinfo_and_help() {
        let (state, id) = seeded().await;
        let databases = run_script(&state, &id, ".databases").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(rows_of(&databases, 0).first().map(|r| r[1].clone()), Some("main".to_string()));
        let info = run_script(&state, &id, ".dbinfo").await.unwrap_or_else(|e| panic!("{e}"));
        let facts = rows_of(&info, 0);
        let fact = |key: &str| facts.iter().find(|r| r[0] == key).map(|r| r[1].clone());
        assert_eq!(fact("number of tables").as_deref(), Some("3"), "users, orders and sqlite_sequence");
        assert_eq!(fact("number of views").as_deref(), Some("1"));
        assert_eq!(fact("number of triggers").as_deref(), Some("1"));
        assert!(fact("database page size").is_some() && fact("software version").is_some(), "{facts:?}");
        let help = run_script(&state, &id, ".help").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(columns_of(&help, 0), ["Command", "Syntax", "Description", "Runs"]);
        assert!(rows_of(&help, 0).iter().any(|r| r[0] == ".tables"));
    }

    #[tokio::test]
    async fn commands_and_statements_run_in_the_order_written() {
        let (state, id) = seeded().await;
        let outcome = run_script(&state, &id, ".tables\nSELECT count(*) AS n FROM users;\n.indexes users").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(outcome.statements.len(), 3);
        assert_eq!(columns_of(&outcome, 0), ["name", "type"]);
        assert_eq!(rows_of(&outcome, 1), [["1"]]);
        assert_eq!(columns_of(&outcome, 2)[0], "index");
        assert_eq!(outcome.total_rows, Some(3 + 1 + 1));
    }

    #[tokio::test]
    async fn a_read_only_connection_still_answers_structure_commands() {
        let (state, id) = state_with_sqlite(true).await;
        for command in [".tables", ".schema", ".indexes", ".databases", ".dbinfo", ".help"] {
            assert!(run_script(&state, &id, command).await.is_ok(), "{command} is a read");
        }
        // The lock still stands for anything that is not a command.
        let refused = run_script(&state, &id, ".tables\nCREATE TABLE t (a int)").await;
        assert!(matches!(refused, Err(AppError::ReadOnly { .. })), "{refused:?}");
    }

    #[tokio::test]
    async fn an_unknown_dot_command_is_refused_without_reaching_the_database() {
        let (state, id) = seeded().await;
        let refused = run_script(&state, &id, ".frobnicate").await;
        assert!(matches!(&refused, Err(AppError::InvalidInput { message }) if message.contains(".frobnicate") && message.contains(".help")), "{refused:?}");
        let bad = run_script(&state, &id, ".tables a b").await;
        assert!(matches!(bad, Err(AppError::InvalidInput { .. })));
    }

    #[tokio::test]
    async fn a_hostile_argument_is_only_ever_a_pattern() {
        let (state, id) = seeded().await;
        let outcome = run_script(&state, &id, ".tables 'x''; DROP TABLE users; --'").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(rows_of(&outcome, 0).is_empty());
        let after = run_script(&state, &id, ".tables users").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(rows_of(&after, 0), [["users", "table"]], "nothing was dropped");
        let schema = run_script(&state, &id, ".schema nope.x").await;
        assert!(schema.is_err(), "an attached database that does not exist is an error, not an injection");
        let quoted = run_script(&state, &id, ".schema \"nope\".x").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(rows_of(&quoted, 0).is_empty(), "a quoted prefix is part of the pattern");
    }

    #[tokio::test]
    async fn the_log_keeps_what_was_typed_not_what_was_run() {
        let (state, id) = seeded().await;
        run_script(&state, &id, ".tables").await.unwrap_or_else(|e| panic!("{e}"));
        let history = state.with_store(|s| s.list_history(Some(&id), None, 5)).unwrap_or_default();
        assert_eq!(history.first().map(|h| h.sql.as_str()), Some(".tables"));
        assert_eq!(history.first().and_then(|h| h.operation), Some(crate::model::StatementIntent::Read));
    }

    #[tokio::test]
    async fn the_batch_runs_each_command_as_a_statement_with_its_own_result() {
        let (state, id) = seeded().await;
        let events = Mutex::new(Vec::new());
        let emit = |event: BatchEvent| {
            if let Ok(mut log) = events.lock() {
                log.push(event);
            }
        };
        let sql = ".tables\n.schema users\nSELECT 1";
        let req = BatchRequest { connection_id: &id, sql, confirm_destructive: false, max_rows: 100, namespace: None, run_id: "run-meta", stop_on_error: false, split: true, session_key: None };
        let summary = query::execute_batch(&state, &req, &emit).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((summary.total, summary.succeeded, summary.failed), (3, 3, 0));
        let planned = events.lock().map(|log| log.iter().find_map(|e| if let BatchEvent::Planned { statements } = e { Some(statements.iter().map(|s| (s.sql.clone(), s.intent)).collect::<Vec<_>>()) } else { None })).unwrap_or_default().unwrap_or_default();
        assert_eq!(planned.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), [".tables", ".schema users", "SELECT 1"]);
        assert!(planned.iter().all(|(_, intent)| *intent == crate::model::StatementIntent::Read));
    }
}
