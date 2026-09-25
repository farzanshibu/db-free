// SOT: query-commands, ipc-execute, ipc-history, ipc-buffers, ipc-split-script, ipc-save-sql-file, ipc-cancel-query

use crate::error::AppResult;
use crate::guard;
use crate::model::{EditorBuffer, HistoryEntry, HistoryOrigin, QueryOutcome, StatementSpan};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use std::path::Path;
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
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CancelQueryRequest {
    pub run_id: String,
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
    let max_rows = guard::clamp_result_rows(req.max_rows);
    let sql = req.sql.clone();
    let schema = req.schema.clone();
    guard::statement(
        &state,
        guard::StatementRequest {
            connection_id: &req.connection_id,
            sql: &req.sql,
            confirm_destructive: req.confirm_destructive,
            run_id: req.run_id.as_deref(),
        },
        |ctx| async move { services::query::execute(&ctx, &sql, max_rows, schema.as_deref()).await },
    )
    .await
}

// WHAT:  Stops a running `execute_query` by the run id the editor gave it.
// WHY:   The Stop button. Answers whether a run was still there to stop; one
//        that already finished is not an error — Stop and completion race.
// HOW:   Trips the token the block is racing (step 9); the block then asks the
//        adapter to cancel server-side and logs the run as cancelled.
// WHERE: src-tauri/src/state.rs (QueryRuns), src-tauri/src/guard/mod.rs
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
    guard::local("split_script", async move { Ok(guard::destructive::spans(&req.sql)) }).await
}

// WHAT:  Writes the editor's script to the .sql file the user picked.
#[tauri::command]
pub async fn save_sql_file(req: SaveSqlFileRequest) -> AppResult<String> {
    guard::local("save_sql_file", async move { services::scripts::save(Path::new(&req.path), &req.sql) }).await
}
