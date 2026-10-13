// SOT: changes-commands, ipc-pending-changes, commit-changes, rows-as-sql-command

use crate::error::AppResult;
use crate::guard;
use crate::model::{ChangePreview, CommitReport, RowSqlKind, StagedChange, TableRef, Value};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use ts_rs::TS;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangesRequest {
    pub connection_id: String,
    pub changes: Vec<StagedChange>,
    /// The user has seen the destructive statements in this commit and said yes.
    /// Only a confirmation dialog may set it.
    #[serde(default)]
    #[ts(optional)]
    pub confirm_destructive: Option<bool>,
}

// WHAT:  Exact SQL for the Pending Changes panel; nothing runs.
#[tauri::command]
pub async fn preview_changes(state: State<'_, AppState>, req: ChangesRequest) -> AppResult<ChangePreview> {
    guard::local("preview_changes", async {
        let engine = services::connection::engine_of(&state, &req.connection_id)?;
        services::changes::preview(engine, &req.changes)
    })
    .await
}

// WHAT:  Runs the staged changes through the statement guard — read-only locks,
//        classification, the destructive gate and history logging apply exactly as
//        for typed SQL — and reports what each change did.
// WHY:   The commit is one transaction on one pinned connection, rolled back on the
//        first failure; the report says which changes matched no row and, where the
//        engine has RETURNING, hands back the rows as they are now.
// HOW:   `confirm_destructive` is the user's answer to the guard's confirmation
//        request (`DestructiveConfirmationRequired`), never a default. A staged
//        statement that declares itself destructive needs it too.
#[tauri::command]
pub async fn commit_changes(state: State<'_, AppState>, req: ChangesRequest) -> AppResult<CommitReport> {
    let confirmed = req.confirm_destructive.unwrap_or(false);
    let engine = services::connection::engine_of(&state, &req.connection_id)?;
    let plan = services::changes::plan(engine, &req.changes)?;
    let for_run = plan.clone();
    let staged = req.changes.clone();
    let outcome = guard::statement(
        &state,
        guard::StatementRequest { connection_id: &req.connection_id, sql: &plan.script, confirm_destructive: confirmed, run_id: None },
        |ctx| async move {
            services::changes::require_confirmation(&staged, confirmed)?;
            services::changes::commit(&ctx, &for_run).await
        },
    )
    .await?;
    Ok(services::changes::report(&plan, &outcome))
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RowsSqlRequest {
    pub connection_id: String,
    pub table: TableRef,
    pub kind: RowSqlKind,
    /// Names of the values in each row, in order.
    pub columns: Vec<String>,
    /// The columns that address a row (the primary key); empty when there is none.
    pub key_columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

// WHAT:  Rows as SELECT / INSERT / UPDATE / DELETE statements in the connection's
//        dialect. Nothing runs.
#[tauri::command]
pub async fn rows_as_sql(state: State<'_, AppState>, req: RowsSqlRequest) -> AppResult<String> {
    guard::local("rows_as_sql", async {
        let engine = services::connection::engine_of(&state, &req.connection_id)?;
        services::changes::rows_as_sql(engine, &req.table, req.kind, &req.columns, &req.key_columns, &req.rows)
    })
    .await
}
