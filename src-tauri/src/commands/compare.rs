// SOT: compare-commands, ipc-schema-diff

use crate::error::{AppError, AppResult};
use crate::guard;
use crate::model::{CompareDirection, SchemaDiff};
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
