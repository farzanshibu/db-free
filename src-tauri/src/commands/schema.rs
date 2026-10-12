// SOT: schema-commands, ipc-schema, ipc-ddl-preview, ipc-describe-table, ipc-ddl-profile

use crate::error::AppResult;
use crate::guard;
use crate::model::{ColumnInfo, DdlOperation, DdlPreview, DdlProfile, ForeignKey, SchemaCatalog, TableRef, TableSpec};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use ts_rs::TS;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogRequest {
    pub connection_id: String,
    /// Skip the schema cache and ask the database (the sidebar's Refresh).
    #[serde(default)]
    #[ts(optional)]
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnsRequest {
    pub connection_id: String,
    pub table: TableRef,
    /// Skip the schema cache and ask the database.
    #[serde(default)]
    #[ts(optional)]
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateTemplateRequest {
    pub connection_id: String,
    pub table: TableRef,
    /// Columns the user described in the New-table dialog.
    pub columns: Vec<ColumnInfo>,
}

// WHAT:  The statement that would create this table, in the engine's own
//        language, for the editor to open pre-filled.
// WHY:   Creating is where engines differ most, so the adapter writes it; the
//        user reads it before running, and the guard applies when they do.
#[tauri::command]
pub async fn create_template(state: State<'_, AppState>, req: CreateTemplateRequest) -> AppResult<Option<String>> {
    guard::session(&state, &req.connection_id, |ctx| async move {
        Ok(ctx.integration.create_template(&req.table, &req.columns))
    })
    .await
}

#[tauri::command]
pub async fn load_catalog(state: State<'_, AppState>, req: CatalogRequest) -> AppResult<SchemaCatalog> {
    let app: &AppState = &state;
    guard::session(&state, &req.connection_id, |ctx| async move {
        services::schema::catalog_cached(app, &ctx, req.force.unwrap_or(false)).await
    })
    .await
}

#[tauri::command]
pub async fn load_columns(state: State<'_, AppState>, req: ColumnsRequest) -> AppResult<Vec<ColumnInfo>> {
    let app: &AppState = &state;
    guard::session(&state, &req.connection_id, |ctx| async move {
        services::schema::columns_cached(app, &ctx, &req.table, req.force.unwrap_or(false)).await
    })
    .await
}

#[tauri::command]
pub async fn load_foreign_keys(state: State<'_, AppState>, req: CatalogRequest) -> AppResult<Vec<ForeignKey>> {
    let app: &AppState = &state;
    guard::session(&state, &req.connection_id, |ctx| async move {
        services::schema::foreign_keys_cached(app, &ctx, req.force.unwrap_or(false)).await
    })
    .await
}

#[tauri::command]
pub async fn load_ddl(state: State<'_, AppState>, req: ColumnsRequest) -> AppResult<Option<String>> {
    guard::session(&state, &req.connection_id, |ctx| async move { services::schema::ddl(&ctx, &req.table).await }).await
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PreviewDdlRequest {
    pub connection_id: String,
    pub operation: DdlOperation,
}

// WHAT:  What the schema forms offer for this engine (types, switches, caveats).
#[tauri::command]
pub async fn ddl_profile(state: State<'_, AppState>, req: CatalogRequest) -> AppResult<DdlProfile> {
    guard::session(&state, &req.connection_id, |ctx| async move { Ok(services::schema::ddl_profile(&ctx)) }).await
}

// WHAT:  A live table described as a `TableSpec`, for the Edit and Duplicate forms.
#[tauri::command]
pub async fn describe_table(state: State<'_, AppState>, req: ColumnsRequest) -> AppResult<TableSpec> {
    guard::session(&state, &req.connection_id, |ctx| async move { services::schema::describe_table(&ctx, &req.table).await }).await
}

// WHAT:  The statements a schema form would run, as one script, for preview.
// WHY:   Nothing runs here. The form hands the script to `execute_query`, which
//        applies the read-only lock and the destructive confirmation.
#[tauri::command]
pub async fn preview_ddl(state: State<'_, AppState>, req: PreviewDdlRequest) -> AppResult<DdlPreview> {
    guard::session(&state, &req.connection_id, |ctx| async move { services::schema::preview_ddl(&ctx, &req.operation).await }).await
}
