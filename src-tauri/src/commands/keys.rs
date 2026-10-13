// SOT: key-commands, ipc-scan-keys, ipc-key-info

use crate::error::AppResult;
use crate::guard;
use crate::model::{RedisKeyInfo, RedisKeyPage, RedisKeyScan};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use ts_rs::TS;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScanKeysRequest {
    pub connection_id: String,
    pub scan: RedisKeyScan,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyInfoRequest {
    pub connection_id: String,
    pub key: String,
}

// WHAT:  One step of the sidebar key tree's SCAN (see `RedisKeyScan`).
// WHY:   Session lane: it runs no user-authored statement, so there is nothing to
//        classify, and it works on a read-only connection (a read-only lock must
//        not hide the keys).
#[tauri::command]
pub async fn scan_keys(state: State<'_, AppState>, req: ScanKeysRequest) -> AppResult<RedisKeyPage> {
    let ScanKeysRequest { connection_id, scan } = req;
    guard::session(&state, &connection_id, |ctx| async move { services::keys::scan(&ctx, &scan).await }).await
}

// WHAT:  Type, TTL, memory, encoding, idle time and size of one key.
#[tauri::command]
pub async fn key_info(state: State<'_, AppState>, req: KeyInfoRequest) -> AppResult<RedisKeyInfo> {
    let KeyInfoRequest { connection_id, key } = req;
    guard::session(&state, &connection_id, |ctx| async move { services::keys::info(&ctx, &key).await }).await
}
