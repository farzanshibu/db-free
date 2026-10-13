// SOT: changes-service, staged-change-sql, value-literal, commit-script, review-mode-sql, sql-token, rows-as-sql, commit-transaction, edit-dialect, clickhouse-mutations, dynamodb-partiql

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::integrations::sql::quote_literal_for;
use crate::integrations::{qualified_name_for, quote_ident_for};
use crate::model::{
    CellValue, ChangePreview, ChangeResult, CommitReport, Engine, Family, QueryOutcome, ResultSet, ReturnedRows, RowSqlKind, SqlToken,
    StagedChange, StatementResult, TableRef, Value,
};
use base64::Engine as _;

// WHAT:  Renders a cell Value as an SQL literal for the given engine.
// WHY:   The Pending Changes panel shows the exact SQL that will run; the UI never
//        builds SQL itself.
// HOW:   Text-like values go through quote_literal_for (quote doubling, plus
//        backslash escaping where the engine reads backslashes); binary becomes
//        the engine's hex form; non-finite floats become NULL.
pub fn literal(engine: Engine, value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => match engine.family() {
            Family::Sqlite
            | Family::Libsql
            | Family::ValTown
            | Family::CloudflareD1
            | Family::Mssql
            | Family::Oracle => {
                if *b { "1" } else { "0" }.to_string()
            }
            _ => {
                if *b { "TRUE" } else { "FALSE" }.to_string()
            }
        },
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            if f.is_finite() {
                f.to_string()
            } else {
                "NULL".to_string()
            }
        }
        Value::Decimal(s) => {
            if s.parse::<f64>().is_ok() {
                s.clone()
            } else {
                quote_literal_for(engine, s)
            }
        }
        Value::Text(s) | Value::DateTime(s) | Value::Unsupported(s) => quote_literal_for(engine, s),
        Value::Json(j) => quote_literal_for(engine, &j.to_string()),
        Value::Bytes(b64) => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap_or_default();
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            match engine.family() {
                Family::Postgres => format!("'\\x{hex}'::bytea"),
                Family::Mysql
                | Family::Sqlite
                | Family::Duckdb
                | Family::Libsql
                | Family::ValTown
                | Family::CloudflareD1
                | Family::Snowflake => format!("X'{hex}'"),
                Family::Mssql | Family::Cassandra => format!("0x{hex}"),
                Family::Clickhouse => format!("unhex('{hex}')"),
                Family::Oracle => format!("HEXTORAW('{hex}')"),
                Family::Bigquery => format!("FROM_HEX('{hex}')"),
                _ => quote_literal_for(engine, b64),
            }
        }
    }
}

// WHAT:  The engine's own spelling of a SQL token.
// WHY:   `now()` is a Postgres function; MySQL spells it NOW(), SQL Server
//        SYSDATETIME(), SQLite CURRENT_TIMESTAMP. The list is fixed, so a value
//        typed into a cell can never become arbitrary SQL.
// HOW:   `Default` is rendered only where `SET col = DEFAULT` exists; an insert
//        omits the column instead (see `statement`), which works everywhere.
pub fn token_sql(engine: Engine, token: SqlToken) -> AppResult<String> {
    let family = engine.family();
    let text = match token {
        SqlToken::Default => {
            if matches!(family, Family::Sqlite | Family::Libsql | Family::ValTown | Family::CloudflareD1 | Family::Clickhouse | Family::Druid) {
                return Err(AppError::invalid_input("This engine cannot set a column back to its DEFAULT; leave the column out of an insert instead."));
            }
            "DEFAULT"
        }
        SqlToken::Now => match family {
            Family::Postgres | Family::Clickhouse | Family::Duckdb => "now()",
            Family::Mysql => "NOW()",
            Family::Mssql => "SYSDATETIME()",
            Family::Oracle => "SYSTIMESTAMP",
            Family::Snowflake | Family::Bigquery => "CURRENT_TIMESTAMP()",
            _ => "CURRENT_TIMESTAMP",
        },
        SqlToken::CurrentTimestamp => match family {
            Family::Clickhouse => "now()",
            Family::Snowflake | Family::Bigquery => "CURRENT_TIMESTAMP()",
            _ => "CURRENT_TIMESTAMP",
        },
        SqlToken::CurrentDate => match family {
            Family::Mssql => "CAST(SYSDATETIME() AS DATE)",
            Family::Clickhouse => "today()",
            Family::Snowflake | Family::Bigquery => "CURRENT_DATE()",
            _ => "CURRENT_DATE",
        },
        SqlToken::CurrentTime => match family {
            Family::Mssql => "CAST(SYSDATETIME() AS TIME)",
            Family::Oracle => "TO_CHAR(SYSTIMESTAMP, 'HH24:MI:SS')",
            Family::Clickhouse => "formatDateTime(now(), '%T')",
            Family::Snowflake | Family::Bigquery => "CURRENT_TIME()",
            _ => "CURRENT_TIME",
        },
    };
    Ok(text.to_string())
}

fn cell_sql(engine: Engine, value: &Value, raw: Option<SqlToken>) -> AppResult<String> {
    match raw {
        Some(token) => token_sql(engine, token),
        None => Ok(literal(engine, value)),
    }
}

// WHAT:  How a connection's row edits are written.
// WHY:   Most engines take INSERT / UPDATE / DELETE; ClickHouse changes rows with
//        asynchronous `ALTER TABLE … UPDATE/DELETE` mutations; DynamoDB takes
//        PartiQL with its own literals. One enum keeps that choice out of the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditDialect {
    Sql,
    ClickhouseMutations,
    DynamoPartiql,
}

// WHAT:  The way this engine's rows can be edited, or None when they cannot.
// HOW:   Any family whose adapter speaks SQL, bar Druid (read-only analytics), plus
//        the two dialects above.
pub fn edit_dialect(engine: Engine) -> Option<EditDialect> {
    match engine.family() {
        Family::Dynamodb => Some(EditDialect::DynamoPartiql),
        Family::Clickhouse => Some(EditDialect::ClickhouseMutations),
        Family::Druid => None,
        family if crate::integrations::profile(family).capabilities.sql => Some(EditDialect::Sql),
        _ => None,
    }
}

pub fn supports_edits(engine: Engine) -> bool {
    edit_dialect(engine).is_some()
}

fn key_predicate(engine: Engine, dialect: EditDialect, key: &[CellValue]) -> AppResult<String> {
    if key.is_empty() {
        return Err(AppError::invalid_input("This table has no primary key, so rows cannot be addressed safely for editing."));
    }
    let mut parts = Vec::with_capacity(key.len());
    for k in key {
        let column = quote_ident_for(engine, &k.column);
        parts.push(match (&k.value, dialect) {
            (Value::Null, EditDialect::DynamoPartiql) => {
                return Err(AppError::invalid_input(format!("Key attribute \"{}\" is empty, so the item cannot be addressed.", k.column)));
            }
            (Value::Null, _) => format!("{column} IS NULL"),
            (value, EditDialect::DynamoPartiql) => format!("{column} = {}", partiql_literal(value)?),
            (value, _) => format!("{column} = {}", literal(engine, value)),
        });
    }
    Ok(parts.join(" AND "))
}

fn table_name(engine: Engine, dialect: EditDialect, table: &TableRef) -> String {
    match dialect {
        // A DynamoDB table has no schema; the name is all PartiQL wants.
        EditDialect::DynamoPartiql => quote_ident_for(engine, &table.name),
        _ => qualified_name_for(engine, table),
    }
}

// WHAT:  A value as a PartiQL literal (DynamoDB): numbers bare, strings single-quoted,
//        maps `{'k': v}`, lists `[v, …]`.
fn partiql_literal(value: &Value) -> AppResult<String> {
    Ok(match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) if f.is_finite() => f.to_string(),
        Value::Float(_) => return Err(AppError::invalid_input("DynamoDB cannot store NaN or infinity.")),
        Value::Decimal(s) if s.parse::<f64>().is_ok() => s.clone(),
        Value::Decimal(s) | Value::Text(s) | Value::DateTime(s) | Value::Unsupported(s) => partiql_string(s),
        Value::Json(j) => partiql_json(j),
        Value::Bytes(_) => return Err(AppError::invalid_input("Binary attributes cannot be edited from the grid.")),
    })
}

fn partiql_string(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "''"))
}

fn partiql_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => partiql_string(s),
        serde_json::Value::Array(items) => format!("[{}]", items.iter().map(partiql_json).collect::<Vec<_>>().join(", ")),
        serde_json::Value::Object(map) => {
            format!("{{{}}}", map.iter().map(|(k, v)| format!("{}: {}", partiql_string(k), partiql_json(v))).collect::<Vec<_>>().join(", "))
        }
    }
}

fn partiql_cell(value: &Value, raw: Option<SqlToken>) -> AppResult<String> {
    if raw.is_some() {
        return Err(AppError::invalid_input("DynamoDB has no DEFAULT or current-time functions to write."));
    }
    partiql_literal(value)
}

pub fn statement(engine: Engine, change: &StagedChange) -> AppResult<String> {
    if let StagedChange::Ddl { statement, .. } = change {
        let text = statement.trim().trim_end_matches(';').trim_end();
        if text.is_empty() {
            return Err(AppError::invalid_input("A statement change needs a statement."));
        }
        return Ok(text.to_string());
    }
    let dialect = edit_dialect(engine).ok_or_else(|| AppError::invalid_input("Inline editing is not available for this engine yet."))?;
    let value_sql = |value: &Value, raw: Option<SqlToken>| match dialect {
        EditDialect::DynamoPartiql => partiql_cell(value, raw),
        _ => cell_sql(engine, value, raw),
    };
    Ok(match change {
        StagedChange::Update { table, key, column, new, new_raw, .. } => {
            let target = table_name(engine, dialect, table);
            let assignment = format!("{} = {}", quote_ident_for(engine, column), value_sql(new, *new_raw)?);
            let predicate = key_predicate(engine, dialect, key)?;
            match dialect {
                EditDialect::ClickhouseMutations => format!("ALTER TABLE {target} UPDATE {assignment} WHERE {predicate}"),
                _ => format!("UPDATE {target} SET {assignment} WHERE {predicate}"),
            }
        }
        StagedChange::Insert { table, values, .. } => {
            // A column left at DEFAULT is simply not named: every engine then fills it in.
            let named: Vec<&CellValue> = values.iter().filter(|v| v.raw != Some(SqlToken::Default)).collect();
            let target = table_name(engine, dialect, table);
            if dialect == EditDialect::DynamoPartiql {
                if named.is_empty() {
                    return Err(AppError::invalid_input("An insert needs at least one value."));
                }
                let fields = named
                    .iter()
                    .map(|v| Ok::<String, AppError>(format!("{}: {}", partiql_string(&v.column), partiql_cell(&v.value, v.raw)?)))
                    .collect::<AppResult<Vec<_>>>()?;
                return Ok(format!("INSERT INTO {target} VALUE {{{}}}", fields.join(", ")));
            }
            if named.is_empty() {
                // Nothing to name: let the table fill the whole row.
                return Ok(match engine.family() {
                    Family::Mysql => format!("INSERT INTO {target} () VALUES ()"),
                    _ => format!("INSERT INTO {target} DEFAULT VALUES"),
                });
            }
            let columns = named.iter().map(|v| quote_ident_for(engine, &v.column)).collect::<Vec<_>>().join(", ");
            let cells = named.iter().map(|v| cell_sql(engine, &v.value, v.raw)).collect::<AppResult<Vec<_>>>()?.join(", ");
            format!("INSERT INTO {target} ({columns}) VALUES ({cells})")
        }
        StagedChange::Delete { table, key, .. } => {
            let target = table_name(engine, dialect, table);
            let predicate = key_predicate(engine, dialect, key)?;
            match dialect {
                EditDialect::ClickhouseMutations => format!("ALTER TABLE {target} DELETE WHERE {predicate}"),
                _ => format!("DELETE FROM {target} WHERE {predicate}"),
            }
        }
        StagedChange::Ddl { .. } => return Err(AppError::internal("handled above")),
    })
}

/// The statements that open and close an all-or-nothing script on engines with
/// transactions but no pinned-connection API: (before, commit, rollback).
type ScriptParts = (&'static [&'static str], &'static str, &'static str);

fn script_parts(engine: Engine) -> Option<ScriptParts> {
    match engine.family() {
        Family::Clickhouse | Family::CloudflareD1 | Family::ValTown | Family::Dynamodb | Family::Druid => None,
        Family::Mssql => Some((
            &["SET XACT_ABORT ON", "BEGIN TRANSACTION"],
            "COMMIT TRANSACTION",
            "IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION",
        )),
        // Oracle starts a transaction with the first statement; there is no BEGIN to send.
        Family::Oracle => Some((&[], "COMMIT", "ROLLBACK")),
        _ => Some((&["BEGIN"], "COMMIT", "ROLLBACK")),
    }
}

// WHAT:  Builds the script shown in the Pending Changes panel: the statements,
//        wrapped in the engine's transaction where it has one.
pub fn preview(engine: Engine, changes: &[StagedChange]) -> AppResult<ChangePreview> {
    let statements = changes.iter().map(|c| statement(engine, c)).collect::<AppResult<Vec<_>>>()?;
    let parts = if changes.iter().all(|c| matches!(c, StagedChange::Ddl { .. })) && edit_dialect(engine).is_none() {
        None
    } else {
        script_parts(engine)
    };
    let mut script = String::new();
    if let Some((before, _, _)) = parts {
        for s in before {
            script.push_str(s);
            script.push_str(";\n");
        }
    }
    for s in &statements {
        script.push_str(s);
        script.push_str(";\n");
    }
    if let Some((_, commit, _)) = parts {
        script.push_str(commit);
        script.push(';');
    }
    Ok(ChangePreview { statements, script })
}

// ---- commit -----------------------------------------------------------------------

/// What one planned statement is, for the report: only updates and deletes can "match nothing".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannedKind {
    Update,
    Insert,
    Delete,
    Ddl,
}

#[derive(Debug, Clone)]
pub struct PlannedStatement {
    pub id: String,
    pub sql: String,
    pub kind: PlannedKind,
}

// WHAT:  Everything Commit will run, decided before the guard sees it.
// HOW:   `returning` appends `RETURNING *` to updates and inserts on engines that
//        have it, so the grid can patch the changed rows without re-reading them.
#[derive(Debug, Clone)]
pub struct CommitPlan {
    pub engine: Engine,
    pub items: Vec<PlannedStatement>,
    pub returning: bool,
    /// The statements as one script, for the statement guard (classification, history).
    pub script: String,
}

impl CommitPlan {
    fn run_sql(&self, item: &PlannedStatement) -> String {
        if self.returning && matches!(item.kind, PlannedKind::Update | PlannedKind::Insert) {
            format!("{} RETURNING *", item.sql)
        } else {
            item.sql.clone()
        }
    }

    /// Whether the commit is all-or-nothing on this engine.
    pub fn atomic(&self) -> bool {
        is_atomic(self.engine, &self.items)
    }
}

fn is_atomic(engine: Engine, items: &[PlannedStatement]) -> bool {
    let only_ddl = items.iter().all(|i| i.kind == PlannedKind::Ddl);
    if only_ddl && edit_dialect(engine).is_none() {
        return false;
    }
    matches!(edit_dialect(engine), Some(EditDialect::Sql)) && script_parts(engine).is_some()
}

/// Engines whose statements can hand back the rows they wrote.
/// The Postgres family covers wire-compatible products; the ones that speak the protocol
/// without the `RETURNING` clause are named out.
fn supports_returning(engine: Engine) -> bool {
    if matches!(engine, Engine::Questdb | Engine::IbmIms | Engine::RaimaRdm) {
        return false;
    }
    matches!(engine.family(), Family::Postgres | Family::Sqlite | Family::Duckdb | Family::Libsql)
}

pub fn plan(engine: Engine, changes: &[StagedChange]) -> AppResult<CommitPlan> {
    if changes.is_empty() {
        return Err(AppError::invalid_input("There is nothing to commit."));
    }
    let mut items = Vec::with_capacity(changes.len());
    for change in changes {
        let kind = match change {
            StagedChange::Update { .. } => PlannedKind::Update,
            StagedChange::Insert { .. } => PlannedKind::Insert,
            StagedChange::Delete { .. } => PlannedKind::Delete,
            StagedChange::Ddl { .. } => PlannedKind::Ddl,
        };
        items.push(PlannedStatement { id: change.id().to_string(), sql: statement(engine, change)?, kind });
    }
    let script = items.iter().map(|i| format!("{};", i.sql)).collect::<Vec<_>>().join("\n");
    let returning = edit_dialect(engine) == Some(EditDialect::Sql) && supports_returning(engine);
    Ok(CommitPlan { engine, items, returning, script })
}

// WHAT:  The block's destructive gate for staged statements that declare themselves.
// WHY:   The guard classifies SQL text, but a Redis `FLUSHDB` or an engine-specific
//        `DROP`-like command can read as a plain write. A change that says it is
//        destructive needs the user's explicit yes either way.
pub fn require_confirmation(changes: &[StagedChange], confirmed: bool) -> AppResult<()> {
    if confirmed {
        return Ok(());
    }
    let statements: Vec<String> = changes
        .iter()
        .filter_map(|c| match c {
            StagedChange::Ddl { destructive: true, description, statement, .. } => {
                // The text the commit will run, so the dialog and the statement agree.
                let text = statement.trim().trim_end_matches(';').trim_end();
                Some(if description.trim().is_empty() { text.to_string() } else { format!("{} — {}", text, description.trim()) })
            }
            _ => None,
        })
        .collect();
    if statements.is_empty() {
        return Ok(());
    }
    Err(AppError::DestructiveConfirmationRequired { message: "These changes include destructive statements.".to_string(), statements })
}

/// Rows a RETURNING statement may hand back; one keyed row in practice.
const RETURNED_ROW_CAP: usize = 1_000;

/// Stand-in for "the engine said nothing about this statement": an empty, truncated set.
fn unknown_result() -> StatementResult {
    StatementResult::Rows { result: ResultSet { columns: Vec::new(), rows: Vec::new(), truncated: true } }
}

fn is_unknown(result: &ResultSet) -> bool {
    result.truncated && result.columns.is_empty() && result.rows.is_empty()
}

// WHAT:  Runs a commit plan so it either lands whole or not at all.
// WHY:   `BEGIN; …; COMMIT;` sent as one script left the transaction open when a
//        statement failed, and a pooled session could hand that connection to the
//        next caller. The adapter's transaction API pins one connection for BEGIN,
//        every change and COMMIT, and rolls back on the first failure.
// HOW:   * adapter has manual transactions, none open: BEGIN → each change → COMMIT,
//          ROLLBACK on any error;
//        * one is already open (the editor's Manual mode): the changes join it under
//          a SAVEPOINT, so a failure undoes only the commit;
//        * transactional engines without the API run a wrapped script and roll back
//          on error;
//        * the rest (ClickHouse mutations, DynamoDB) run one by one — `atomic` is false.
//        `statements` of the result line up with `plan.items`.
// WHERE: src-tauri/src/commands/changes.rs, src-tauri/src/integrations/mod.rs (transaction API)
pub async fn commit(ctx: &SessionCtx, plan: &CommitPlan) -> AppResult<QueryOutcome> {
    let it = &ctx.integration;
    let sql_dialect = edit_dialect(plan.engine) == Some(EditDialect::Sql);
    let statements = if sql_dialect && it.manual_transactions() {
        if it.in_transaction() {
            run_in_savepoint(ctx, plan).await?
        } else {
            run_in_new_transaction(ctx, plan).await?
        }
    } else if let (true, Some(parts)) = (plan.atomic(), script_parts(plan.engine)) {
        run_as_script(ctx, plan, parts).await?
    } else {
        run_each(ctx, plan).await?
    };
    Ok(QueryOutcome { statements, total_rows: None, elapsed_ms: ctx.elapsed_ms() })
}

async fn run_in_new_transaction(ctx: &SessionCtx, plan: &CommitPlan) -> AppResult<Vec<StatementResult>> {
    let it = &ctx.integration;
    it.begin_transaction().await?;
    match run_each(ctx, plan).await {
        Ok(results) => {
            // A failed COMMIT still ends the transaction (see `commit_transaction`).
            it.commit_transaction().await?;
            Ok(results)
        }
        Err(err) => {
            if let Err(rollback) = it.rollback_transaction().await {
                log::warn!("rollback after a failed commit also failed: {rollback}");
            }
            Err(err)
        }
    }
}

const SAVEPOINT: &str = "db_free_commit";

async fn run_in_savepoint(ctx: &SessionCtx, plan: &CommitPlan) -> AppResult<Vec<StatementResult>> {
    let it = &ctx.integration;
    it.execute(&format!("SAVEPOINT {SAVEPOINT}"), 0).await?;
    match run_each(ctx, plan).await {
        Ok(results) => {
            it.execute(&format!("RELEASE SAVEPOINT {SAVEPOINT}"), 0).await?;
            Ok(results)
        }
        Err(err) => {
            if let Err(rollback) = it.execute(&format!("ROLLBACK TO SAVEPOINT {SAVEPOINT}"), 0).await {
                log::warn!("rollback to savepoint after a failed commit also failed: {rollback}");
            }
            Err(err)
        }
    }
}

async fn run_as_script(ctx: &SessionCtx, plan: &CommitPlan, (before, commit, rollback): ScriptParts) -> AppResult<Vec<StatementResult>> {
    let it = &ctx.integration;
    let mut script = String::new();
    for s in before {
        script.push_str(s);
        script.push_str(";\n");
    }
    for item in &plan.items {
        script.push_str(&plan.run_sql(item));
        script.push_str(";\n");
    }
    script.push_str(commit);
    script.push(';');
    match it.execute(&script, RETURNED_ROW_CAP).await {
        Ok(mut results) => {
            let wrapped = before.len() + plan.items.len() + 1;
            if results.len() == wrapped {
                results.drain(..before.len());
                results.truncate(plan.items.len());
                Ok(results)
            } else if results.len() == plan.items.len() {
                Ok(results)
            } else {
                // The adapter folded or skipped some results; positions cannot be trusted.
                Ok(plan.items.iter().map(|_| unknown_result()).collect())
            }
        }
        Err(err) => {
            if let Err(undo) = it.execute(rollback, 0).await {
                log::warn!("rollback after a failed commit also failed: {undo}");
            }
            Err(err)
        }
    }
}

async fn run_each(ctx: &SessionCtx, plan: &CommitPlan) -> AppResult<Vec<StatementResult>> {
    let mut out = Vec::with_capacity(plan.items.len());
    for (index, item) in plan.items.iter().enumerate() {
        match ctx.integration.execute(&plan.run_sql(item), RETURNED_ROW_CAP).await {
            Ok(results) => out.push(results.into_iter().next().unwrap_or_else(unknown_result)),
            Err(mut err) => {
                if let AppError::Driver { message, .. } = &mut err {
                    *message = format!("Change {} of {} failed: {message}", index + 1, plan.items.len());
                }
                return Err(err);
            }
        }
    }
    Ok(out)
}

// WHAT:  Turns the outcome of `commit` into the per-change report.
// HOW:   An update or delete that touched no row is flagged `matched_nothing` — but
//        only where the engine really counts rows (SQL; ClickHouse mutations and
//        DynamoDB do not).
pub fn report(plan: &CommitPlan, outcome: &QueryOutcome) -> CommitReport {
    let counts = edit_dialect(plan.engine) == Some(EditDialect::Sql);
    let results = plan
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let (rows_affected, returned) = match outcome.statements.get(index) {
                Some(StatementResult::Affected { rows_affected }) if counts => (Some(*rows_affected), None),
                Some(StatementResult::Rows { result }) if counts && !is_unknown(result) => {
                    let returned = (plan.returning && matches!(item.kind, PlannedKind::Update | PlannedKind::Insert)).then(|| ReturnedRows {
                        columns: result.columns.iter().map(|c| c.name.clone()).collect(),
                        rows: result.rows.clone(),
                    });
                    (Some(result.rows.len() as u64), returned)
                }
                _ => (None, None),
            };
            let matched_nothing = matches!(item.kind, PlannedKind::Update | PlannedKind::Delete) && rows_affected == Some(0);
            ChangeResult { id: item.id.clone(), statement: item.sql.clone(), rows_affected, matched_nothing, returned }
        })
        .collect();
    CommitReport { results, atomic: plan.atomic(), elapsed_ms: outcome.elapsed_ms }
}

// ---- rows as SQL --------------------------------------------------------------------

// WHAT:  Rows written out as SELECT / INSERT / UPDATE / DELETE statements in the
//        engine's own dialect (identifier quoting, string escaping, binary form).
// WHY:   Copying a row to another environment should paste and run, and the same
//        literal rules that build Commit's SQL are the ones that know each dialect.
// HOW:   Select / Update / Delete address rows by `key_columns`; Insert writes every
//        column. One statement per row, except a Select over several rows which
//        reads them with one `IN (…)` (single key) or an OR of key tuples.
// WHERE: src-tauri/src/commands/changes.rs (`rows_as_sql`), src/features/grid/RecordInspector.tsx
pub fn rows_as_sql(
    engine: Engine,
    table: &TableRef,
    kind: RowSqlKind,
    columns: &[String],
    key_columns: &[String],
    rows: &[Vec<Value>],
) -> AppResult<String> {
    if !matches!(edit_dialect(engine), Some(EditDialect::Sql | EditDialect::ClickhouseMutations)) {
        return Err(AppError::invalid_input("SQL statements are not available for this engine."));
    }
    if rows.is_empty() {
        return Err(AppError::invalid_input("There are no rows to write."));
    }
    let target = qualified_name_for(engine, table);
    let ident = |name: &str| quote_ident_for(engine, name);
    let cell = |row: &[Value], column: &str| -> Value {
        columns.iter().position(|c| c == column).and_then(|i| row.get(i)).cloned().unwrap_or(Value::Null)
    };
    let predicate = |row: &[Value]| -> AppResult<String> {
        if key_columns.is_empty() {
            return Err(AppError::invalid_input("This table has no primary key, so rows cannot be addressed by key."));
        }
        Ok(key_columns
            .iter()
            .map(|k| match cell(row, k) {
                Value::Null => format!("{} IS NULL", ident(k)),
                value => format!("{} = {}", ident(k), literal(engine, &value)),
            })
            .collect::<Vec<_>>()
            .join(" AND "))
    };
    let mut out = Vec::new();
    match kind {
        RowSqlKind::Select => {
            if let ([only], true) = (key_columns, rows.len() > 1) {
                let values = rows.iter().map(|r| literal(engine, &cell(r, only))).collect::<Vec<_>>().join(", ");
                out.push(format!("SELECT * FROM {target} WHERE {} IN ({values});", ident(only)));
            } else if rows.len() > 1 {
                let groups = rows.iter().map(|r| predicate(r).map(|p| format!("({p})"))).collect::<AppResult<Vec<_>>>()?;
                out.push(format!("SELECT * FROM {target} WHERE {};", groups.join(" OR ")));
            } else if let Some(row) = rows.first() {
                out.push(format!("SELECT * FROM {target} WHERE {};", predicate(row)?));
            }
        }
        RowSqlKind::Insert => {
            let names = columns.iter().map(|c| ident(c)).collect::<Vec<_>>().join(", ");
            for row in rows {
                let values = columns.iter().map(|c| literal(engine, &cell(row, c))).collect::<Vec<_>>().join(", ");
                out.push(format!("INSERT INTO {target} ({names}) VALUES ({values});"));
            }
        }
        RowSqlKind::Update => {
            let settable: Vec<&String> = columns.iter().filter(|c| !key_columns.contains(c)).collect();
            if settable.is_empty() {
                return Err(AppError::invalid_input("Every column is part of the key, so there is nothing to update."));
            }
            for row in rows {
                let sets = settable.iter().map(|c| format!("{} = {}", ident(c), literal(engine, &cell(row, c)))).collect::<Vec<_>>().join(", ");
                out.push(format!("UPDATE {target} SET {sets} WHERE {};", predicate(row)?));
            }
        }
        RowSqlKind::Delete => {
            for row in rows {
                out.push(format!("DELETE FROM {target} WHERE {};", predicate(row)?));
            }
        }
    }
    Ok(out.join("\n"))
}

// WHAT:  Multi-row INSERT batches for imports.
pub fn insert_batches(engine: Engine, table: &TableRef, columns: &[String], rows: &[Vec<Value>], batch: usize) -> Vec<String> {
    let cols = columns.iter().map(|c| quote_ident_for(engine, c)).collect::<Vec<_>>().join(", ");
    rows.chunks(batch.max(1))
        .map(|chunk| {
            let values = chunk
                .iter()
                .map(|r| format!("({})", r.iter().map(|v| literal(engine, v)).collect::<Vec<_>>().join(", ")))
                .collect::<Vec<_>>()
                .join(",\n");
            format!("INSERT INTO {} ({}) VALUES\n{}", qualified_name_for(engine, table), cols, values)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ColumnMeta;

    fn t() -> TableRef {
        TableRef { schema: Some("public".into()), name: "users".into() }
    }

    fn cv(column: &str, value: Value) -> CellValue {
        CellValue { column: column.into(), value, raw: None }
    }

    fn update(id: &str, column: &str, old: Value, new: Value) -> StagedChange {
        StagedChange::Update { id: id.into(), table: t(), key: vec![cv("id", Value::Int(7))], column: column.into(), old, new, new_raw: None }
    }

    #[test]
    fn literals_per_engine() {
        assert_eq!(literal(Engine::Postgres, &Value::Text("it's".into())), "'it''s'");
        assert_eq!(literal(Engine::Sqlite, &Value::Bool(true)), "1");
        assert_eq!(literal(Engine::Postgres, &Value::Bool(false)), "FALSE");
        assert_eq!(literal(Engine::Postgres, &Value::Bytes("aGk=".into())), "'\\x6869'::bytea");
        assert_eq!(literal(Engine::Mysql, &Value::Bytes("aGk=".into())), "X'6869'");
        assert_eq!(literal(Engine::Postgres, &Value::Float(f64::NAN)), "NULL");
        assert_eq!(literal(Engine::Postgres, &Value::Json(serde_json::json!({"a": 1}))), "'{\"a\":1}'");
    }

    #[test]
    fn backslash_engines_cannot_be_broken_out_of() {
        // `\'` ends a MySQL literal early unless the backslash is escaped as well.
        assert_eq!(literal(Engine::Mysql, &Value::Text("a\\'b".into())), "'a\\\\\\'b'");
        assert_eq!(literal(Engine::Clickhouse, &Value::Text("C:\\x".into())), "'C:\\\\x'");
        assert_eq!(literal(Engine::Postgres, &Value::Text("C:\\x".into())), "'C:\\x'");
    }

    #[test]
    fn update_insert_delete_scripts() {
        let key = vec![cv("id", Value::Int(7))];
        let changes = vec![
            update("1", "name", Value::Text("a".into()), Value::Text("b".into())),
            StagedChange::Insert { id: "2".into(), table: t(), values: vec![cv("name", Value::Text("c".into())), cv("n", Value::Null)] },
            StagedChange::Delete { id: "3".into(), table: t(), key },
        ];
        let p = preview(Engine::Postgres, &changes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(p.statements.first().map(String::as_str), Some("UPDATE \"public\".\"users\" SET \"name\" = 'b' WHERE \"id\" = 7"));
        assert_eq!(p.statements.get(1).map(String::as_str), Some("INSERT INTO \"public\".\"users\" (\"name\", \"n\") VALUES ('c', NULL)"));
        assert_eq!(p.statements.get(2).map(String::as_str), Some("DELETE FROM \"public\".\"users\" WHERE \"id\" = 7"));
        assert!(p.script.starts_with("BEGIN;\n") && p.script.ends_with("COMMIT;"));
        let my = preview(Engine::Mysql, &changes).unwrap_or_else(|e| panic!("{e}"));
        assert!(my.statements.first().is_some_and(|s| s.contains("`public`.`users`")));
        let ms = preview(Engine::Mssql, &changes).unwrap_or_else(|e| panic!("{e}"));
        assert!(ms.script.starts_with("SET XACT_ABORT ON;\nBEGIN TRANSACTION;\n") && ms.script.ends_with("COMMIT TRANSACTION;"));
        let ora = preview(Engine::Oracle, &changes).unwrap_or_else(|e| panic!("{e}"));
        assert!(!ora.script.contains("BEGIN") && ora.script.ends_with("COMMIT;"));
    }

    #[test]
    fn clickhouse_rows_change_through_mutations() {
        let changes = vec![
            update("1", "name", Value::Null, Value::Text("b".into())),
            StagedChange::Delete { id: "3".into(), table: t(), key: vec![cv("id", Value::Int(7))] },
        ];
        let p = preview(Engine::Clickhouse, &changes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(p.statements.first().map(String::as_str), Some("ALTER TABLE \"public\".\"users\" UPDATE \"name\" = 'b' WHERE \"id\" = 7"));
        assert_eq!(p.statements.get(1).map(String::as_str), Some("ALTER TABLE \"public\".\"users\" DELETE WHERE \"id\" = 7"));
        assert!(!p.script.contains("BEGIN"), "mutations cannot be wrapped in a transaction");
    }

    #[test]
    fn dynamodb_rows_change_through_partiql() {
        let table = TableRef { schema: None, name: "Music".into() };
        let key = vec![cv("Artist", Value::Text("Acme's".into())), cv("Year", Value::Int(1999))];
        let changes = vec![
            StagedChange::Update { id: "1".into(), table: table.clone(), key: key.clone(), column: "Plays".into(), old: Value::Null, new: Value::Int(5), new_raw: None },
            StagedChange::Insert {
                id: "2".into(),
                table: table.clone(),
                values: vec![cv("Artist", Value::Text("X".into())), cv("Tags", Value::Json(serde_json::json!(["a", {"b": true}])))],
            },
            StagedChange::Delete { id: "3".into(), table, key },
        ];
        let p = preview(Engine::Dynamodb, &changes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(p.statements.first().map(String::as_str), Some("UPDATE \"Music\" SET \"Plays\" = 5 WHERE \"Artist\" = 'Acme''s' AND \"Year\" = 1999"));
        assert_eq!(p.statements.get(1).map(String::as_str), Some("INSERT INTO \"Music\" VALUE {'Artist': 'X', 'Tags': ['a', {'b': true}]}"));
        assert_eq!(p.statements.get(2).map(String::as_str), Some("DELETE FROM \"Music\" WHERE \"Artist\" = 'Acme''s' AND \"Year\" = 1999"));
        assert!(!p.script.contains("BEGIN"));
        let null_key = StagedChange::Delete { id: "x".into(), table: TableRef { schema: None, name: "Music".into() }, key: vec![cv("Artist", Value::Null)] };
        assert!(statement(Engine::Dynamodb, &null_key).is_err());
    }

    #[test]
    fn sql_tokens_are_rendered_unquoted_per_engine() {
        let mut change = update("1", "created", Value::Null, Value::Text("now()".into()));
        if let StagedChange::Update { new_raw, .. } = &mut change {
            *new_raw = Some(SqlToken::Now);
        }
        assert_eq!(statement(Engine::Postgres, &change).ok().as_deref(), Some("UPDATE \"public\".\"users\" SET \"created\" = now() WHERE \"id\" = 7"));
        assert!(statement(Engine::Mysql, &change).is_ok_and(|s| s.contains("= NOW()")));
        assert!(statement(Engine::Sqlite, &change).is_ok_and(|s| s.contains("= CURRENT_TIMESTAMP")));
        assert!(statement(Engine::Mssql, &change).is_ok_and(|s| s.contains("= SYSDATETIME()")));
        assert_eq!(token_sql(Engine::Postgres, SqlToken::CurrentDate).ok().as_deref(), Some("CURRENT_DATE"));
    }

    #[test]
    fn default_token_sets_default_and_is_left_out_of_an_insert() {
        let mut change = update("1", "role", Value::Text("a".into()), Value::Null);
        if let StagedChange::Update { new_raw, .. } = &mut change {
            *new_raw = Some(SqlToken::Default);
        }
        assert!(statement(Engine::Postgres, &change).is_ok_and(|s| s.contains("\"role\" = DEFAULT")));
        assert!(statement(Engine::Sqlite, &change).is_err(), "SQLite has no DEFAULT keyword to set");
        let insert = StagedChange::Insert {
            id: "i".into(),
            table: t(),
            values: vec![cv("name", Value::Text("n".into())), CellValue { column: "role".into(), value: Value::Null, raw: Some(SqlToken::Default) }],
        };
        assert_eq!(statement(Engine::Sqlite, &insert).ok().as_deref(), Some("INSERT INTO \"public\".\"users\" (\"name\") VALUES ('n')"));
        let all_default = StagedChange::Insert { id: "d".into(), table: t(), values: vec![CellValue { column: "role".into(), value: Value::Null, raw: Some(SqlToken::Default) }] };
        assert_eq!(statement(Engine::Postgres, &all_default).ok().as_deref(), Some("INSERT INTO \"public\".\"users\" DEFAULT VALUES"));
        assert_eq!(statement(Engine::Mysql, &all_default).ok().as_deref(), Some("INSERT INTO `public`.`users` () VALUES ()"));
    }

    #[test]
    fn missing_key_is_rejected() {
        let err = preview(Engine::Sqlite, &[StagedChange::Delete { id: "x".into(), table: t(), key: vec![] }]).err();
        assert!(matches!(err, Some(AppError::InvalidInput { .. })));
        assert!(preview(Engine::Redis, &[update("1", "a", Value::Null, Value::Null)]).is_err());
    }

    #[test]
    fn statements_pass_through_for_any_engine_and_destructive_ones_need_a_yes() {
        let drop = StagedChange::Ddl { id: "d".into(), table: t(), statement: "DROP INDEX idx;  ".into(), description: "Drop idx".into(), destructive: true };
        let flush = StagedChange::Ddl { id: "f".into(), table: t(), statement: "DEL cache:1".into(), description: String::new(), destructive: false };
        assert_eq!(statement(Engine::Redis, &flush).ok().as_deref(), Some("DEL cache:1"));
        assert_eq!(statement(Engine::Postgres, &drop).ok().as_deref(), Some("DROP INDEX idx"));
        match require_confirmation(std::slice::from_ref(&drop), false) {
            Err(AppError::DestructiveConfirmationRequired { statements, .. }) => assert_eq!(statements, vec!["DROP INDEX idx — Drop idx".to_string()]),
            other => panic!("expected a confirmation request, got {other:?}"),
        }
        assert!(require_confirmation(&[drop], true).is_ok());
        assert!(require_confirmation(std::slice::from_ref(&flush), false).is_ok());
        let p = plan(Engine::Redis, &[flush]).unwrap_or_else(|e| panic!("{e}"));
        assert!(!p.atomic() && !p.returning);
    }

    #[test]
    fn returning_is_asked_for_only_where_it_exists() {
        let changes = [update("1", "name", Value::Null, Value::Text("b".into())), StagedChange::Delete { id: "2".into(), table: t(), key: vec![cv("id", Value::Int(1))] }];
        let pg = plan(Engine::Postgres, &changes).unwrap_or_else(|e| panic!("{e}"));
        assert!(pg.returning && pg.atomic());
        assert!(pg.run_sql(&pg.items[0]).ends_with(" RETURNING *"));
        assert!(!pg.run_sql(&pg.items[1]).contains("RETURNING"), "a delete has nothing to hand back");
        let questdb = plan(Engine::Questdb, &changes).unwrap_or_else(|e| panic!("{e}"));
        assert!(!questdb.returning, "QuestDB speaks the Postgres protocol without RETURNING");
        let my = plan(Engine::Mysql, &changes).unwrap_or_else(|e| panic!("{e}"));
        assert!(!my.returning && my.atomic());
        let ch = plan(Engine::Clickhouse, &changes).unwrap_or_else(|e| panic!("{e}"));
        assert!(!ch.returning && !ch.atomic());
    }

    #[test]
    fn report_flags_changes_that_matched_nothing() {
        let changes = [update("u1", "name", Value::Null, Value::Text("b".into())), update("u2", "name", Value::Null, Value::Text("c".into())), StagedChange::Delete { id: "d".into(), table: t(), key: vec![cv("id", Value::Int(1))] }];
        let mysql = plan(Engine::Mysql, &changes).unwrap_or_else(|e| panic!("{e}"));
        let outcome = QueryOutcome {
            statements: vec![
                StatementResult::Affected { rows_affected: 1 },
                StatementResult::Affected { rows_affected: 0 },
                StatementResult::Affected { rows_affected: 0 },
            ],
            total_rows: None,
            elapsed_ms: 3,
        };
        let r = report(&mysql, &outcome);
        assert_eq!(r.results.iter().map(|c| c.matched_nothing).collect::<Vec<_>>(), vec![false, true, true]);
        assert!(r.atomic);

        let pg = plan(Engine::Postgres, &changes[..2]).unwrap_or_else(|e| panic!("{e}"));
        let returned = StatementResult::Rows {
            result: ResultSet {
                columns: vec![ColumnMeta { name: "id".into(), type_name: "int4".into() }],
                rows: vec![vec![Value::Int(7)]],
                truncated: false,
            },
        };
        let empty = StatementResult::Rows { result: ResultSet { columns: Vec::new(), rows: Vec::new(), truncated: false } };
        let r = report(&pg, &QueryOutcome { statements: vec![returned, empty], total_rows: None, elapsed_ms: 0 });
        assert_eq!(r.results[0].returned.as_ref().map(|x| x.rows.len()), Some(1));
        assert!(!r.results[0].matched_nothing && r.results[1].matched_nothing);

        // Counts nobody reported are not "zero rows".
        let ch = plan(Engine::Clickhouse, &changes[..1]).unwrap_or_else(|e| panic!("{e}"));
        let r = report(&ch, &QueryOutcome { statements: vec![StatementResult::Affected { rows_affected: 0 }], total_rows: None, elapsed_ms: 0 });
        assert!(!r.results[0].matched_nothing && r.results[0].rows_affected.is_none());
        let r = report(&mysql, &QueryOutcome { statements: vec![unknown_result(); 3], total_rows: None, elapsed_ms: 0 });
        assert!(r.results.iter().all(|c| !c.matched_nothing && c.rows_affected.is_none()));
    }

    #[test]
    fn rows_become_dialect_correct_statements() {
        let columns = vec!["id".to_string(), "name".to_string()];
        let key = vec!["id".to_string()];
        let rows = vec![vec![Value::Int(1), Value::Text("a'b".into())], vec![Value::Int(2), Value::Null]];
        let table = TableRef { schema: None, name: "t".into() };
        let sql = |engine, kind| rows_as_sql(engine, &table, kind, &columns, &key, &rows).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(sql(Engine::Sqlite, RowSqlKind::Insert), "INSERT INTO \"t\" (\"id\", \"name\") VALUES (1, 'a''b');\nINSERT INTO \"t\" (\"id\", \"name\") VALUES (2, NULL);");
        assert_eq!(sql(Engine::Mysql, RowSqlKind::Insert).lines().next(), Some("INSERT INTO `t` (`id`, `name`) VALUES (1, 'a\\'b');"));
        assert_eq!(sql(Engine::Postgres, RowSqlKind::Select), "SELECT * FROM \"t\" WHERE \"id\" IN (1, 2);");
        assert_eq!(sql(Engine::Postgres, RowSqlKind::Update).lines().next(), Some("UPDATE \"t\" SET \"name\" = 'a''b' WHERE \"id\" = 1;"));
        assert_eq!(sql(Engine::Postgres, RowSqlKind::Delete).lines().nth(1), Some("DELETE FROM \"t\" WHERE \"id\" = 2;"));
        let composite = vec!["id".to_string(), "name".to_string()];
        let tuples = rows_as_sql(Engine::Postgres, &table, RowSqlKind::Select, &columns, &composite, &rows).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(tuples, "SELECT * FROM \"t\" WHERE (\"id\" = 1 AND \"name\" = 'a''b') OR (\"id\" = 2 AND \"name\" IS NULL);");
        assert!(rows_as_sql(Engine::Postgres, &table, RowSqlKind::Delete, &columns, &[], &rows).is_err(), "no key, no safe delete");
        assert!(rows_as_sql(Engine::Redis, &table, RowSqlKind::Insert, &columns, &key, &rows).is_err());
    }

    // ---- Commit against a live SQLite file -------------------------------------

    async fn sqlite_ctx(setup: &str) -> SessionCtx {
        use crate::model::{ConnectionInput, ConnectionSummary, Environment, ResolvedConnection, SslMode};
        let dir = std::env::temp_dir().join(format!("db-free-changes-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let input = ConnectionInput {
            name: "commit".into(),
            engine: Engine::Sqlite,
            environment: Environment::Local,
            read_only: false,
            host: None,
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: Some(dir.join("c.db").to_string_lossy().into_owned()),
            ssl_mode: SslMode::Disable,
            ssh: crate::model::SshTunnel::default(),
            ssh_secret: None,
            folder: None,
            color: None,
            favorite: false,
            auth: Default::default(),
            auth_secrets: Default::default(),
            extra: Default::default(),
        };
        let summary = ConnectionSummary::draft(&input, false);
        let resolved = ResolvedConnection { summary: summary.clone(), secret: None };
        let integration = crate::integrations::connect(&resolved).await.unwrap_or_else(|e| panic!("{e}"));
        integration.execute(setup, 10).await.unwrap_or_else(|e| panic!("{e}"));
        SessionCtx { connection: summary, integration, started: std::time::Instant::now() }
    }

    fn users() -> TableRef {
        TableRef { schema: None, name: "users".into() }
    }

    fn rename(id: &str, row: i64, to: &str) -> StagedChange {
        StagedChange::Update { id: id.into(), table: users(), key: vec![cv("id", Value::Int(row))], column: "name".into(), old: Value::Null, new: Value::Text(to.into()), new_raw: None }
    }

    async fn names(ctx: &SessionCtx) -> Vec<String> {
        let results = ctx.integration.execute("SELECT name FROM users ORDER BY id", 100).await.unwrap_or_else(|e| panic!("{e}"));
        match results.into_iter().next() {
            Some(StatementResult::Rows { result }) => result
                .rows
                .iter()
                .map(|row| match row.first() {
                    Some(Value::Text(t)) => t.clone(),
                    other => format!("{other:?}"),
                })
                .collect(),
            other => panic!("expected rows, got {other:?}"),
        }
    }

    const SEED: &str = "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL); INSERT INTO users VALUES (1, 'ann'), (2, 'bob');";

    #[tokio::test]
    async fn a_commit_lands_whole_and_hands_back_the_changed_rows() {
        let ctx = sqlite_ctx(SEED).await;
        let changes = vec![rename("u", 1, "amy"), StagedChange::Insert { id: "i".into(), table: users(), values: vec![cv("name", Value::Text("cy".into()))] }];
        let planned = plan(Engine::Sqlite, &changes).unwrap_or_else(|e| panic!("{e}"));
        let outcome = commit(&ctx, &planned).await.unwrap_or_else(|e| panic!("{e}"));
        let done = report(&planned, &outcome);
        assert!(done.atomic);
        assert_eq!(names(&ctx).await, vec!["amy", "bob", "cy"]);
        assert!(!done.results.iter().any(|r| r.matched_nothing));
        let returned = done.results.first().and_then(|r| r.returned.as_ref());
        assert_eq!(returned.map(|r| r.rows.len()), Some(1), "the updated row comes back, so the grid need not re-read it");
        assert!(returned.is_some_and(|r| r.columns == vec!["id".to_string(), "name".to_string()] && r.rows[0][1] == Value::Text("amy".into())));
        assert!(!ctx.integration.in_transaction());
    }

    #[tokio::test]
    async fn a_failing_change_undoes_the_whole_commit() {
        let ctx = sqlite_ctx(SEED).await;
        // The second change breaks NOT NULL: the first must not survive it, and no transaction stays open.
        let bad = StagedChange::Insert { id: "bad".into(), table: users(), values: vec![cv("name", Value::Null)] };
        let planned = plan(Engine::Sqlite, &[rename("u", 1, "zed"), bad]).unwrap_or_else(|e| panic!("{e}"));
        let err = commit(&ctx, &planned).await.err().map(|e| e.message().to_string());
        assert!(err.is_some_and(|m| m.contains("Change 2 of 2")), "the error says which change failed");
        assert_eq!(names(&ctx).await, vec!["ann", "bob"]);
        assert!(!ctx.integration.in_transaction());
        // The connection is usable straight away.
        let again = plan(Engine::Sqlite, &[rename("u", 2, "ben")]).unwrap_or_else(|e| panic!("{e}"));
        commit(&ctx, &again).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(names(&ctx).await, vec!["ann", "ben"]);
    }

    #[tokio::test]
    async fn a_change_that_matched_no_row_is_flagged_and_the_rest_still_land() {
        let ctx = sqlite_ctx(SEED).await;
        let planned = plan(Engine::Sqlite, &[rename("gone", 99, "nobody"), rename("here", 1, "amy")]).unwrap_or_else(|e| panic!("{e}"));
        let outcome = commit(&ctx, &planned).await.unwrap_or_else(|e| panic!("{e}"));
        let done = report(&planned, &outcome);
        assert_eq!(done.results.iter().map(|r| (r.id.as_str(), r.matched_nothing)).collect::<Vec<_>>(), vec![("gone", true), ("here", false)]);
        assert_eq!(names(&ctx).await, vec!["amy", "bob"]);
    }

    #[tokio::test]
    async fn a_commit_inside_the_users_own_transaction_joins_it() {
        let ctx = sqlite_ctx(SEED).await;
        ctx.integration.begin_transaction().await.unwrap_or_else(|e| panic!("{e}"));
        let bad = StagedChange::Insert { id: "bad".into(), table: users(), values: vec![cv("name", Value::Null)] };
        let planned = plan(Engine::Sqlite, &[rename("u", 1, "zed"), bad]).unwrap_or_else(|e| panic!("{e}"));
        assert!(commit(&ctx, &planned).await.is_err());
        // Only the commit was undone: the user's transaction is still open and still theirs to end.
        assert!(ctx.integration.in_transaction());
        assert_eq!(names(&ctx).await, vec!["ann", "bob"]);
        ctx.integration.rollback_transaction().await.unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn insert_batches_split() {
        let rows: Vec<Vec<Value>> = (0..5).map(|i| vec![Value::Int(i), Value::Text(format!("n{i}"))]).collect();
        let batches = insert_batches(Engine::Sqlite, &TableRef { schema: None, name: "t".into() }, &["id".into(), "name".into()], &rows, 2);
        assert_eq!(batches.len(), 3);
        assert!(batches.first().is_some_and(|b| b.starts_with("INSERT INTO \"t\" (\"id\", \"name\") VALUES\n(0, 'n0'),\n(1, 'n1')")));
    }
}
