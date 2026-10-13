// SOT: query-commands, ipc-execute, ipc-history, ipc-buffers, ipc-split-script, ipc-save-sql-file, ipc-cancel-query, ipc-transactions

use crate::error::AppResult;
use crate::guard;
use crate::model::{BatchEvent, BatchSummary, EditorBuffer, Engine, HistoryEntry, HistoryOrigin, HistoryPage, OpenedSqlFile, QueryOutcome, StatementSpan};
use crate::commands::connections::SessionRequest;
use crate::services;
use crate::services::query::TransactionStep;
use crate::state::AppState;
use serde::Deserialize;
use std::path::Path;
use tauri::ipc::Channel;
use tauri::State;
use ts_rs::TS;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExecuteQueryRequest {
    pub connection_id: String,
    pub sql: String,
    pub confirm_destructive: bool,
    /// None = "No limit" in the editor; otherwise clamped by the block.
    pub max_rows: Option<u32>,
    /// Schema / keyspace the editor's picker is on, applied for this run only.
    pub schema: Option<String>,
    /// Names this run so `cancel_query` can stop it. Omitted = not stoppable.
    #[serde(default)]
    #[ts(optional)]
    pub run_id: Option<String>,
    /// Names the query tab this run belongs to (its tab id). The adapter keeps one
    /// dedicated connection per key, so SET, temp tables and BEGIN persist between
    /// Runs; the tab's close sends `close_session`. Omitted = the shared pool.
    #[serde(default)]
    #[ts(optional)]
    pub session_key: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CancelQueryRequest {
    pub run_id: String,
}

// WHAT:  A script to run statement by statement (`execute_batch`).
// HOW:   Like `ExecuteQueryRequest`, plus `stop_on_error` (carry on after a
//        failing statement, or stop at it) and `split` (cut the script at its
//        statement boundaries, or send the text whole — for command languages
//        that have no `;`). `run_id` is required: it is what Stop names.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExecuteBatchRequest {
    pub connection_id: String,
    pub sql: String,
    pub confirm_destructive: bool,
    /// None or 0 = "No limit"; otherwise clamped by the block.
    pub max_rows: Option<u32>,
    pub schema: Option<String>,
    pub run_id: String,
    pub stop_on_error: bool,
    pub split: bool,
    /// The query tab's id: its statements share one pinned connection.
    #[serde(default)]
    #[ts(optional)]
    pub session_key: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryPageRequest {
    pub connection_id: Option<String>,
    pub origin: Option<HistoryOrigin>,
    /// Some(true) = only what was typed in the editor; None = everything.
    pub from_editor: Option<bool>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReadSqlFileRequest {
    /// Chosen in the OS open dialog.
    pub path: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditorHistoryRequest {
    pub connection_id: String,
    pub limit: u32,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryRequest {
    pub connection_id: Option<String>,
    pub origin: Option<HistoryOrigin>,
    pub limit: u32,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveBufferRequest {
    pub buffer: EditorBuffer,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BufferIdRequest {
    pub id: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SplitScriptRequest {
    pub sql: String,
    /// The connection's engine: a script is cut where that engine's adapter
    /// cuts it (`;` for SQL, a line for Redis, a blank line for Mongo…).
    /// Omitted = SQL.
    #[serde(default)]
    #[ts(optional)]
    pub engine: Option<Engine>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveSqlFileRequest {
    /// Chosen in the OS save dialog; `.sql` is forced on if it is missing.
    pub path: String,
    pub sql: String,
}

#[tauri::command]
pub async fn execute_query(state: State<'_, AppState>, req: ExecuteQueryRequest) -> AppResult<QueryOutcome> {
    let max_rows = guard::clamp_run_rows(req.max_rows);
    let sql = req.sql.clone();
    let schema = req.schema.clone();
    // The log marks it as typed in the editor (step 10).
    guard::as_editor(guard::statement_in_session(
        &state,
        req.session_key.as_deref(),
        guard::StatementRequest {
            connection_id: &req.connection_id,
            sql: &req.sql,
            confirm_destructive: req.confirm_destructive,
            run_id: req.run_id.as_deref(),
        },
        |ctx| async move { services::query::execute(&ctx, &sql, max_rows, schema.as_deref()).await },
    ))
    .await
}

// WHAT:  Runs a script statement by statement and streams each one's progress.
// WHY:   The editor's result tabs show per-statement status and time, fill in as
//        statements finish, and survive a failing statement in the middle.
// HOW:   The service checks the whole script first (read-only, destructive
//        confirmation), then runs each statement through the block on its own.
//        Events travel on `on_event`; the return value is only the summary, so
//        no row is sent twice. `guard::local` here is the command's entry: every
//        statement inside passes `guard::statement_in_session` individually.
// WHERE: src-tauri/src/services/query.rs (execute_batch), src/lib/ipc.ts (executeBatch)
#[tauri::command]
pub async fn execute_batch(state: State<'_, AppState>, req: ExecuteBatchRequest, on_event: Channel<BatchEvent>) -> AppResult<BatchSummary> {
    guard::local("execute_batch", async {
        let batch = services::query::BatchRequest {
            connection_id: &req.connection_id,
            sql: &req.sql,
            confirm_destructive: req.confirm_destructive,
            max_rows: guard::clamp_run_rows(req.max_rows),
            namespace: req.schema.as_deref(),
            run_id: &req.run_id,
            stop_on_error: req.stop_on_error,
            split: req.split,
            session_key: req.session_key.as_deref(),
        };
        let emit = |event: BatchEvent| {
            if let Err(err) = on_event.send(event) {
                log::debug!("batch progress not delivered: {err}");
            }
        };
        guard::as_editor(services::query::execute_batch(&state, &batch, &emit)).await
    })
    .await
}

// WHAT:  One page of the query log with the filtered total (the log viewer's pager).
#[tauri::command]
pub async fn list_history_page(state: State<'_, AppState>, req: HistoryPageRequest) -> AppResult<HistoryPage> {
    guard::local("list_history_page", async {
        services::history::page(&state, req.connection_id.as_deref(), req.origin, req.from_editor, req.limit, req.offset)
    })
    .await
}

// WHAT:  The editor's recent statements, each text once (the History panel).
#[tauri::command]
pub async fn list_editor_history(state: State<'_, AppState>, req: EditorHistoryRequest) -> AppResult<Vec<HistoryEntry>> {
    guard::local("list_editor_history", async { services::history::recent_editor(&state, &req.connection_id, req.limit) }).await
}

// WHAT:  Stops a running `execute_query` by the run id the editor gave it.
// WHY:   The Stop button. Answers whether a run was still there to stop; one
//        that already finished is not an error — Stop and completion race.
// HOW:   Trips the token the block is racing (step 9); the block then asks the
//        adapter to cancel server-side and logs the run as cancelled.
// WHERE: src-tauri/src/state.rs (QueryRuns), src-tauri/src/guard/mod.rs
// WHAT:  The editor's Manual mode: BEGIN / COMMIT / ROLLBACK on the session's
//        pinned connection. Each answers whether a transaction is open after it.
// WHY:   Session lane, not statement lane: these run no user-authored SQL, so
//        there is nothing to classify. A read-only connection may still BEGIN —
//        every write inside it is still stopped by the block's step 5.
// WHERE: src-tauri/src/services/query.rs (`transaction`), integrations/mod.rs
#[tauri::command]
pub async fn begin_transaction(state: State<'_, AppState>, req: SessionRequest) -> AppResult<bool> {
    guard::session(&state, &req.connection_id, |ctx| async move {
        services::query::transaction(&ctx, TransactionStep::Begin).await
    })
    .await
}

#[tauri::command]
pub async fn commit_transaction(state: State<'_, AppState>, req: SessionRequest) -> AppResult<bool> {
    guard::session(&state, &req.connection_id, |ctx| async move {
        services::query::transaction(&ctx, TransactionStep::Commit).await
    })
    .await
}

#[tauri::command]
pub async fn rollback_transaction(state: State<'_, AppState>, req: SessionRequest) -> AppResult<bool> {
    guard::session(&state, &req.connection_id, |ctx| async move {
        services::query::transaction(&ctx, TransactionStep::Rollback).await
    })
    .await
}

#[tauri::command]
pub async fn cancel_query(state: State<'_, AppState>, req: CancelQueryRequest) -> AppResult<bool> {
    guard::local("cancel_query", async { Ok(state.query_runs().cancel(&req.run_id)) }).await
}

#[tauri::command]
pub async fn list_history(state: State<'_, AppState>, req: HistoryRequest) -> AppResult<Vec<HistoryEntry>> {
    guard::local("list_history", async {
        services::history::list(&state, req.connection_id.as_deref(), req.origin, req.limit)
    })
    .await
}

#[tauri::command]
pub async fn list_buffers(state: State<'_, AppState>) -> AppResult<Vec<EditorBuffer>> {
    guard::local("list_buffers", async { services::buffers::list(&state) }).await
}

#[tauri::command]
pub async fn save_buffer(state: State<'_, AppState>, req: SaveBufferRequest) -> AppResult<EditorBuffer> {
    guard::local("save_buffer", async { services::buffers::save(&state, &req.buffer) }).await
}

#[tauri::command]
pub async fn delete_buffer(state: State<'_, AppState>, req: BufferIdRequest) -> AppResult<()> {
    guard::local("delete_buffer", async { services::buffers::delete(&state, &req.id) }).await
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ClearHistoryRequest {
    pub connection_id: Option<String>,
}

#[tauri::command]
pub async fn clear_history(state: State<'_, AppState>, req: ClearHistoryRequest) -> AppResult<u64> {
    guard::local("clear_history", async { services::history::clear(&state, req.connection_id.as_deref()) }).await
}

// WHAT:  Where every statement in the editor's script starts and ends.
// WHY:   PRD §4.3 — the gutter ▶ and Run at cursor send one statement instead of
//        the whole buffer, and they must cut it where the block would.
// HOW:   Pure text work, so it takes the local lane: no session, no database.
// WHERE: src-tauri/src/guard/destructive.rs (spans), src/features/editor/SqlEditor.tsx
#[tauri::command]
pub async fn split_script(req: SplitScriptRequest) -> AppResult<Vec<StatementSpan>> {
    guard::local("split_script", async move {
        Ok(match req.engine {
            Some(engine) => guard::dialect::spans(engine, &req.sql),
            None => guard::destructive::spans(&req.sql),
        })
    })
    .await
}

// WHAT:  Reads the .sql file the user picked, for a new query tab.
#[tauri::command]
pub async fn read_sql_file(req: ReadSqlFileRequest) -> AppResult<OpenedSqlFile> {
    guard::local("read_sql_file", async move { services::scripts::read(Path::new(&req.path)) }).await
}

// WHAT:  Writes the editor's script to the .sql file the user picked.
#[tauri::command]
pub async fn save_sql_file(req: SaveSqlFileRequest) -> AppResult<String> {
    guard::local("save_sql_file", async move { services::scripts::save(Path::new(&req.path), &req.sql) }).await
}
