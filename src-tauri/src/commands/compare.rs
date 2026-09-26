// SOT: compare-commands, ipc-schema-diff, ipc-compare-table-data

use crate::error::{AppError, AppResult};
use crate::guard;
use crate::model::{CompareDirection, DataCompare, SchemaDiff, TableRef};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use ts_rs::TS;

// WHAT:  One side of a schema compare: a connection and a schema from its catalog.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SchemaSide {
    pub connection_id: String,
    /// A schema name as the catalog lists it; None takes the first one.
    pub schema: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SchemaDiffRequest {
    pub left: SchemaSide,
    pub right: SchemaSide,
    pub direction: CompareDirection,
}

fn validate_side(side: &SchemaSide) -> AppResult<()> {
    if side.connection_id.trim().is_empty() {
        return Err(AppError::invalid_input("Pick a connection for both sides."));
    }
    Ok(())
}

// WHAT:  Compares a schema on two sessions and returns the diff and the
//        migration script. Nothing is executed.
// WHY:   Both sides must be live, so each is resolved by the block: the outer
//        `guard::session` resolves the left one, the inner the right one. Both
//        get connection lookup, the not-connected check and the timeout; the
//        same connection on both sides is simply resolved twice.
// WHERE: src-tauri/src/services/schema_diff.rs
#[tauri::command]
pub async fn schema_diff(state: State<'_, AppState>, req: SchemaDiffRequest) -> AppResult<SchemaDiff> {
    validate_side(&req.left)?;
    validate_side(&req.right)?;
    let app: &AppState = &state;
    let SchemaDiffRequest { left, right, direction } = req;
    let (left_id, right_id) = (left.connection_id.clone(), right.connection_id.clone());
    guard::session(app, &left_id, |left_ctx| async move {
        guard::session(app, &right_id, |right_ctx| async move {
            services::schema_diff::compare(&left_ctx, &right_ctx, left.schema.as_deref(), right.schema.as_deref(), direction).await
        })
        .await
    })
    .await
}

// WHAT:  One side of a data compare: a connection and a table on it.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableSide {
    pub connection_id: String,
    pub table: TableRef,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompareTableDataRequest {
    pub left: TableSide,
    pub right: TableSide,
    /// Columns rows are matched by; empty = the primary key.
    pub key_columns: Vec<String>,
    /// Rows read per side; None = the service default (100k).
    pub max_rows: Option<u32>,
    pub direction: CompareDirection,
    /// Also write the INSERT / UPDATE / DELETE script for the target.
    pub include_script: bool,
}

// WHAT:  Compares two tables row by row. Nothing is executed; the sync script
//        is returned for the user to open in a query tab.
// WHY:   Same two-session resolution as `schema_diff`; the row cap is the
//        block's bounds step (clamped, never unbounded).
// WHERE: src-tauri/src/services/data_compare.rs
#[tauri::command]
pub async fn compare_table_data(state: State<'_, AppState>, req: CompareTableDataRequest) -> AppResult<DataCompare> {
    for side in [&req.left, &req.right] {
        if side.connection_id.trim().is_empty() || side.table.name.trim().is_empty() {
            return Err(AppError::invalid_input("Pick a connection and a table for both sides."));
        }
    }
    let app: &AppState = &state;
    let options = services::data_compare::CompareOptions {
        key_columns: req.key_columns,
        max_rows: guard::clamp_result_rows(Some(req.max_rows.unwrap_or(services::data_compare::DEFAULT_MAX_ROWS))),
        direction: req.direction,
        include_script: req.include_script,
    };
    let (left, right) = (req.left, req.right);
    let (left_id, right_id) = (left.connection_id.clone(), right.connection_id.clone());
    guard::session(app, &left_id, |left_ctx| async move {
        guard::session(app, &right_id, |right_ctx| async move {
            services::data_compare::compare(&left_ctx, &left.table, &right_ctx, &right.table, &options).await
        })
        .await
    })
    .await
}
