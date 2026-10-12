// SOT: data-commands, ipc-table-page, ipc-fetch-rows, ipc-fk-remaps

use crate::error::AppResult;
use crate::guard;
use crate::model::{FilterMode, FkRemap, PageQuery, ResultSet, TablePage, TableRef, Value};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use ts_rs::TS;

/// Smallest cut `truncate_over` honours; anything shorter would hide ordinary values.
const MIN_TRUNCATE: u32 = 100;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TablePageRequest {
    pub connection_id: String,
    pub table: TableRef,
    pub query: PageQuery,
    /// How the filter rules combine; omitted = every rule must hold.
    #[serde(default)]
    #[ts(optional)]
    pub filter_mode: Option<FilterMode>,
    /// The caller already has the total (paging on from the first page); skip the count.
    #[serde(default)]
    #[ts(optional)]
    pub skip_count: Option<bool>,
    /// Cut text, JSON and binary cells longer than this many characters (the page
    /// lists them in `truncated`). Omitted = whole values, which is what exports
    /// and the Redis key viewer need.
    #[serde(default)]
    #[ts(optional)]
    pub truncate_over: Option<u32>,
}

#[tauri::command]
pub async fn fetch_table_page(state: State<'_, AppState>, req: TablePageRequest) -> AppResult<TablePage> {
    let query = PageQuery { limit: guard::clamp_page_limit(req.query.limit), ..req.query };
    let table = req.table.clone();
    let query_for_log = query.clone();
    let options = services::data::PageOptions {
        filter_mode: req.filter_mode.unwrap_or_default(),
        skip_count: req.skip_count.unwrap_or(false),
        truncate_over: req.truncate_over.map(|n| n.max(MIN_TRUNCATE) as usize),
    };
    let started = std::time::Instant::now();
    let result = guard::session(&state, &req.connection_id, |ctx| async move {
        let sql = services::data::describe_page_sql(&ctx, &table, &query_for_log, options.filter_mode);
        let page = services::data::table_page_with(&ctx, &table, &query, options).await;
        Ok((sql, page))
    })
    .await;
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok((sql, Ok(page))) => {
            services::history::record_system(&state, &req.connection_id, &sql, Ok(page.rows.len() as u64), elapsed);
            Ok(page)
        }
        Ok((sql, Err(err))) => {
            services::history::record_system(&state, &req.connection_id, &sql, Err(err.message()), elapsed);
            Err(err)
        }
        Err(err) => Err(err),
    }
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FetchRowsRequest {
    pub connection_id: String,
    pub table: TableRef,
    /// The columns that address a row (the primary key), in the order `keys` lists them.
    pub key_columns: Vec<String>,
    pub keys: Vec<Vec<Value>>,
}

// WHAT:  Whole rows by primary key: every column, no truncation, the database's
//        current values.
// WHY:   Opening a shortened cell, editing a long value and refreshing only the rows a
//        commit touched all need the real rows without paging for them.
#[tauri::command]
pub async fn fetch_rows(state: State<'_, AppState>, req: FetchRowsRequest) -> AppResult<ResultSet> {
    let started = std::time::Instant::now();
    let FetchRowsRequest { connection_id, table, key_columns, keys } = req;
    let result = guard::session(&state, &connection_id, |ctx| async move {
        let sql = services::data::rows_by_key_sql(&ctx, &table, &key_columns, &keys)?;
        let rows = services::data::rows_by_key(&ctx, &table, &key_columns, &keys).await;
        Ok((sql, rows))
    })
    .await;
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok((sql, Ok(set))) => {
            services::history::record_system(&state, &connection_id, &sql, Ok(set.row_count()), elapsed);
            Ok(set)
        }
        Ok((sql, Err(err))) => {
            services::history::record_system(&state, &connection_id, &sql, Err(err.message()), elapsed);
            Err(err)
        }
        Err(err) => Err(err),
    }
}

#[tauri::command]
pub async fn list_fk_remaps(state: State<'_, AppState>, req: crate::commands::connections::ConnectionIdRequest) -> AppResult<Vec<FkRemap>> {
    guard::local("list_fk_remaps", async { services::fk_remaps::list(&state, &req.id) }).await
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveFkRemapRequest {
    pub remap: FkRemap,
}

#[tauri::command]
pub async fn save_fk_remap(state: State<'_, AppState>, req: SaveFkRemapRequest) -> AppResult<FkRemap> {
    guard::local("save_fk_remap", async { services::fk_remaps::save(&state, &req.remap) }).await
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeleteFkRemapRequest {
    pub id: String,
}

#[tauri::command]
pub async fn delete_fk_remap(state: State<'_, AppState>, req: DeleteFkRemapRequest) -> AppResult<()> {
    guard::local("delete_fk_remap", async { services::fk_remaps::delete(&state, &req.id) }).await
}
