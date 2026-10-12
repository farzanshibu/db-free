// SOT: transfer-commands, ipc-export, ipc-export-preview, ipc-import, ipc-import-preview, ipc-json-import, ipc-sql-import, transfer-progress-channel

use crate::error::{AppError, AppResult};
use crate::guard;
use crate::model::{
    CsvImportOptions, ExportLayout, ExportOptions, ExportPreview, ExportProgress, ExportReport, ImportConflict, ImportMapping,
    ImportOperationChoice, ImportOperationKind, ImportOperationResult, ImportPreview, ImportProgress, ImportReport,
    JsonImportPlan, JsonImportReport, QueryOutcome, SqlFileInfo, SqlImportProgress, SqlImportReport, StatementResult,
    TableRef, TransferFormat, Value,
};
use crate::services;
use crate::services::export::ExportJob;
use crate::services::sql_import::{self, SqlFile};
use crate::services::transfer::{self as transfer, ChunkPlan, ImportSource};
use crate::state::AppState;
use serde::Deserialize;
use std::path::Path;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::State;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

// ============================================================================
// TRANSFER COMMANDS
//
// WHAT:  The IPC surface of export and import: write tables to files, preview
//        what that writes, preview a file before importing it, import a CSV /
//        JSON file into a table, import a multi-table JSON file as a checklist,
//        and run a .sql script.
// WHY:   Each of these runs long (a million-row table, a gigabyte dump), so
//        the reply is a stream of progress frames on a Tauri channel plus one
//        final report, and the user can stop a run by its id (`cancel_query`).
// HOW:   Reads (export, previews) take their session from the block and then
//        run outside its 300 s request timeout: a long export is not a hung
//        request, and Stop is the bound. Writes go through `guard::statement`
//        one chunk (CSV / JSON) or one statement (.sql) at a time, so the
//        read-only lock, the destructive confirmation and the history log
//        apply to every one.
// WHERE: src-tauri/src/services/{export,transfer,sql_import}.rs,
//        src/features/transfer/TransferTab.tsx
// ============================================================================

/// Progress frames are at most this far apart.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);
const DEFAULT_EXPORT_ROWS: u64 = 1_000_000;
const PREVIEW_ROWS_PER_TABLE: u64 = 50;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportRequest {
    pub connection_id: String,
    pub tables: Vec<TableRef>,
    pub options: ExportOptions,
    pub layout: ExportLayout,
    /// A bundle: the file to write. A folder: the directory.
    pub path: String,
    pub max_rows: Option<u32>,
    /// Names this run so `cancel_query` can stop it.
    #[serde(default)]
    #[ts(optional)]
    pub run_id: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PreviewExportRequest {
    pub connection_id: String,
    pub tables: Vec<TableRef>,
    pub options: ExportOptions,
    /// Rows shown per table (default 50).
    pub rows_per_table: Option<u32>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PreviewImportRequest {
    pub connection_id: String,
    /// The target, once chosen: its columns come back with a suggested mapping.
    pub table: Option<TableRef>,
    pub path: String,
    pub format: TransferFormat,
    pub csv: CsvImportOptions,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportRequest {
    pub connection_id: String,
    pub table: TableRef,
    pub path: String,
    pub format: TransferFormat,
    pub csv: CsvImportOptions,
    /// One entry per target column; `source: null` skips it.
    pub mapping: Vec<ImportMapping>,
    /// Empty the table first (in the same transaction as the first chunk).
    pub truncate_first: bool,
    pub on_conflict: ImportConflict,
    /// The user confirmed the destructive part (truncate-first).
    pub confirm_destructive: bool,
    /// Rows in the file, from the preview, for the progress bar.
    pub rows_total: u64,
    #[serde(default)]
    #[ts(optional)]
    pub run_id: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JsonPlanRequest {
    pub connection_id: String,
    pub path: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JsonImportRequest {
    pub connection_id: String,
    pub path: String,
    /// The ticked checklist rows, in the order to run them.
    pub operations: Vec<ImportOperationChoice>,
    #[serde(default)]
    #[ts(optional)]
    pub run_id: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SqlFileRequest {
    pub connection_id: String,
    pub path: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SqlImportRequest {
    pub connection_id: String,
    pub path: String,
    /// The user confirmed the file's destructive statements.
    pub confirm_destructive: bool,
    /// Size and time of the file as inspected: a changed file is refused.
    pub expected_size: u64,
    pub expected_modified_ms: u64,
    #[serde(default)]
    #[ts(optional)]
    pub run_id: Option<String>,
}

// WHAT:  A run's Stop switch: registered under the run id, released on drop.
struct RunScope<'a> {
    state: &'a AppState,
    id: Option<String>,
    token: CancellationToken,
}

impl<'a> RunScope<'a> {
    fn new(state: &'a AppState, run_id: Option<&str>) -> RunScope<'a> {
        match run_id {
            Some(id) => RunScope { state, id: Some(id.to_string()), token: state.query_runs().register(id) },
            None => RunScope { state, id: None, token: CancellationToken::new() },
        }
    }
}

impl Drop for RunScope<'_> {
    fn drop(&mut self) {
        if let Some(id) = &self.id {
            self.state.query_runs().finish(id);
        }
    }
}

fn millis(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

// WHAT:  Writes the tables to a file or a folder, streaming progress.
#[tauri::command]
pub async fn export_tables(state: State<'_, AppState>, req: ExportRequest, on_progress: Channel<ExportProgress>) -> AppResult<ExportReport> {
    // The UI always passes a channel (ipcProgress); the body treats progress as optional.
    let on_progress = Some(on_progress);
    let scope = RunScope::new(&state, req.run_id.as_deref());
    let ctx = guard::session(&state, &req.connection_id, |ctx| async move { Ok(ctx) }).await?;
    let job = ExportJob {
        tables: &req.tables,
        options: &req.options,
        layout: req.layout,
        path: Path::new(&req.path),
        limit: req.max_rows.map_or(DEFAULT_EXPORT_ROWS, u64::from),
    };
    let mut tick = |frame: ExportProgress| {
        if let Some(channel) = &on_progress {
            let _ = channel.send(frame);
        }
    };
    services::export::export_tables(&ctx, &job, &scope.token, &mut tick).await
}

// WHAT:  The first rows of an export, formatted as the file will be.
#[tauri::command]
pub async fn preview_export(state: State<'_, AppState>, req: PreviewExportRequest) -> AppResult<ExportPreview> {
    let rows = req.rows_per_table.map_or(PREVIEW_ROWS_PER_TABLE, u64::from);
    guard::session(&state, &req.connection_id, |ctx| async move { services::export::preview(&ctx, &req.tables, &req.options, rows).await }).await
}

// WHAT:  A file's columns, its first 50 rows, and a suggested mapping.
#[tauri::command]
pub async fn preview_import(state: State<'_, AppState>, req: PreviewImportRequest) -> AppResult<ImportPreview> {
    guard::session(&state, &req.connection_id, |ctx| async move {
        services::transfer::preview(&ctx, req.table.as_ref(), Path::new(&req.path), req.format, &req.csv).await
    })
    .await
}

// The rows a chunk reports back: what the server said each INSERT changed.
fn affected(outcome: &QueryOutcome) -> Option<u64> {
    let mut total: u64 = 0;
    for statement in &outcome.statements {
        match statement {
            StatementResult::Affected { rows_affected } => total += rows_affected,
            StatementResult::Rows { .. } => return None,
        }
    }
    Some(total)
}

// WHAT:  One chunk through the block: the guard classifies and logs `summary`,
//        the adapter runs `script`.
async fn run_chunk(state: &AppState, connection_id: &str, summary: &str, script: String, confirm: bool) -> AppResult<QueryOutcome> {
    guard::statement(
        state,
        guard::StatementRequest { connection_id, sql: summary, confirm_destructive: confirm, run_id: None },
        |ctx| async move { services::query::execute(&ctx, &script, 10, None).await },
    )
    .await
}

fn file_label(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

// WHAT:  Imports a CSV / JSON file into a table, a chunk at a time.
// WHY:   A chunk is one transaction, so the file is atomic up to
//        `IMPORT_CHUNK_ROWS` rows and a failure after that leaves the earlier
//        chunks committed — which the report says, with how far it got.
// HOW:   Truncate-first rides in the first chunk's transaction; the guard sees
//        it (its summary holds the TRUNCATE), so it needs `confirm_destructive`
//        and is refused on a read-only connection.
#[tauri::command]
pub async fn import_file(state: State<'_, AppState>, req: ImportRequest, on_progress: Channel<ImportProgress>) -> AppResult<ImportReport> {
    // The UI always passes a channel (ipcProgress); the body treats progress as optional.
    let on_progress = Some(on_progress);
    let started = Instant::now();
    let scope = RunScope::new(&state, req.run_id.as_deref());
    let table = req.table.clone();
    let target = guard::session(&state, &req.connection_id, |ctx| async move { services::transfer::load_target(&ctx, &table).await }).await?;
    let (path, format, csv) = (req.path.clone(), req.format, req.csv.clone());
    let mut source = tokio::task::spawn_blocking(move || ImportSource::open(Path::new(&path), format, &csv)).await.map_err(AppError::internal)??;
    let (columns, picks) = transfer::mapped_columns(&req.mapping, &target.columns, source.headers.len())?;
    let label = file_label(&req.path);
    let plan = ChunkPlan { engine: target.engine, table: &req.table, columns: &columns, keys: &target.keys, conflict: req.on_conflict, transactional: target.transactional };

    let mut rows_done: u64 = 0;
    let mut statements: u64 = 0;
    let mut reported: Option<u64> = Some(0);
    let mut error: Option<String> = None;
    let mut last_frame = Instant::now();
    let mut first = true;
    loop {
        if scope.token.is_cancelled() {
            error = Some("Cancelled by user".to_string());
            break;
        }
        let rows = match source.next_rows(transfer::IMPORT_CHUNK_ROWS) {
            Ok(rows) => rows,
            Err(err) if !first => {
                error = Some(err.message().to_string());
                break;
            }
            Err(err) => return Err(err),
        };
        if rows.is_empty() {
            break;
        }
        let count = rows.len() as u64;
        let chunk = transfer::chunk_script(&plan, &transfer::pick_columns(rows, &picks), first && req.truncate_first, &label)?;
        match run_chunk(&state, &req.connection_id, &chunk.summary, chunk.script, req.confirm_destructive).await {
            Ok(outcome) => {
                rows_done += count;
                statements += outcome.statements.len() as u64;
                reported = reported.zip(affected(&outcome)).map(|(a, b)| a + b);
            }
            // Nothing has changed yet: a refusal (read-only, unconfirmed
            // truncate) or a first-chunk failure is an error, not a report.
            Err(err) if first => return Err(err),
            Err(err) => {
                error = Some(err.message().to_string());
                break;
            }
        }
        first = false;
        if last_frame.elapsed() >= PROGRESS_EVERY {
            last_frame = Instant::now();
            if let Some(channel) = &on_progress {
                let _ = channel.send(ImportProgress { rows_done, rows_total: req.rows_total.max(rows_done) });
            }
        }
    }
    if let Some(channel) = &on_progress {
        let _ = channel.send(ImportProgress { rows_done, rows_total: req.rows_total.max(rows_done) });
    }
    Ok(ImportReport { rows_inserted: rows_done, rows_affected: reported, rows_total: req.rows_total.max(rows_done), statements, elapsed_ms: millis(started), error })
}

// WHAT:  Reads a JSON file: one table's rows, or the checklist of a multi-table file.
#[tauri::command]
pub async fn plan_json_import(state: State<'_, AppState>, req: JsonPlanRequest) -> AppResult<JsonImportPlan> {
    guard::session(&state, &req.connection_id, |ctx| async move { services::transfer::plan_json(&ctx, Path::new(&req.path)).await }).await
}

// WHAT:  Runs the ticked operations of a multi-table JSON file, in order.
// WHY:   Each operation succeeds or fails on its own (a CREATE for a table that
//        already exists must not stop the rest), so the result is a list, one
//        entry per operation, rather than one error.
// HOW:   A CREATE TABLE goes through the guard as the statement it is; an
//        INSERT as chunks, exactly like a CSV import. An INSERT for a table
//        whose CREATE failed is skipped with that reason.
#[tauri::command]
pub async fn run_json_import(state: State<'_, AppState>, req: JsonImportRequest) -> AppResult<JsonImportReport> {
    let started = Instant::now();
    let scope = RunScope::new(&state, req.run_id.as_deref());
    let path = req.path.clone();
    let parsed = tokio::task::spawn_blocking(move || transfer::read_tables(Path::new(&path))).await.map_err(AppError::internal)??;
    let (known, namespaces, engine) = guard::session(&state, &req.connection_id, |ctx| async move {
        let (known, namespaces) = services::transfer::known_tables(&ctx).await?;
        Ok((known, namespaces, ctx.connection.engine))
    })
    .await?;
    let plan = transfer::plan_operations(engine, namespaces, &parsed, &known);
    let label = file_label(&req.path);
    let mut results: Vec<ImportOperationResult> = Vec::new();
    let mut failed_creates: Vec<String> = Vec::new();

    for choice in &req.operations {
        if scope.token.is_cancelled() {
            results.push(ImportOperationResult { id: choice.id.clone(), ok: false, message: "Cancelled by user".to_string(), rows: 0 });
            break;
        }
        let Some(operation) = plan.iter().find(|op| op.id == choice.id) else {
            results.push(ImportOperationResult { id: choice.id.clone(), ok: false, message: "This operation is not in the file.".to_string(), rows: 0 });
            continue;
        };
        let Some(table) = parsed.tables.iter().find(|t| t.name == operation.table) else {
            results.push(ImportOperationResult { id: choice.id.clone(), ok: false, message: "The table is not in the file.".to_string(), rows: 0 });
            continue;
        };
        let wanted = choice.target.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(&operation.table);
        let target = transfer::resolve_target(namespaces, wanted, &known);
        let outcome = match operation.kind {
            ImportOperationKind::CreateTable => {
                let described = parsed.schema.iter().find(|(name, _)| name == &table.name).map(|(_, columns)| columns.clone()).unwrap_or_default();
                match transfer::create_table_sql(engine, &target, &described, &table.inferred) {
                    Ok(sql) => run_chunk(&state, &req.connection_id, &sql, sql.clone(), false).await.map(|_| (format!("Created {}", target.name), 0)),
                    Err(err) => Err(err),
                }
            }
            ImportOperationKind::Insert => {
                if failed_creates.contains(&operation.table) {
                    Err(AppError::invalid_input(format!("Skipped: creating {} failed.", target.name)))
                } else {
                    insert_rows(&state, &req.connection_id, &target, table, &label).await
                }
            }
        };
        match outcome {
            Ok((message, rows)) => results.push(ImportOperationResult { id: choice.id.clone(), ok: true, message, rows }),
            Err(err) => {
                if operation.kind == ImportOperationKind::CreateTable {
                    failed_creates.push(operation.table.clone());
                }
                results.push(ImportOperationResult { id: choice.id.clone(), ok: false, message: err.message().to_string(), rows: 0 });
            }
        }
    }
    Ok(JsonImportReport { results, elapsed_ms: millis(started) })
}

// WHAT:  One checklist INSERT: the file's rows into the target's matching columns.
async fn insert_rows(state: &AppState, connection_id: &str, target: &TableRef, table: &transfer::ParsedTable, label: &str) -> AppResult<(String, u64)> {
    let table_ref = target.clone();
    let loaded = guard::session(state, connection_id, |ctx| async move { services::transfer::load_target(&ctx, &table_ref).await }).await?;
    let mut names: Vec<String> = Vec::new();
    let mut picks: Vec<usize> = Vec::new();
    let mut ignored: Vec<&str> = Vec::new();
    for (index, column) in table.columns.iter().enumerate() {
        match loaded.columns.iter().find(|c| c.name == *column).or_else(|| loaded.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column))) {
            Some(found) if !names.contains(&found.name) => {
                names.push(found.name.clone());
                picks.push(index);
            }
            _ => ignored.push(column),
        }
    }
    if names.is_empty() {
        return Err(AppError::invalid_input(format!("None of the file's columns exist on {}.", target.name)));
    }
    let plan = ChunkPlan { engine: loaded.engine, table: target, columns: &names, keys: &loaded.keys, conflict: ImportConflict::Error, transactional: loaded.transactional };
    let mut done: u64 = 0;
    for chunk in table.rows.chunks(transfer::IMPORT_CHUNK_ROWS) {
        let rows: Vec<Vec<Value>> = transfer::pick_columns(chunk.to_vec(), &picks);
        let script = transfer::chunk_script(&plan, &rows, false, label)?;
        if let Err(err) = run_chunk(state, connection_id, &script.summary, script.script, false).await {
            return Err(AppError::invalid_input(format!("{} (after {done} row(s) were inserted)", err.message())));
        }
        done += chunk.len() as u64;
    }
    let note = if ignored.is_empty() { String::new() } else { format!("; ignored {}", ignored.join(", ")) };
    Ok((format!("Inserted {done} row(s) into {}{note}", target.name), done))
}

// WHAT:  Scans a .sql file with the run's own splitter: statement count, the
//        destructive ones, and the file's head.
#[tauri::command]
pub async fn inspect_sql_file(state: State<'_, AppState>, req: SqlFileRequest) -> AppResult<SqlFileInfo> {
    let engine = services::connection::engine_of(&state, &req.connection_id)?;
    guard::local("inspect_sql_file", async { sql_import::scan(Path::new(&req.path), engine).await }).await
}

// WHAT:  Runs a .sql file statement by statement, each through the block.
// WHY:   A dump has no wrapping transaction (DDL commits on most engines and a
//        gigabyte transaction helps nobody), so a failure stops the run and the
//        report says which statement, which line, and how many ran before it.
// HOW:   The file is read a chunk at a time; the guard classifies and logs each
//        statement. The file must be the one the user inspected and confirmed.
#[tauri::command]
pub async fn import_sql_file(state: State<'_, AppState>, req: SqlImportRequest, on_progress: Channel<SqlImportProgress>) -> AppResult<SqlImportReport> {
    // The UI always passes a channel (ipcProgress); the body treats progress as optional.
    let on_progress = Some(on_progress);
    let started = Instant::now();
    let scope = RunScope::new(&state, req.run_id.as_deref());
    let engine = services::connection::engine_of(&state, &req.connection_id)?;
    let path = Path::new(&req.path);
    sql_import::verify_unchanged(path, req.expected_size, req.expected_modified_ms).await?;
    let mut file = SqlFile::open(path, sql_import::dialect_of(engine)).await?;
    let mut report = SqlImportReport { statements: 0, elapsed_ms: 0, error: None, failed_line: None, failed_statement: None };
    let mut last_frame = Instant::now();
    loop {
        if scope.token.is_cancelled() {
            report.error = Some("Cancelled by user".to_string());
            break;
        }
        let statement = match file.next().await {
            Ok(Some(statement)) => statement,
            Ok(None) => break,
            Err(err) => {
                report.error = Some(err.message().to_string());
                break;
            }
        };
        if sql_import::changes_escaping(&statement.text) {
            report.error = Some(sql_import::ESCAPING_PROBLEM.to_string());
            report.failed_line = Some(statement.line);
            report.failed_statement = Some(sql_import::shorten_statement(&statement.text));
            break;
        }
        let sql = statement.text.clone();
        let outcome = run_chunk(&state, &req.connection_id, &statement.text, sql, req.confirm_destructive).await;
        if let Err(err) = outcome {
            report.error = Some(err.message().to_string());
            report.failed_line = Some(statement.line);
            report.failed_statement = Some(sql_import::shorten_statement(&statement.text));
            break;
        }
        report.statements += 1;
        if last_frame.elapsed() >= PROGRESS_EVERY {
            last_frame = Instant::now();
            if let Some(channel) = &on_progress {
                let _ = channel.send(SqlImportProgress { statements: report.statements, bytes_read: file.bytes_read, bytes_total: file.size });
            }
        }
    }
    if let Some(channel) = &on_progress {
        let _ = channel.send(SqlImportProgress { statements: report.statements, bytes_read: file.bytes_read, bytes_total: file.size });
    }
    report.elapsed_ms = millis(started);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affected_rows_are_summed_only_when_every_statement_reports_them() {
        let outcome = |statements| QueryOutcome { statements, total_rows: None, elapsed_ms: 0 };
        assert_eq!(affected(&outcome(vec![StatementResult::Affected { rows_affected: 0 }, StatementResult::Affected { rows_affected: 200 }])), Some(200));
        let rows = StatementResult::Rows { result: crate::model::ResultSet { columns: Vec::new(), rows: Vec::new(), truncated: false } };
        assert_eq!(affected(&outcome(vec![StatementResult::Affected { rows_affected: 1 }, rows])), None);
    }

    #[test]
    fn file_labels_are_the_file_name() {
        assert_eq!(file_label("/data/in/users.csv"), "users.csv");
        assert_eq!(file_label("users.csv"), "users.csv");
    }
}
